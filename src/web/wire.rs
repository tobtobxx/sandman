//! What a Watcher sees. The one place that decides that.
//!
//! Two frames. `init` carries every entity, once, when a browser connects.
//! `patch` carries what one Event changed. A Patch always carries the whole
//! current entity, fetched fresh from the Store by the id the Event named, so
//! a browser can replace what it holds for that id outright rather than
//! merging fields into it. Every connection begins with a fresh `init`, so
//! reconnecting needs no replay.
//!
//! Nothing here recomputes anything: every field on the wire comes off the
//! value the Store handed over, never derived or summed here.
//!
//! Defines: [`Frame`], [`Bucket`], [`patch_for`], [`init_frame`].

use crate::event::Event;
use crate::store::{Snapshot, Store};

/// Which collection an entity belongs to on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Bucket {
	Tasks,
	Sessions,
	Calls,
	Channels,
	Lessons,
}

/// One thing sent to a browser.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Frame {
	/// Everything, on connect. The Run rides along so a Watcher can say how
	/// long Sandman has been up without timing its own connection — a
	/// reconnect must not look like a restart.
	Init {
		state: serde_json::Value,
		spend: serde_json::Value,
		run: serde_json::Value,
	},
	/// One entity that changed.
	Patch {
		bucket: Bucket,
		id: String,
		entity: serde_json::Value,
	},
	/// One message appended to a Session, without resending the conversation.
	Appended {
		session: String,
		index: usize,
		message: serde_json::Value,
	},
	/// The answer to a Lessons search: ids and scores, in order.
	Ranked { query: String, hits: Vec<(String, f32)> },
}

/// The first frame a browser gets.
pub fn init_frame(snapshot: &Snapshot, spend: crate::domain::Spend) -> Frame {
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
	}
}

/// What one Event means to a browser.
///
/// Some Events change nothing a Watcher shows, and produce nothing. Every
/// Patch carries the whole current entity — fetched fresh from the Store —
/// rather than just the field the Event named, so a browser never has to
/// merge a partial record: it replaces what it holds for that id outright.
pub fn patch_for(store: &Store, event: &Event) -> Option<Frame> {
	let patch = |bucket: Bucket, id: String, entity: serde_json::Value| {
		Some(Frame::Patch { bucket, id, entity })
	};

	match event {
		Event::RunStarted(_) | Event::RunEnded(_) => None,

		Event::TaskCreated(task) => {
			patch(Bucket::Tasks, task.id.to_string(), json(task))
		},
		Event::TaskStateChanged { task, .. } => {
			let task = store.task(*task).ok().flatten()?;
			patch(Bucket::Tasks, task.id.to_string(), json(&task))
		},

		Event::SessionStarted(session) => {
			patch(Bucket::Sessions, session.id.to_string(), json(session))
		},
		Event::SessionStatusChanged { session, .. }
		| Event::ReflectionRecorded { session, .. } => {
			let session = store.session(*session).ok().flatten()?;
			patch(Bucket::Sessions, session.id.to_string(), json(&session))
		},
		Event::MessageAppended { session, index, message } => {
			Some(Frame::Appended {
				session: session.to_string(),
				index: *index,
				message: json(message),
			})
		},
		Event::MailReceived { .. } => None,

		Event::CallQueued(call) => {
			patch(Bucket::Calls, call.id.to_string(), json(call))
		},
		Event::CallStatusChanged { call, .. } => {
			let call = store.call(*call).ok().flatten()?;
			patch(Bucket::Calls, call.id.to_string(), json(&call))
		},

		Event::ChannelOpened { channel, .. } | Event::Said { channel, .. } => {
			let channel = store.channel(*channel).ok().flatten()?;
			patch(Bucket::Channels, channel.id.to_string(), json(&channel))
		},

		Event::LessonKept(lesson) => {
			patch(Bucket::Lessons, lesson.id.to_string(), json(lesson))
		},

		Event::ToolCalled { .. } | Event::ToolReturned { .. } => None,
	}
}

/// Shorthand for the one thing every arm above does to its payload.
fn json<T: serde::Serialize>(value: &T) -> serde_json::Value {
	serde_json::to_value(value).expect("domain values always serialize")
}
