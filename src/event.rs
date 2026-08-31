//! The one ordered trace. Every state change and every tool call goes onto
//! [`Events`] as an [`Event`]; observers read that single broadcast instead of
//! polling the Store.
//!
//! Construct with [`Events::new`] (capacity per consumer). Use with
//! [`Events::emit`] (never blocks, never fails) and [`Events::subscribe`]
//! (from now on; past state needs a `Store::snapshot`).
//!
//! Consumers and what they read it for:
//!
//! | Consumer | Reads | What for |
//! | --- | --- | --- |
//! | `log.rs` | every `Event` | one line per event in `sandman.log` — order the DB cannot show |
//! | `web/wire.rs` | `Task`/`Session`/`Call`/`Channel`/`Lesson` + `MessageAppended` | whole-entity `Patch`/`Appended` frames to Watchers |
//! | `bench/rig.rs` | every `Event` | wake tripwires without polling |
//!
//! Two emitters, one bus. `Store` emits state (`Run`/`Task`/`Session`/`Call`/
//! `Channel`/`Lesson`/`Message`/`Mail`); `tools::Registry` emits `ToolCalled`/
//! `ToolReturned` on its own handle because tool calls are not state changes.
//! A slow consumer loses events and never slows the swarm; the database remains
//! the durable state.
//!
//! Rules: **one Event per state change — no mutation without an emit.**
//! **broadcast is lossy, not blocking.** **state and tools stay separately
//! testable.**
//!
//! Defines: [`Event`], [`Events`].

use crate::domain::{
	CallId, CallStatus, ChannelId, Incoming, Lesson, LlmCall, Message,
	Reflection, Run, Session, SessionId, SessionStatus, Task, TaskId,
	TaskState, Utterance,
};
use crate::roles::ToolName;
use tokio::sync::broadcast;

/// One ordered change in the trace.
///
/// Carries whole entities where a consumer would otherwise re-read and
/// ids where it would not.
#[derive(Debug, Clone)]
pub enum Event {
	RunStarted(Run),
	RunEnded(Run),

	TaskCreated(Task),
	TaskStateChanged {
		task: TaskId,
		to: TaskState,
	},

	SessionStarted(Session),
	SessionStatusChanged {
		session: SessionId,
		to: SessionStatus,
	},
	MessageAppended {
		session: SessionId,
		index: usize,
		message: Message,
	},
	ReflectionRecorded {
		session: SessionId,
		reflection: Reflection,
	},
	MailReceived {
		session: SessionId,
		incoming: Incoming,
	},

	CallQueued(LlmCall),
	CallStatusChanged {
		call: CallId,
		to: CallStatus,
	},

	ChannelOpened {
		channel: ChannelId,
		session: SessionId,
	},
	Said {
		channel: ChannelId,
		utterance: Utterance,
	},

	LessonKept(Lesson),

	ToolCalled {
		session: SessionId,
		name: ToolName,
		args: serde_json::Value,
	},
	ToolReturned {
		session: SessionId,
		name: ToolName,
		output: String,
	},
}

/// Broadcast bus for every [`Event`].
///
/// Independent per-consumer queues; a lagging consumer drops rather than
/// blocks.
#[derive(Debug)]
pub struct Events {
	tx: broadcast::Sender<Event>,
}

impl Events {
	/// Create a bus with room for `capacity` events per consumer.
	///
	/// A consumer that falls `capacity` behind starts losing events.
	pub fn new(capacity: usize) -> Self {
		let (tx, _rx) = broadcast::channel(capacity);
		Events { tx }
	}

	/// Emit one event onto the bus.
	///
	/// Never blocks or fails; dropped if no listeners are subscribed.
	pub fn emit(&self, event: Event) {
		let _ = self.tx.send(event);
	}

	/// Subscribe to events from now on.
	///
	/// Past events are not replayed. Snapshot the Store first if needed.
	pub fn subscribe(&self) -> broadcast::Receiver<Event> {
		self.tx.subscribe()
	}
}

impl Event {
	/// Log category for this event.
	///
	/// Maps to `run|task|session|meta|comms|llm|tool`.
	pub fn category(&self) -> &'static str {
		match self {
			Event::RunStarted(_) | Event::RunEnded(_) => "run",

			Event::TaskCreated(_) | Event::TaskStateChanged { .. } => "task",

			Event::SessionStarted(_)
			| Event::SessionStatusChanged { .. }
			| Event::MessageAppended { .. } => "session",

			Event::ReflectionRecorded { .. } | Event::LessonKept(_) => "meta",

			Event::MailReceived { .. }
			| Event::ChannelOpened { .. }
			| Event::Said { .. } => "comms",

			Event::CallQueued(_) | Event::CallStatusChanged { .. } => "llm",

			Event::ToolCalled { .. } | Event::ToolReturned { .. } => "tool",
		}
	}
}
