//! `sandman.log`: the sequence, which a view of current state cannot show.
//!
//! Two Tasks that both ran are indistinguishable in a snapshot, and the log is
//! where their order lives. It reads the [`crate::event::Event`] stream and
//! writes one line per Event, so nothing anywhere has to remember to log
//! anything.
//!
//! **The log is the index; the database is the content.** A line names what
//! happened and the id to look it up under — it does not carry a model's whole
//! reply, a Brief, or a recorded request. Those are rows now, and a log that
//! reprinted them would bury the sequence it exists to show. `--verbose`
//! restores the bodies for a session where that is what is wanted.
//!
//! The terminal shows only the conversation, so the two never interleave.
//!
//! Defines: [`Logger`], [`Verbosity`], [`banner`].

use std::path::Path;

use crate::event::Event;

/// How much of an Event's content reaches the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Verbosity {
    /// One line per Event, bodies elided to a length and a count.
    #[default]
    Terse,
    /// Bodies written out whole. For a run being read closely.
    Verbose,
}

/// Writes the trace.
///
/// Takes the log's path rather than writing to the working directory, so several
/// Harnesses in one process — a bench running its cases — never write over each
/// other and nothing has to move `cwd` to keep them apart.
pub struct Logger {
    file: std::sync::Mutex<std::fs::File>,
    verbosity: Verbosity,
}

impl Logger {
    /// Open a log, truncating whatever was there.
    pub fn create(_path: &Path, _verbosity: Verbosity) -> std::io::Result<Self> {
        unimplemented!()
    }

    /// Follow an Event stream until it ends. Spawned once, at startup.
    pub async fn follow(self: std::sync::Arc<Self>, _events: &crate::event::Events) {
        unimplemented!()
    }

    /// Write one Event.
    pub fn write(&self, _event: &Event) {
        unimplemented!()
    }

    /// A line that is not an Event: a startup banner, a warning from the
    /// Harness, a note from the bench driver.
    pub fn note(&self, _category: &str, _text: &str) {
        unimplemented!()
    }
}

/// Shorten a body for a terse line: a length, and enough of the text to
/// recognise it by.
fn elide(_text: &str) -> String {
    unimplemented!()
}

/// The header a run opens with: when, which model, which database.
pub fn banner(_what: &str) -> String {
    unimplemented!()
}
