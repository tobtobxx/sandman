//! The one ordered trace of everything that happens.
//!
//! Every change the Store makes emits an [`Event`], and every consumer that
//! needs to know what happened reads that one stream:
//!
//! - `log.rs` writes one line per Event into `sandman.log` — the sequence, which
//!   a view of current state cannot show.
//! - `web/` turns each Event into a patch for a Watcher's browser.
//! - the bench Rig waits on it, and evaluates tripwires against it, so a case
//!   wakes exactly when something changed instead of polling.
//!
//! The Store's fields are private and mutation only happens through its methods,
//! so a change without an Event is not something to remember: it cannot be
//! written. That is the whole reason state and trace are one mechanism here
//! rather than two.
//!
//! Tool calls are not state changes, so `tools/` holds its own handle on the
//! stream and emits [`Event::ToolCalled`] and [`Event::ToolReturned`] itself.
//! State and trace stay separately testable.
//!
//! Defines: [`Event`], [`Events`].

use crate::domain::{
    CallId, CallStatus, ChannelId, Incoming, Lesson, LlmCall, Message, Reflection, Run, Session,
    SessionId, SessionStatus, Task, TaskId, TaskState, Utterance,
};
use crate::roles::ToolName;
use tokio::sync::broadcast;

/// One thing that happened, in order.
///
/// Events carry whole entities where a consumer would otherwise have to look one
/// up, and ids where it would not — a Watcher merges a whole Task without
/// knowing its shape, but does not need the whole Session to learn its status
/// changed.
#[derive(Debug, Clone)]
pub enum Event {
    RunStarted(Run),
    RunEnded(Run),

    TaskCreated(Task),
    TaskStateChanged { task: TaskId, to: TaskState },

    SessionStarted(Session),
    SessionStatusChanged { session: SessionId, to: SessionStatus },
    MessageAppended { session: SessionId, index: usize, message: Message },
    ReflectionRecorded { session: SessionId, reflection: Reflection },
    MailReceived { session: SessionId, incoming: Incoming },

    CallQueued(LlmCall),
    CallStatusChanged { call: CallId, to: CallStatus },

    ChannelOpened { channel: ChannelId, session: SessionId },
    Said { channel: ChannelId, utterance: Utterance },

    LessonKept(Lesson),

    ToolCalled { session: SessionId, name: ToolName, args: serde_json::Value },
    ToolReturned { session: SessionId, name: ToolName, output: String },
}

/// The bus every Event goes onto.
///
/// A broadcast channel, so consumers are independent: a Watcher that
/// disconnects, or a bench that stops listening, slows nothing down. A consumer
/// that falls far enough behind loses events rather than blocking the swarm,
/// which is the right trade for a trace — the database still holds the state.
#[derive(Debug)]
pub struct Events {
    tx: broadcast::Sender<Event>,
}

impl Events {
    /// A new bus with room for `capacity` events per consumer before the slowest
    /// one starts losing them.
    pub fn new(_capacity: usize) -> Self {
        unimplemented!()
    }

    /// Put one Event on the bus. Never fails and never blocks: an Event with no
    /// listeners is simply dropped.
    pub fn emit(&self, _event: Event) {
        unimplemented!()
    }

    /// Listen from now on. Events emitted before this returns are not replayed —
    /// a consumer that needs the state so far asks the Store for a snapshot
    /// first.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        unimplemented!()
    }
}

impl Event {
    /// The category this Event is logged under: `task`, `session`, `llm`,
    /// `tool`, `meta`, `comms`, `run`.
    pub fn category(&self) -> &'static str {
        unimplemented!()
    }
}
