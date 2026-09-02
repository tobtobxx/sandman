//! One slot in front of the models, ordered by [`Tier`] then arrival.
//!
//! Construct: `Scheduler::new(models, store, clock)` — owns the queue; `Inner`
//! (`in_flight`, `waiting`, `next_arrival`) is private and only `try_grant` decides who runs next.
//! Use: `request(session, request, tier, purpose) -> (CallId, Completion)` queues as `Queued`,
//! waits for the slot, marks `InFlight`, sends via [`crate::model::Model`], records `Done|Failed`;
//! `waiting() -> usize` inspects depth for wind-down. `CallId` returns on both paths so reflection can anchor.
//! Consumers: `session::turn` (`Tier::from(TaskPriority)` / `Tier::Comms`), `reflect::interrupt`
//! (`Tier::Metacognition`), `Harness::wind_down` via `waiting`, Watchers via `CallQueued`/`CallStatusChanged`,
//! `SessionCtx` threads `Arc<Scheduler>` through every turn and tool.
//! Seam: **scheduler decides *when*, [`crate::model::Model`] decides *how*** — `Model` sits under the queue
//! so a scripted bench still exercises real tier ordering and one-at-a-time.
//!
//! | Tier | Caller | Purpose |
//! | --- | --- | --- |
//! | `Comms` (1) | Comms Session — human never behind swarm | `Purpose::Comms` |
//! | `TaskHigh` (2) | Worker on high Task | `Purpose::Work(role)` |
//! | `Metacognition` (3) | review / interrupt — not held behind work | `Purpose::Metacognition` |
//! | `TaskNormal` (4) | Worker on normal Task | `Purpose::Work(role)` |
//! | `TaskLow` (5) | Worker on low Task | `Purpose::Work(role)` |
//!
//! Rules: **exactly one `InFlight` across the whole Harness.** **higher Tier jumps waiting, never preempts in-flight — committed and paid.** **within one Tier arrival order alternates Workers.** **Tier fixed at queue time; declaration order is the policy and `repr u8` 1..=5 mirrors stored value.** **queued call visible before send.** **Store is the only writer; every status change emits one Event.** **release hands slot directly to lowest `(Tier, arrival)`.
//!
//! Defines: [`Tier`], [`Scheduler`], [`SchedulerError`].

use std::sync::Arc;

use crate::domain::{
	CallId, CallRequest, CallStatus, Clock, Completion, NewCall, SessionId,
	TaskPriority,
};
use crate::model::{ModelError, Models, Purpose};
use crate::store::Store;

/// Queue position. Lower runs first; declaration order is the policy.
/// `repr u8` 1..=5 mirrors the stored value so ordering and display cannot disagree.
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
	num_enum::IntoPrimitive,
	num_enum::TryFromPrimitive,
)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Tier {
	/// A human is never left behind the swarm.
	Comms = 1,
	/// A Worker on a `high` priority Task.
	TaskHigh = 2,
	/// Review or interrupt — not held behind ordinary work.
	Metacognition = 3,
	/// A Worker on a `normal` priority Task.
	TaskNormal = 4,
	/// A Worker on a `low` priority Task.
	TaskLow = 5,
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

/// The one queue in front of the models.
pub struct Scheduler {
	models: Models,
	store: Arc<Store>,
	clock: Arc<dyn Clock>,
	inner: tokio::sync::Mutex<Inner>,
}

/// Waiting calls and the one in flight. Private: only `try_grant` decides next.
struct Inner {
	/// Whether the one slot is taken. Granted waiter sees this already `true`.
	in_flight: bool,
	/// Counts up, never down — arrival order within a [`Tier`].
	next_arrival: u64,
	waiting: Vec<Waiting>,
}

/// One queued call waiting for the slot.
/// Single-use `notify`; at most one `notify_one` from the releaser.
struct Waiting {
	tier: Tier,
	arrival: u64,
	notify: Arc<tokio::sync::Notify>,
}

/// Failure from `request`.
/// `Call` carries the queued `CallId` for `FailedOpen` reflection; `Store` means nothing was queued.
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
	pub fn new(
		models: Models,
		store: Arc<Store>,
		clock: Arc<dyn Clock>,
	) -> Self {
		Scheduler {
			models,
			store,
			clock,
			inner: tokio::sync::Mutex::new(Inner {
				in_flight: false,
				next_arrival: 0,
				waiting: Vec::new(),
			}),
		}
	}

	/// Queue, send, and record one model exchange.
	/// Queued immediately so waiting is visible; `CallId` returns on success and on `Call` failure for reflection.
	pub async fn request(
		&self,
		session: SessionId,
		request: CallRequest,
		tier: Tier,
		purpose: Purpose,
	) -> Result<(CallId, Completion), SchedulerError> {
		// Queue call
		let model = self.models.pick(purpose);
		let id = self.store.queue_call(
			NewCall {
				session,
				tier,
				model: model.name().to_string(),
				request: request.clone(),
			},
			self.clock.now(),
		)?;

		// Acquire slot
		self.acquire(tier).await;

		// Mark in-flight
		let sent_at = self.clock.now();
		self.store
			.set_call_status(id, CallStatus::InFlight { sent_at })?;

		// Send request
		let outcome = model.send(&request).await;
		let finished_at = self.clock.now();

		// Release slot
		self.release().await;

		// Persist result
		match outcome {
			// Call succeeded - record Done
			Ok(completion) => {
				self.store.set_call_status(
					id,
					CallStatus::Done {
						sent_at,
						finished_at,
						reply: completion.reply.clone(),
						usage: completion.usage,
					},
				)?;
				Ok((id, completion))
			},
			// Call failed - record Failed
			Err(error) => {
				self.store.set_call_status(
					id,
					CallStatus::Failed {
						sent_at,
						finished_at,
						error: error.to_string(),
					},
				)?;
				Err(SchedulerError::Call { call: id, source: error })
			},
		}
	}

	/// Number of calls waiting for the slot.
	/// Used by wind-down to check whether spend is still possible.
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

	/// Free the slot and hand it to the lowest waiting `(tier, arrival)`.
	async fn release(&self) {
		let mut inner = self.inner.lock().await;
		inner.in_flight = false;
		Self::try_grant(&mut inner);
	}

	/// Grant the slot to the lowest `(tier, arrival)` if free.
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
