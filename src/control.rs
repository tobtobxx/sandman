//! The control socket: how another process puts work into a running Sandman.
//!
//! A Channel is a two-way connection to a human. Cron, an RSS script, a mail
//! watcher and a shell one-liner are none of those — they have nothing to say
//! back to and nothing to be told. They get this instead: one line of JSON in,
//! one line out, and the connection closes.
//!
//! It is a **socket rather than a second writer to the database**, and that is
//! the whole design decision here. A process inserting a row directly would
//! bypass the Store, so no [`crate::event::Event`] would be emitted for it — the
//! log, the Watcher and anything replaying the stream would all have the same
//! blind spot. One writer is the property the Store was shaped around, and this
//! keeps it.
//!
//! A Unix domain socket with restrictive permissions, never a TCP port: this is a
//! write path into a running swarm.
//!
//! Defines: [`Request`], [`Response`], [`serve`], [`send`], [`socket_path`].

use std::path::PathBuf;

use crate::domain::{Spend, TaskId, TaskSummary};

/// What another process may ask for.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Request {
    /// Put a Task on the queue. Recorded as
    /// [`crate::domain::Creator::Control`], so where it came from is not lost.
    CreateTask {
        role: String,
        title: String,
        brief: String,
        /// Seconds to wait before it may run.
        run_at_seconds: Option<i64>,
        /// Seconds between occurrences.
        repeat_seconds: Option<i64>,
        priority: Option<String>,
    },
    ListTasks {
        state: Option<String>,
        count: Option<usize>,
    },
    CancelTask {
        id: String,
    },
    /// What the running Sandman has spent.
    Spend,
}

/// What it gets back.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Response {
    Created { id: String },
    Tasks { tasks: Vec<TaskLine> },
    Cancelled { ids: Vec<String>, running: bool },
    Spent { calls: u32, tokens: u64, cost: String },
    /// Everything that went wrong, said in one sentence.
    Error { message: String },
}

/// A Task as the socket reports it: flat, so a shell script can read it without
/// knowing the domain.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskLine {
    pub id: String,
    pub title: String,
    pub role: String,
    pub state: String,
    pub not_before: Option<i64>,
    pub created_at: i64,
}

/// Where the socket lives: `$SANDMAN_SOCKET`, else `$XDG_RUNTIME_DIR/sandman.sock`,
/// else a path beside the database.
pub fn socket_path() -> PathBuf {
    unimplemented!()
}

/// Listen, and answer requests until the Harness stops.
///
/// Removes a stale socket file left by a killed process, and creates the new one
/// readable and writable only by its owner.
pub async fn serve(
    _harness: std::sync::Arc<crate::harness::Harness>,
    _path: &std::path::Path,
) -> std::io::Result<()> {
    unimplemented!()
}

/// Send one request from the client side and read the answer.
///
/// What `sandman task` does. A missing socket means no Sandman is running, and
/// that is the error the caller gets.
pub async fn send(_path: &std::path::Path, _request: &Request) -> std::io::Result<Response> {
    unimplemented!()
}

/// Turn one request into what the Harness does about it.
async fn handle(
    _harness: &std::sync::Arc<crate::harness::Harness>,
    _request: Request,
) -> Response {
    unimplemented!()
}

impl From<Spend> for Response {
    fn from(_s: Spend) -> Response {
        unimplemented!()
    }
}

impl From<TaskSummary> for TaskLine {
    fn from(_t: TaskSummary) -> TaskLine {
        unimplemented!()
    }
}

impl From<TaskId> for Response {
    fn from(_id: TaskId) -> Response {
        unimplemented!()
    }
}
