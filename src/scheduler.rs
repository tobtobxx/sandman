//! The central model-call scheduler.
//!
//! Every model call in the whole Harness goes through here. Exactly one is in
//! flight at any moment, and the rest wait ordered by [`Tier`], then by arrival
//! within a tier.
//!
//! One call at a time is deliberate. It makes a run possible to follow, which is
//! the only guard against runaway work — there is no budget on a turn and no cap
//! on how many Tasks the swarm creates, so the human watching is the guard rail.
//!
//! A higher-priority call that arrives while a lower one waits jumps ahead of it
//! in the waiting queue. It never aborts the call already in flight: that one is
//! committed and paid. So "skip the queue" means skip the *waiting* calls, not
//! preempt the one with the model. Within one tier, arrival order decides, which
//! is what makes two same-tier Workers alternate at the model-call level — each
//! one's next call lands behind the other's call that was already waiting while
//! its own was in flight.
//!
//! Priority is a property of the caller, not of the call. A Comms Session passes
//! [`Tier::Comms`], a Worker passes its Task's tier, metacognition passes
//! [`Tier::Metacognition`].
//!
//! The scheduler decides *when*; the [`crate::model::Model`] seam decides *how*.
//!
//! Defines: [`Tier`], [`Scheduler`], [`SchedulerError`].

use std::sync::Arc;

use crate::domain::{
	CallId, CallRequest, Completion, SessionId, TaskPriority, Timestamp,
};
use crate::model::{Model, ModelError};
use crate::store::Store;

/// Where a call waits. Lower runs first, and the derived ordering is the
/// ordering — declaration order is the policy.
#[derive(
	Debug,
	Clone,
	Copy,
	PartialEq,
	Eq,
	PartialOrd,
	Ord,
	Hash,
	serde::Serialize,
	serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
	/// A human is never left behind the swarm.
	Comms,
	/// A Worker on a `high` priority Task.
	TaskHigh,
	/// A review or an interrupt, so metacognition is not held behind ordinary
	/// work.
	Metacognition,
	/// A Worker on a `normal` priority Task.
	TaskNormal,
	/// A Worker on a `low` priority Task.
	TaskLow,
}

impl From<TaskPriority> for Tier {
	fn from(_p: TaskPriority) -> Tier {
		unimplemented!()
	}
}

impl Tier {
	/// 1 to 5, as the call record and the Watcher show it.
	pub fn as_number(&self) -> u8 {
		match self {
			Tier::Comms => 1,
			Tier::TaskHigh => 2,
			Tier::Metacognition => 3,
			Tier::TaskNormal => 4,
			Tier::TaskLow => 5,
		}
	}
}

/// The one queue in front of the model.
pub struct Scheduler {
	model: Arc<dyn Model>,
	store: Arc<Store>,
	inner: tokio::sync::Mutex<Inner>,
}

/// The waiting calls and the one in flight. Private: nothing outside decides
/// what runs next.
struct Inner {
	_private: (),
}

/// What can go wrong asking for a model call.
///
/// [`SchedulerError::Call`] carries the [`CallId`] of the exchange that failed.
/// The call record exists from the moment it joined the queue, so a failure has
/// one to name — which is what lets metacognition record a `FailedOpen`
/// reflection against the call it could not make. [`SchedulerError::Store`] is
/// the one case with no id at all: nothing was ever queued.
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
	#[error("{source}")]
	Call {
		call: CallId,
		#[source]
		source: ModelError,
	},
	#[error(transparent)]
	Store(#[from] crate::store::StoreError),
}

impl Scheduler {
	pub fn new(_model: Arc<dyn Model>, _store: Arc<Store>) -> Self {
		unimplemented!()
	}

	/// Ask the model, and leave a full record of the exchange in the Store
	/// whatever happens.
	///
	/// The call is recorded the moment it joins the queue, so a Watcher sees it
	/// waiting. It is sent when it reaches the front and nothing else is in
	/// flight.
	///
	/// The [`CallId`] comes back on both paths — beside the [`Completion`], and
	/// inside [`SchedulerError::Call`]. `reflect.rs` anchors a
	/// [`crate::domain::Reflection`] on it, and that record is not optional when
	/// the call fails. A Turn does not want it and drops it.
	pub async fn request(
		&self,
		_session: SessionId,
		_request: CallRequest,
		_tier: Tier,
		_now: Timestamp,
	) -> Result<(CallId, Completion), SchedulerError> {
		unimplemented!()
	}

	/// How many calls are waiting. For a wind-down that wants to know whether
	/// anything can still spend.
	pub async fn waiting(&self) -> usize {
		unimplemented!()
	}
}
