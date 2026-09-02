//! Event → Frame: what a Watcher sees, decided in one place.
//!
//! Construct: no state — `init_frame(&Snapshot, Spend, Vec<String>) -> Frame::Init`
//! and `patch_for(&Store, &Event) -> Option<Frame>` are pure translators.
//! Use: `server::watch` sends one `Init` on connect (full `Snapshot` plus
//! `Spend`, `Run` and the log so far), then one `patch_for` per `Event`;
//! `Ranked` and `Logged` are built in `server`, not here.
//! Consumers: browser JS over `/ws` via `server::watch` — sole external
//! consumer; `Store` is read for fresh entities, `Events` is the input bus.
//! Seam: `Event` → `Frame` translation lives only here; `server` owns
//! transport, broadcast-lag recovery, and the two Watcher writes (`Say`/`Find`).
//!
//! | `Event` | `Frame` |
//! | --- | --- |
//! | `TaskCreated` / `TaskStateChanged` / `TaskReArmed` | `Patch(Tasks)` — whole Task re-read, browser replaces |
//! | `SessionStarted` / `SessionStatusChanged` / `ReflectionRecorded` | `Patch(Sessions)` — whole Session re-read |
//! | `CallQueued` / `CallStatusChanged` | `Patch(Calls)` — whole Call re-read |
//! | `ChannelOpened` / `Said` | `Patch(Channels)` — whole Channel re-read |
//! | `LessonKept` | `Patch(Lessons)` — lesson as kept |
//! | `MessageAppended` | `Appended` — single message, no conversation resend |
//! | `RunStarted` / `RunEnded` / `MailReceived` / `ToolCalled` / `ToolReturned` | `None` — nothing a Watcher shows |
//!
//! Call trace: `watch → init_frame(snapshot) → send(Init)` then
//! `events.recv → patch_for → send(Patch/Appended)`; `Lagged` is handled in
//! `server` with a fresh `Init`, not here.
//! Rules: **patch carries whole entity, never a delta.**
//! **nothing recomputed — every wire field comes off the Store value.**
//! **reconnect gets fresh `Init`, no replay.**
//! **broadcast is lossy — slow Watcher loses Events, never slows the swarm.**
//!
//! Log lines are not `Event`s and never come through here: `server` reads them
//! off the `Logger` and sends `Logged`.
//!
//! Defines: [`Frame`], [`Bucket`], [`patch_for`], [`init_frame`].

use crate::event::Event;
use crate::store::{Snapshot, Store};

/// Collection a patched entity belongs to on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Bucket {
	Tasks,
	Sessions,
	Calls,
	Channels,
	Lessons,
}

/// One frame sent to a Watcher browser.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Frame {
	/// Everything on connect — every entity plus `Spend`, `Run` and the log so far.
	Init {
		state: serde_json::Value,
		spend: serde_json::Value,
		run: serde_json::Value,
		logs: Vec<String>,
	},
	/// One entity that changed — whole current value.
	Patch {
		bucket: Bucket,
		id: String,
		entity: serde_json::Value,
	},
	/// One message appended, without resending the conversation.
	Appended {
		session: String,
		index: usize,
		message: serde_json::Value,
	},
	/// Answer to a Lessons search — ids and scores in rank order.
	Ranked { query: String, hits: Vec<(String, f32)> },
	/// One line the `Logger` wrote, verbatim.
	Logged { line: String },
}

/// Build the first frame a browser gets.
///
/// Maps every entity in the `Snapshot` by id and bundles `Spend`, `Run` and
/// every log line written so far — `Logged` carries the ones after it.
pub fn init_frame(
	snapshot: &Snapshot,
	spend: crate::domain::Spend,
	logs: Vec<String>,
) -> Frame {
	// Build id map
	fn map<T: serde::Serialize>(
		items: &[T],
		id: impl Fn(&T) -> String,
	) -> serde_json::Value {
		serde_json::Value::Object(
			items
				.iter()
				.map(|item| (id(item), serde_json::to_value(item).unwrap()))
				.collect(),
		)
	}

	// Assemble init payload
	let state = serde_json::json!({
		"tasks": map(&snapshot.tasks, |t| t.id.to_string()),
		"sessions": map(&snapshot.sessions, |s| s.id.to_string()),
		"calls": map(&snapshot.calls, |c| c.id.to_string()),
		"channels": map(&snapshot.channels, |c| c.id.to_string()),
		"lessons": map(&snapshot.lessons, |l| l.id.to_string()),
	});

	Frame::Init {
		state,
		spend: serde_json::to_value(spend).unwrap(),
		run: serde_json::to_value(&snapshot.run).unwrap(),
		logs,
	}
}

/// Translate one `Event` into a Watcher frame.
///
/// Returns `None` when the Event changes nothing a Watcher shows.
/// Re-reads the entity from the `Store` so a `Patch` carries the whole current value.
pub fn patch_for(store: &Store, event: &Event) -> Option<Frame> {
	// Build patch helper
	let patch = |bucket: Bucket, id: String, entity: serde_json::Value| {
		Some(Frame::Patch { bucket, id, entity })
	};

	// Map event to frame
	match event {
		// Run lifecycle - nothing a Watcher shows
		Event::RunStarted(_) | Event::RunEnded(_) => None,

		// Task created - patch directly
		Event::TaskCreated(task) => {
			patch(Bucket::Tasks, task.id.to_string(), json(task))
		},
		// Task changed or re-armed - re-read then patch
		Event::TaskStateChanged { task, .. }
		| Event::TaskReArmed { task, .. } => {
			let task = store.task(*task).ok().flatten()?;
			patch(Bucket::Tasks, task.id.to_string(), json(&task))
		},

		// Session started - patch directly
		Event::SessionStarted(session) => {
			patch(Bucket::Sessions, session.id.to_string(), json(session))
		},
		// Session or reflection changed - re-read then patch
		Event::SessionStatusChanged { session, .. }
		| Event::ReflectionRecorded { session, .. } => {
			let session = store.session(*session).ok().flatten()?;
			patch(Bucket::Sessions, session.id.to_string(), json(&session))
		},
		// Message appended - single message
		Event::MessageAppended { session, index, message } => {
			Some(Frame::Appended {
				session: session.to_string(),
				index: *index,
				message: json(message),
			})
		},
		// Mail received - not shown
		Event::MailReceived { .. } => None,

		// Call queued - patch directly
		Event::CallQueued(call) => {
			patch(Bucket::Calls, call.id.to_string(), json(call))
		},
		// Call changed - re-read then patch
		Event::CallStatusChanged { call, .. } => {
			let call = store.call(*call).ok().flatten()?;
			patch(Bucket::Calls, call.id.to_string(), json(&call))
		},

		// Channel opened or said - re-read then patch
		Event::ChannelOpened { channel, .. } | Event::Said { channel, .. } => {
			let channel = store.channel(*channel).ok().flatten()?;
			patch(Bucket::Channels, channel.id.to_string(), json(&channel))
		},

		// Lesson kept - patch directly
		Event::LessonKept(lesson) => {
			patch(Bucket::Lessons, lesson.id.to_string(), json(lesson))
		},

		// Tool activity - not shown
		Event::ToolCalled { .. } | Event::ToolReturned { .. } => None,
	}
}

/// Serialize a domain value to JSON.
fn json<T: serde::Serialize>(value: &T) -> serde_json::Value {
	serde_json::to_value(value).expect("domain values always serialize")
}
