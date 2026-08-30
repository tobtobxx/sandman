//! The SQLite schema, and the migrations that reach it.
//!
//! Two rules shape the tables.
//!
//! **A transcript is a query, not a blob.** Messages, mail, utterances and
//! reflections each get a row per item, keyed `(owner, idx)`. Appending is one
//! insert; reading a whole conversation is one ordered scan. Rewriting a
//! Session's entire history on every message — which is what a JSON column would
//! mean — would make the cost of a long-running Comms Session quadratic.
//!
//! **Sum types are stored as a discriminant plus JSON.** `tasks.state` holds
//! `'pending' | 'running' | 'completed' | 'cancelled'` as its own column so the
//! queue scan stays an index lookup, and `tasks.state_json` holds the payload
//! that variant carries. The same pattern covers `CallStatus`, `Schedule`,
//! `ReflectionResult` and `LessonSubject`. Nothing that a query filters on hides
//! inside JSON.
//!
//! Migrations are ordered statements applied under `meta.schema_version`.
//! Opening a database written by a newer binary is a clean error rather than a
//! partial read.
//!
//! Defines: [`MIGRATIONS`], [`SCHEMA_VERSION`], [`apply`].

use rusqlite::Connection;

/// The schema version this binary writes and expects.
pub const SCHEMA_VERSION: u32 = 1;

/// Every migration, oldest first. Index + 1 is the version it produces.
///
/// A migration is never edited once released; a change is a new entry.
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

/// Bring a connection up to [`SCHEMA_VERSION`], or say why it cannot be.
///
/// Applying from empty and applying twice both end in the same place. A database
/// at a version this binary does not know is refused, not read.
pub fn apply(_conn: &Connection) -> Result<(), super::DbError> {
	unimplemented!()
}

/// The version a database is currently at. Zero for an empty one.
pub fn version_of(_conn: &Connection) -> Result<u32, super::DbError> {
	unimplemented!()
}
