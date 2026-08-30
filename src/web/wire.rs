//! What a Watcher sees. The one place that decides that.
//!
//! Two frames. `init` carries every entity, once, when a browser connects.
//! `patch` carries what one Event changed. Patches name a bucket and an entity
//! rather than a field, so a browser merges one without knowing its shape, and
//! every connection begins with a fresh `init`, so reconnecting needs no replay.
//!
//! Nothing here recomputes anything: every field on the wire comes off the value
//! the Store handed over.
//!
//! Defines: [`Frame`], [`Bucket`], [`patch_for`], [`init_frame`].

use crate::event::Event;
use crate::store::Snapshot;

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
	/// Everything, on connect.
	Init { state: serde_json::Value, spend: serde_json::Value },
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
pub fn init_frame(_snapshot: &Snapshot, _spend: crate::domain::Spend) -> Frame {
	unimplemented!()
}

/// What one Event means to a browser.
///
/// Some Events change nothing a Watcher shows, and produce nothing.
pub fn patch_for(_event: &Event) -> Option<Frame> {
	unimplemented!()
}
