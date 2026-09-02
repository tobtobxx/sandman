//! SQLite backing, schema, and row mapping beneath the Store.
//!
//! What it is: the only place that knows SQLite — files, pragmas, migrations,
//! row encoding, id minting, and the pid lockfile. [`store.rs`](crate::store)
//! owns the vocabulary; this module owns the mapping.
//!
//! Construct: `Backing::File(path)` or `Memory` → [`open`] returns
//! `(Connection, Option<(from, to)>)` with `schema::apply` applied. File
//! backing needs `Lock::take` first; `Memory` is private per connection.
//!
//! Use: `schema` migrates, `rows` encodes/decodes, `counters::take(tx, prefix)`
//! mints ids inside the caller's transaction, `save_copy` snapshots via
//! `VACUUM INTO`.
//!
//! Consumers: `store.rs` exclusively — nothing else opens a connection or
//! writes SQL. Bench cases use `Memory` + `save_copy`; real runs use
//! `File` + `Lock`.
//!
//! Rules: **only `store.rs` writes** — connection is private, every mutation
//! emits an Event there. **ids are transactional** — `counters.next` bumps in
//! the same tx as the insert. **stale locks are cleared** — pid checked under
//! `/proc`, `clear` + `create_new` closes the race. **newer schema is refused**.
//!
//! | Backing | Pragmas | Lock | Consumer |
//! | --- | --- | --- | --- |
//! | `File(path)` | WAL + FK + busy timeout | `Lock::take` required | real Run |
//! | `Memory` | FK + busy timeout, no WAL | none (private) | bench case |
//!
//! Defines: [`Backing`], [`DbError`], [`Lock`], [`open`], [`save_copy`], [`counters`].
//! Submodules: [`schema`] — tables and migrations; [`rows`] — rows to domain values.

pub mod rows;
pub mod schema;

use rusqlite::Connection;

/// Where a database lives.
#[derive(Debug, Clone)]
pub enum Backing {
	/// A file on disk. What a real Sandman run uses.
	File(std::path::PathBuf),
	/// Private to this process and gone when it closes. What each bench case uses.
	Memory,
}

/// Anything that can go wrong between the domain and the database.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
	#[error("sqlite: {0}")]
	Sqlite(#[from] rusqlite::Error),
	#[error("could not read a stored value: {0}")]
	Json(#[from] serde_json::Error),
	#[error(
		"this database is at schema version {found}; this build writes {expected}"
	)]
	SchemaVersion { found: u32, expected: u32 },
	#[error("a stored `{what}` had the unknown variant `{tag}`")]
	UnknownVariant { what: &'static str, tag: String },
	#[error("{0}")]
	Corrupt(String),
	/// Another Sandman has this database open, and its Run is live. See
	/// [`Lock`].
	#[error(
		"{path} is open in process {pid}; \
		 only one Sandman may use a database at a time"
	)]
	Locked { path: String, pid: u32 },
	/// The lockfile itself could not be written — a read-only directory, most
	/// likely.
	#[error("could not take the lock at {path}: {source}")]
	Lock {
		path: String,
		#[source]
		source: std::io::Error,
	},
}

/// Exclusive file lock for one Sandman per database file.
///
/// Held as this value; `Drop` removes the file. Stale pids are cleared via
/// `/proc` so a killed Run never blocks the next start.
pub struct Lock {
	path: std::path::PathBuf,
}

impl Lock {
	/// Take the file lock or report the live holder.
	///
	/// Checks for a live pid, clears a stale file, and creates the new
	/// lockfile atomically with `create_new`.
	pub fn take(db: &std::path::Path) -> Result<Lock, DbError> {
		// Check holder
		let path = lock_path(db);
		if let Some(pid) = holder(&path) {
			return Err(DbError::Locked {
				path: db.display().to_string(),
				pid,
			});
		}
		// Clear stale file
		let _ = std::fs::remove_file(&path);
		// Create lockfile
		let mut file = std::fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(&path)
			.map_err(|source| DbError::Lock {
				path: path.display().to_string(),
				source,
			})?;
		// Write pid
		std::io::Write::write_all(
			&mut file,
			std::process::id().to_string().as_bytes(),
		)
		.map_err(|source| DbError::Lock {
			path: path.display().to_string(),
			source,
		})?;
		Ok(Lock { path })
	}

	/// Clear any lockfile at `db`, ignoring missing.
	///
	/// What `--break-lock` does for a recycled pid that looks live.
	pub fn clear(db: &std::path::Path) -> std::io::Result<()> {
		match std::fs::remove_file(lock_path(db)) {
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
			other => other,
		}
	}
}

impl Drop for Lock {
	/// Best-effort cleanup.
	fn drop(&mut self) {
		let _ = std::fs::remove_file(&self.path);
	}
}

/// Resolve lockfile path beside the database.
///
/// Appends `.lock` so `sandman.sqlite` and `sandman.db` never share one.
fn lock_path(db: &std::path::Path) -> std::path::PathBuf {
	let mut path = db.as_os_str().to_os_string();
	path.push(".lock");
	std::path::PathBuf::from(path)
}

/// Live holder pid, if any.
///
/// Returns `None` when no lockfile, not a pid, or pid gone from `/proc`.
fn holder(lock: &std::path::Path) -> Option<u32> {
	let pid: u32 = std::fs::read_to_string(lock).ok()?.trim().parse().ok()?;
	std::path::Path::new("/proc")
		.join(pid.to_string())
		.exists()
		.then_some(pid)
}

/// Open a database and migrate to the current schema.
///
/// Sets FK, busy timeout, and WAL for files. Returns `(from, to)` when a
/// migration ran, or `None` if already current.
pub fn open(
	backing: Backing,
) -> Result<(Connection, Option<(u32, u32)>), DbError> {
	// Open connection
	let is_file = matches!(backing, Backing::File(_));
	let conn = match backing {
		Backing::File(path) => Connection::open(path)?,
		Backing::Memory => Connection::open_in_memory()?,
	};
	// Configure pragmas
	conn.pragma_update(None, "foreign_keys", "ON")?;
	conn.busy_timeout(std::time::Duration::from_secs(5))?;
	if is_file {
		conn.pragma_update(None, "journal_mode", "WAL")?;
	}
	// Apply migrations
	let migration = schema::apply(&conn)?;
	Ok((conn, migration))
}

/// Snapshot the whole database to a file while in use.
///
/// Uses `VACUUM INTO`; how bench cases keep `store.sqlite`.
pub fn save_copy(
	conn: &Connection,
	to: &std::path::Path,
) -> Result<(), DbError> {
	// Validate path
	let to = to.to_str().ok_or_else(|| {
		DbError::Corrupt(format!("{} is not valid UTF-8", to.display()))
	})?;
	// Vacuum into file
	conn.execute("VACUUM INTO ?1", [to])?;
	Ok(())
}

/// Transactional id minting from the `counters` table.
pub mod counters {
	use super::DbError;
	use rusqlite::Transaction;

	/// Mint the next id for `prefix` inside `tx`.
	///
	/// Bumps `next` atomically; rolled-back inserts never leak ids.
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
