//! One model exchange, visible from queue time to finish.
//!
//! A call is recorded when it joins the scheduler, not when it is sent —
//! waiting is as visible as talking. Data valid only after sending
//! (`sent_at`, `reply`, `usage`, `cost`) lives inside [`CallStatus`], so a
//! `Queued` call cannot carry a reply and a `Done` call always does.
//!
//! Construct via [`NewCall`] at [`crate::store::Store::queue_call`] — the Store
//! mints [`CallId`] and stamps `queued_at` in the same transaction. The request
//! is frozen at that instant (`messages` + `tools`), never updated as the
//! Session's transcript grows afterwards.
//!
//! Use through [`crate::scheduler::Scheduler::request`] which drives the
//! lifecycle and [`crate::model::Model::send`] which fills the reply:
//! ```text
//! queue_call(NewCall) → Queued → set InFlight {sent_at} → Model::send → Done|Failed
//! recover (new Run)  → Dropped {at} for anything still Queued|InFlight
//! ```
//!
//! Consumers handle the same [`CallStatus`] differently:
//!
//! | Status | Store | Scheduler | Model | Spend | Watchers/Events |
//! | --- | --- | --- | --- | --- | --- |
//! | `Queued` | persists `tier`+`request` | waits by `(Tier, arrival)` | — | ignored | `CallQueued` |
//! | `InFlight` | records `sent_at` | holds the one slot | `send(&CallRequest)` | ignored | `CallStatusChanged` |
//! | `Done` | records `reply`+`Usage` | releases slot | returns `Completion` | summed | `CallStatusChanged` |
//! | `Failed` | records `error` | releases slot | returns `ModelError` | ignored | `CallStatusChanged` |
//! | `Dropped` | set by `recover` on stale `Queued`/`InFlight` | — | — | ignored | `CallStatusChanged` |
//!
//! Rules / asymmetry:
//!
//! - **Tier is fixed at queue time and never changes.** `Tier` orders waiting calls; same-tier arrival order alternates workers.
//! - **At most one `InFlight` across the whole Harness.** Scheduler grants by `(Tier, arrival)`; higher tier jumps waiting calls, never preempts the one in flight.
//! - **`Done` always has `reply` and `Usage`; `Queued` never has `sent_at`.** `Dropped` has no `sent_at` either — it is not a `Failed` inventing one for a call never sent.
//! - **Only `Done` contributes to [`crate::domain::Spend`].** `Dropped` costs nothing knowable; `Failed` is one attempt, no retry, with a path to a failed `TaskResult`.
//! - **Metacognition has no Session.** Its calls are recorded against the Session judged, so cost lands where the work is.
//! - **Store is the only writer; scheduler decides *when*, `Model` decides *how*.** `Model` sits under the scheduler so scripted benches still exercise the real queue.
//!
//! Defines: [`LlmCall`], [`CallStatus`], [`CallRequest`], [`NewCall`], [`Usage`].

use super::ids::{CallId, RunId, SessionId};
use super::message::{Message, Reply, ToolSchema};
use super::time::{Cost, Timestamp};
use crate::scheduler::Tier;

/// One exchange with the model, owned by a Session.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LlmCall {
	pub id: CallId,
	pub run: RunId,
	/// Session that pays; metacognition records against the judged Session.
	pub session: SessionId,
	/// Priority at queue time; never changes.
	pub tier: Tier,
	pub model: String,
	pub request: CallRequest,
	pub queued_at: Timestamp,
	pub status: CallStatus,
}

/// Frozen request sent to the model.
///
/// Captured at queue time so later transcript growth does not alter it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CallRequest {
	pub messages: Vec<Message>,
	pub tools: Vec<ToolSchema>,
}

/// Lifecycle of a call, and data valid in each state.
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
	/// Waiting for the one scheduler slot.
	Queued,
	/// Out with the model.
	InFlight { sent_at: Timestamp },
	/// Succeeded with reply and usage.
	Done {
		sent_at: Timestamp,
		finished_at: Timestamp,
		reply: Reply,
		usage: Usage,
	},
	/// Failed after one attempt; no retry.
	Failed {
		sent_at: Timestamp,
		finished_at: Timestamp,
		error: String,
	},
	/// Abandoned by a dead Run; ignored by Spend.
	Dropped { at: Timestamp },
}

/// Tokens and cost of one finished call.
///
/// The prompt is split by what the provider had to compute: `cached` was
/// served from its prefix cache, `prefill` was processed for this call. The two
/// add up to the prompt sent — a long transcript re-sent unchanged is nearly
/// all `cached`, and that is the difference between a fast turn and a slow one.
#[derive(
	Debug,
	Clone,
	Copy,
	Default,
	PartialEq,
	Eq,
	serde::Serialize,
	serde::Deserialize,
)]
pub struct Usage {
	/// Prompt tokens the provider read back from its cache.
	pub cached: u64,
	/// Prompt tokens processed for this call; excludes `cached`.
	pub prefill: u64,
	/// Tokens generated, reasoning included.
	pub produced: u64,
	pub cost: Cost,
}

/// Inputs to queue a call. Store mints the id.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NewCall {
	pub session: SessionId,
	pub tier: Tier,
	pub model: String,
	pub request: CallRequest,
}

impl CallStatus {
	/// Usage if this call finished successfully.
	///
	/// `Some` only for `Done`; used to sum [`crate::domain::Spend`].
	pub fn usage(&self) -> Option<Usage> {
		match self {
			CallStatus::Done { usage, .. } => Some(*usage),
			_ => None,
		}
	}

	/// Whether this call still expects a result.
	///
	/// True for `Queued` and `InFlight`, false otherwise.
	pub fn is_outstanding(&self) -> bool {
		matches!(self, CallStatus::Queued | CallStatus::InFlight { .. })
	}
}
