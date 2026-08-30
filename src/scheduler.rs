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
	CallId, CallRequest, CallStatus, Completion, NewCall, SessionId,
	TaskPriority, Timestamp, Usage,
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
	fn from(p: TaskPriority) -> Tier {
		match p {
			TaskPriority::High => Tier::TaskHigh,
			TaskPriority::Normal => Tier::TaskNormal,
			TaskPriority::Low => Tier::TaskLow,
		}
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
	/// Whether the one slot is taken. A waiter that is granted the slot sees
	/// this already `true` — it never flips it itself.
	in_flight: bool,
	/// Counts up, never down: arrival order within a [`Tier`].
	next_arrival: u64,
	waiting: Vec<Waiting>,
}

/// One call, registered and waiting for the slot. `notify` is single-use: at
/// most one `notify_one` is ever sent on it, by whichever call releases the
/// slot next.
struct Waiting {
	tier: Tier,
	arrival: u64,
	notify: Arc<tokio::sync::Notify>,
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
	pub fn new(model: Arc<dyn Model>, store: Arc<Store>) -> Self {
		Scheduler {
			model,
			store,
			inner: tokio::sync::Mutex::new(Inner {
				in_flight: false,
				next_arrival: 0,
				waiting: Vec::new(),
			}),
		}
	}

	/// Ask the model, and leave a full record of the exchange in the Store
	/// whatever happens.
	///
	/// The call is recorded the moment it joins the queue, so a Watcher sees it
	/// waiting. It is sent when it reaches the front and nothing else is in
	/// flight.
	///
	/// One timestamp stands for the whole exchange — the Scheduler has no
	/// [`crate::domain::Clock`] of its own, only what the caller hands it — so
	/// `queued_at`, `sent_at` and `finished_at` all read the same instant. See
	/// `TASKS.md`.
	///
	/// The [`CallId`] comes back on both paths — beside the [`Completion`], and
	/// inside [`SchedulerError::Call`]. `reflect.rs` anchors a
	/// [`crate::domain::Reflection`] on it, and that record is not optional when
	/// the call fails. A Turn does not want it and drops it.
	pub async fn request(
		&self,
		session: SessionId,
		request: CallRequest,
		tier: Tier,
		now: Timestamp,
	) -> Result<(CallId, Completion), SchedulerError> {
		let id = self.store.queue_call(
			NewCall {
				session,
				tier,
				model: self.model.name().to_string(),
				request: request.clone(),
			},
			now,
		)?;

		self.acquire(tier).await;

		self.store
			.set_call_status(id, CallStatus::InFlight { sent_at: now })?;

		let outcome = self.model.send(&request).await;

		self.release().await;

		match outcome {
			Ok(completion) => {
				let usage =
					Usage { tokens: completion.tokens, cost: completion.cost };
				self.store.set_call_status(
					id,
					CallStatus::Done {
						sent_at: now,
						finished_at: now,
						reply: completion.reply.clone(),
						usage,
					},
				)?;
				Ok((id, completion))
			},
			Err(error) => {
				self.store.set_call_status(
					id,
					CallStatus::Failed {
						sent_at: now,
						finished_at: now,
						error: error.to_string(),
					},
				)?;
				Err(SchedulerError::Call { call: id, source: error })
			},
		}
	}

	/// How many calls are waiting. For a wind-down that wants to know whether
	/// anything can still spend.
	pub async fn waiting(&self) -> usize {
		self.inner.lock().await.waiting.len()
	}

	/// Register `(tier, arrival)` and block until this call holds the slot.
	async fn acquire(&self, tier: Tier) {
		let notify = Arc::new(tokio::sync::Notify::new());
		let mut inner = self.inner.lock().await;
		let arrival = inner.next_arrival;
		inner.next_arrival += 1;
		inner
			.waiting
			.push(Waiting { tier, arrival, notify: notify.clone() });
		Self::try_grant(&mut inner);
		drop(inner);
		notify.notified().await;
	}

	/// Free the slot, and hand it straight to whichever waiter now sorts lowest.
	async fn release(&self) {
		let mut inner = self.inner.lock().await;
		inner.in_flight = false;
		Self::try_grant(&mut inner);
	}

	/// If the slot is free and someone is waiting, give it to the lowest
	/// `(tier, arrival)` — a higher tier jumps every call still waiting, never
	/// the one already in flight.
	fn try_grant(inner: &mut Inner) {
		if inner.in_flight {
			return;
		}
		let Some(next) = inner
			.waiting
			.iter()
			.enumerate()
			.min_by_key(|(_, w)| (w.tier, w.arrival))
			.map(|(i, _)| i)
		else {
			return;
		};
		let winner = inner.waiting.remove(next);
		inner.in_flight = true;
		winner.notify.notify_one();
	}
}
