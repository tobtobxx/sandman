//! SQLite: where all of Sandman's state actually lives.
//!
//! The database is the single source of truth. There is no in-memory mirror of
//! it, so there is nothing for the two to disagree about. Every read is a query
//! and every write is a transaction. That is affordable because the swarm
//! already serialises on one model call at a time — a query is microseconds
//! against a network round trip.
//!
//! Nothing outside `store.rs` opens a connection or writes a statement. This
//! module is the mapping and the schema; the Store is the vocabulary.
//!
//! Ids are minted here, from the [`counters`] table, inside the same transaction
//! as the insert that uses them. They survive a restart, and a fresh database
//! counts from one — which is what lets two Harnesses live in one process
//! without sharing an id space.
//!
//! Modules: [`schema`] — tables and migrations; [`rows`] — rows to domain values.
//!
//! Defines: [`Backing`], [`DbError`], [`open`], [`counters`].

pub mod rows;
pub mod schema;

use rusqlite::Connection;

/// Where a database lives.
#[derive(Debug, Clone)]
pub enum Backing {
	/// A file on disk. What a real Sandman run uses.
	File(std::path::PathBuf),
	/// Private to this process and gone when it closes. What each bench case
	/// uses, which is why a case needs no process of its own.
	Memory,
}

/// Anything that can go wrong between the domain and the database.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
	#[error("sqlite: {0}")]
	Sqlite(#[from] rusqlite::Error),
	#[error("could not read a stored value: {0}")]
	Json(#[from] serde_json::Error),
	#[error("this database is at schema version {found}; this build writes {expected}")]
	SchemaVersion { found: u32, expected: u32 },
	#[error("a stored `{what}` had the unknown variant `{tag}`")]
	UnknownVariant { what: &'static str, tag: String },
	#[error("{0}")]
	Corrupt(String),
}

/// Open a database and bring it up to the current schema.
///
/// WAL, foreign keys on, and a busy timeout — a Watcher and a `VACUUM INTO` may
/// read while the swarm writes.
///
/// The second element is `(from, to)` if [`schema::apply`] actually migrated
/// the database — worth a log line once a Logger exists (see TASKS.md, step 8)
/// — or `None` if it was already current.
pub fn open(
	backing: Backing,
) -> Result<(Connection, Option<(u32, u32)>), DbError> {
	let is_file = matches!(backing, Backing::File(_));
	let conn = match backing {
		Backing::File(path) => Connection::open(path)?,
		Backing::Memory => Connection::open_in_memory()?,
	};
	conn.pragma_update(None, "foreign_keys", "ON")?;
	conn.busy_timeout(std::time::Duration::from_secs(5))?;
	// An in-memory database is private to this connection already, and SQLite
	// does not support WAL for one.
	if is_file {
		conn.pragma_update(None, "journal_mode", "WAL")?;
	}
	let migration = schema::apply(&conn)?;
	Ok((conn, migration))
}

/// Copy the whole database to a file, consistently, while it is in use.
///
/// This is how a bench case keeps its artifact: one `store.sqlite` holding every
/// Task, Session, transcript and model call of the run, queryable afterwards
/// with `sqlite3`.
pub fn save_copy(
	conn: &Connection,
	to: &std::path::Path,
) -> Result<(), DbError> {
	let to = to.to_str().ok_or_else(|| {
		DbError::Corrupt(format!("{} is not valid UTF-8", to.display()))
	})?;
	conn.execute("VACUUM INTO ?1", [to])?;
	Ok(())
}

/// Id minting.
///
/// One row per entity prefix. `next` is read and bumped inside the caller's
/// transaction, so an id is never handed out twice and never handed out for an
/// insert that then rolls back.
pub mod counters {
	use super::DbError;
	use rusqlite::Transaction;

	/// Take the next number for a prefix, bumping the counter in the same
	/// transaction.
	pub fn take(tx: &Transaction<'_>, prefix: &str) -> Result<u32, DbError> {
		let taken: u32 = tx.query_row(
			"INSERT INTO counters (name, next) VALUES (?1, 2)
             ON CONFLICT(name) DO UPDATE SET next = next + 1
             RETURNING next - 1",
			[prefix],
			|row| row.get(0),
		)?;
		Ok(taken)
	}
}
