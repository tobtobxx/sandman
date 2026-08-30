//! The model call: one exchange with the model, belonging to a Session.
//!
//! A call exists from the moment it joins the scheduler's queue, not from when
//! it is sent, so waiting for the model is as visible from outside as talking to
//! it. Everything that is only true once a call has been sent — when it went out,
//! when it came back, what came back, what it cost — lives inside
//! [`CallStatus`]. A call that is `Done` therefore always has a reply and a
//! usage record, and one that is `Queued` cannot pretend to have either.
//!
//! Defines: [`LlmCall`], [`CallStatus`], [`CallRequest`], [`NewCall`],
//! [`Usage`].

use super::ids::{CallId, RunId, SessionId};
use super::message::{Message, Reply, ToolSchema};
use super::time::{Cost, Timestamp};
use crate::scheduler::Tier;

/// One exchange with the model.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LlmCall {
	pub id: CallId,
	pub run: RunId,
	/// The Session this call belongs to. Metacognition has no Session of its
	/// own, so its calls are recorded against the Session it judges — its cost
	/// lands where the work is.
	pub session: SessionId,
	/// Where this call waited. Set when it joins the queue and never changed, so
	/// a Watcher can see what was prioritised over what.
	pub tier: Tier,
	pub model: String,
	pub request: CallRequest,
	pub queued_at: Timestamp,
	pub status: CallStatus,
}

/// What was sent, recorded as it was sent rather than as the Session went on to
/// accumulate afterwards.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CallRequest {
	pub messages: Vec<Message>,
	pub tools: Vec<ToolSchema>,
}

/// Where a call is, and everything that depends on being there.
#[derive(
	Debug,
	Clone,
	PartialEq,
	serde::Serialize,
	serde::Deserialize,
	strum::Display,
	strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum CallStatus {
	/// In the scheduler's queue, waiting for the one slot.
	Queued,
	/// With the model. At most one call is ever here.
	InFlight { sent_at: Timestamp },
	Done {
		sent_at: Timestamp,
		finished_at: Timestamp,
		reply: Reply,
		usage: Usage,
	},
	/// One attempt, no retry: a failed call already has a full path to a failed
	/// Result.
	Failed {
		sent_at: Timestamp,
		finished_at: Timestamp,
		error: String,
	},
}

/// What one finished call consumed.
#[derive(
	Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
pub struct Usage {
	pub tokens: u64,
	pub cost: Cost,
}

/// Everything needed to record a call as it joins the queue. The Store mints the
/// id.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NewCall {
	pub session: SessionId,
	pub tier: Tier,
	pub model: String,
	pub request: CallRequest,
}

impl CallStatus {
	/// What this call consumed, if it finished successfully. Spend sums these.
	pub fn usage(&self) -> Option<Usage> {
		match self {
			CallStatus::Done { usage, .. } => Some(*usage),
			_ => None,
		}
	}

	/// Whether this call is still expected to produce something.
	pub fn is_outstanding(&self) -> bool {
		matches!(self, CallStatus::Queued | CallStatus::InFlight { .. })
	}
}
