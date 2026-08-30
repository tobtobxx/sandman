//! Sandman's entry point. Three ways in.
//!
//! ```text
//! sandman
//!     Interactive. Two Channels at once — the terminal, and a browser on
//!     :8080 that also watches everything the Harness owns, live. A control
//!     socket is opened so another process can put work in.
//!
//! sandman run --role planning --title "..." --brief "..."
//!            [--at 600] [--every 86400] [--priority high|normal|low]
//!     One Task, run until nothing is left, then print the Results and what it
//!     cost. Its own Harness; no socket, no browser.
//!
//! sandman task --role planning --title "..." --brief "..."
//!             [--at 600] [--every 86400] [--priority high|normal|low]
//!     Put a Task into a Sandman that is already running, through its control
//!     socket, and print the id. This is how anything that is not a Channel —
//!     cron, an RSS script, a mail watcher — gets work in.
//! ```
//!
//! Common flags: `--db <path>` (default `sandman.sqlite`), `--log <path>`
//! (default `sandman.log`), `--socket <path>`, `--verbose`.
//!
//! Wiring lives here and only here: which [`sandman::model::Model`], which
//! [`sandman::tools::ToolRunner`], which [`sandman::domain::Clock`]. Everything
//! below takes what it needs and builds nothing itself, which is what lets the
//! bench assemble the same Harness with different pieces.

/// Which way in this invocation is.
enum Command {
    Interactive,
    /// A one-shot Task in its own Harness.
    Run(TaskArgs),
    /// A Task into a Sandman already running.
    Task(TaskArgs),
}

/// What both Task-creating commands take.
struct TaskArgs {
    role: String,
    title: Option<String>,
    brief: String,
    at_seconds: Option<i64>,
    every_seconds: Option<i64>,
    priority: Option<String>,
}

/// Where the state, the trace and the socket live.
struct Paths {
    db: std::path::PathBuf,
    log: std::path::PathBuf,
    socket: std::path::PathBuf,
}

fn parse(_argv: &[String]) -> Result<(Command, Paths, sandman::log::Verbosity), String> {
    unimplemented!()
}

/// Build a whole Sandman: database, Event stream, logger, model, tools,
/// scheduler, Harness.
async fn assemble(
    _paths: &Paths,
    _verbosity: sandman::log::Verbosity,
) -> Result<std::sync::Arc<sandman::harness::Harness>, String> {
    unimplemented!()
}

/// Two Channels, a Watcher, and a control socket, running until the human
/// leaves.
async fn interactive(_paths: Paths, _verbosity: sandman::log::Verbosity) -> Result<(), String> {
    unimplemented!()
}

/// One Task in its own Harness, until nothing is left. Prints every Task's
/// Result and what the run spent.
async fn one_shot(
    _args: TaskArgs,
    _paths: Paths,
    _verbosity: sandman::log::Verbosity,
) -> Result<(), String> {
    unimplemented!()
}

/// One Task into a running Sandman, over the control socket.
async fn into_running(_args: TaskArgs, _paths: Paths) -> Result<(), String> {
    unimplemented!()
}

#[tokio::main]
async fn main() {
    unimplemented!()
}
