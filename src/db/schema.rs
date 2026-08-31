//! SQLite schema and migrations — the only place that names tables or columns.
//!
//! No type to construct: [`SCHEMA_VERSION`] and [`MIGRATIONS`] are constants, [`apply`] mutates a `Connection` in place.
//! Use via `db::open`, which calls [`apply`] after pragmas; [`version_of`] reads `meta.schema_version` without writing.
//! Only `db::open` calls it — `Store` never writes SQL, `rows.rs` never sees DDL, and nothing else imports this module.
//!
//! ```text
//! db::open → schema::apply(conn) → version_of(conn) → execute_batch per migration → update meta.schema_version
//!                                → refuse if found > SCHEMA_VERSION
//! ```
//!
//! Rules:
//! - **Transcript is a query, not a blob.** `(owner, idx)` rows; append is one insert, read is one ordered scan.
//! - **Sum types are discriminant plus JSON.** Filter column stays indexable, payload stays opaque.
//! - **Migrations are forward-only and idempotent.** Apply from empty or twice ends at `SCHEMA_VERSION`; newer database is refused.
//!
//! Defines: [`MIGRATIONS`], [`SCHEMA_VERSION`], [`apply`], [`version_of`].

use rusqlite::{Connection, OptionalExtension};

/// The schema version this binary writes and expects.
pub const SCHEMA_VERSION: u32 = 1;

/// Every migration, oldest first. Index `i` produces version `i + 1`.
/// `v1` may be rewritten while prototype — ask first.
pub const MIGRATIONS: &[&str] = &[
	// v1 — the initial schema.
	r#"
    CREATE TABLE runs (
        id          INTEGER PRIMARY KEY,
        started_at  INTEGER NOT NULL,
        ended_at    INTEGER,
        model       TEXT    NOT NULL
    );

    CREATE TABLE tasks (
        id            INTEGER PRIMARY KEY,
        run           INTEGER NOT NULL REFERENCES runs(id),
        title         TEXT    NOT NULL,
        brief         TEXT    NOT NULL,
        role          TEXT    NOT NULL,
        state         TEXT    NOT NULL,
        state_json    TEXT    NOT NULL,
        schedule      TEXT    NOT NULL,
        schedule_json TEXT    NOT NULL,
        not_before    INTEGER,
        subscriber    INTEGER,
        priority      TEXT    NOT NULL,
        created_by    TEXT    NOT NULL,
        created_at    INTEGER NOT NULL
    );
    CREATE INDEX tasks_pending ON tasks(state, not_before);
    CREATE INDEX tasks_run ON tasks(run);

    CREATE TABLE sessions (
        id         INTEGER PRIMARY KEY,
        run        INTEGER NOT NULL REFERENCES runs(id),
        kind       TEXT    NOT NULL,
        task       INTEGER REFERENCES tasks(id),
        role       TEXT,
        channel    INTEGER,
        status     TEXT    NOT NULL,
        status_json TEXT   NOT NULL,
        started_at INTEGER NOT NULL,
        ended_at   INTEGER
    );
    CREATE INDEX sessions_task ON sessions(task);

    CREATE TABLE messages (
        session   INTEGER NOT NULL REFERENCES sessions(id),
        idx       INTEGER NOT NULL,
        role      TEXT    NOT NULL,
        body_json TEXT    NOT NULL,
        PRIMARY KEY (session, idx)
    );

    CREATE TABLE mail (
        session  INTEGER NOT NULL REFERENCES sessions(id),
        idx      INTEGER NOT NULL,
        from_who TEXT    NOT NULL,
        text     TEXT    NOT NULL,
        at       INTEGER NOT NULL,
        read     INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (session, idx)
    );

    CREATE TABLE reflections (
        session       INTEGER NOT NULL REFERENCES sessions(id),
        idx           INTEGER NOT NULL,
        kind          TEXT    NOT NULL,
        call          INTEGER NOT NULL REFERENCES calls(id),
        after_message INTEGER NOT NULL,
        result        TEXT    NOT NULL,
        result_json   TEXT    NOT NULL,
        at            INTEGER NOT NULL,
        PRIMARY KEY (session, idx)
    );

    CREATE TABLE calls (
        id           INTEGER PRIMARY KEY,
        run          INTEGER NOT NULL REFERENCES runs(id),
        session      INTEGER NOT NULL REFERENCES sessions(id),
        tier         INTEGER NOT NULL,
        model        TEXT    NOT NULL,
        request_json TEXT    NOT NULL,
        status       TEXT    NOT NULL,
        status_json  TEXT    NOT NULL,
        tokens       INTEGER,
        cost         INTEGER,
        queued_at    INTEGER NOT NULL
    );
    CREATE INDEX calls_run ON calls(run, status);
    CREATE INDEX calls_session ON calls(session);

    CREATE TABLE channels (
        id      INTEGER PRIMARY KEY,
        kind    TEXT    NOT NULL,
        session INTEGER NOT NULL REFERENCES sessions(id)
    );

    CREATE TABLE utterances (
        channel INTEGER NOT NULL REFERENCES channels(id),
        idx     INTEGER NOT NULL,
        who     TEXT    NOT NULL,
        text    TEXT    NOT NULL,
        at      INTEGER NOT NULL,
        PRIMARY KEY (channel, idx)
    );

    CREATE TABLE lessons (
        id         INTEGER PRIMARY KEY,
        run        INTEGER NOT NULL REFERENCES runs(id),
        text       TEXT    NOT NULL,
        day        TEXT    NOT NULL,
        session    INTEGER NOT NULL REFERENCES sessions(id),
        about      TEXT    NOT NULL,
        about_json TEXT    NOT NULL,
        at         INTEGER NOT NULL
    );

    -- Embeddings for search by meaning. Kept out of the entity tables: a vector
    -- is several hundred floats no human reads, and nothing that walks an
    -- entity should carry it. Never stale, because nothing in the corpus is
    -- edited after it is written.
    CREATE TABLE vectors (
        key    TEXT PRIMARY KEY,   -- 'lesson/l-01', 'task/t-03'
        model  TEXT NOT NULL,
        vector BLOB NOT NULL
    );

    CREATE TABLE counters (
        name TEXT PRIMARY KEY,
        next INTEGER NOT NULL
    );

    CREATE TABLE meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    "#,
];

/// Bring `conn` to [`SCHEMA_VERSION`].
/// Returns `Some((from, to))` if migrations ran, `None` if already current. Errors if the database is newer.
pub fn apply(conn: &Connection) -> Result<Option<(u32, u32)>, super::DbError> {
	// Check current version
	let found = version_of(conn)?;
	// Refuse newer database
	if found > SCHEMA_VERSION {
		return Err(super::DbError::SchemaVersion {
			found,
			expected: SCHEMA_VERSION,
		});
	}
	// Skip if current
	if found == SCHEMA_VERSION {
		return Ok(None);
	}

	// Apply migrations
	for migration in &MIGRATIONS[found as usize..] {
		conn.execute_batch(migration)?;
	}
	// Record new version
	conn.execute(
		"INSERT INTO meta (key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
		[SCHEMA_VERSION],
	)?;
	Ok(Some((found, SCHEMA_VERSION)))
}

/// Read `meta.schema_version` from `conn`.
/// Returns `0` for an empty database. Errors if the stored value is not a number.
pub fn version_of(conn: &Connection) -> Result<u32, super::DbError> {
	// Check for meta table
	let has_meta: bool = conn.query_row(
		"SELECT EXISTS (
             SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta'
         )",
		[],
		|row| row.get(0),
	)?;
	if !has_meta {
		return Ok(0);
	}

	// Read stored version
	let version: Option<String> = conn
		.query_row(
			"SELECT value FROM meta WHERE key = 'schema_version'",
			[],
			|row| row.get(0),
		)
		.optional()?;
	match version {
		Some(v) => v.parse().map_err(|_| {
			super::DbError::Corrupt(format!(
				"meta.schema_version is `{v}`, not a number"
			))
		}),
		None => Ok(0),
	}
}
