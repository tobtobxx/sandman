//! One entry on the time-ordered queue.
//!
//! A `Task` is the single unit of work — human request, investigation, and
//! delegated work are the same type. `TaskState` and `Schedule` make invalid
//! states unrepresentable: a completed task always has a result, a pending one
//! never does, a running one always names its `Session`, a repeating task always
//! has an anchor.
//!
//! Construct via [`NewTask`] at [`crate::store::Store::create_task`] — the Store
//! mints [`TaskId`] and stamps `created_at` in the same transaction, derives
//! `subscriber` from [`Creator`] (so no caller chooses it), and emits
//! `TaskCreated`. No id without a row; no second way in.
//!
//! Use through [`crate::store::Store`]: `next_pending(now)` picks by
//! `not_before`, `start_task` `Pending→Running`, `complete_task` and
//! `cancel_tasks` reach terminals, `Schedule::next_occurrence` arms the next
//! chain link anchored to schedule not wall-clock. `Schedule::from_offsets` is
//! the single parser for tool, control, and CLI.
//!
//! Consumers — same `TaskState`/`Schedule` handled differently:
//!
//! | State | Store (only writer) | Harness / Worker / Comms | Review / Events |
//! | --- | --- | --- | --- |
//! | `Pending` | enqueues with `not_before` | `next_pending` picks by time | `TaskCreated` |
//! | `Running` | records `session`+`started_at` | worker drives `Turn`s; cancel checked each turn | `TaskStateChanged` |
//! | `Completed` | persists `result`+`at`; terminal | review `Complete`→`Succeeded`, `Unreachable`→`Failed` | `TaskStateChanged`+delivery |
//! | `Cancelled` | terminal, no result, stops repeating chain | session ends at next decision; waiters resolved | `TaskStateChanged`+`render_cancelled` |
//!
//! | Schedule | `not_before` | `next_occurrence` | pick |
//! | --- | --- | --- | --- |
//! | `Now` | `None` | `None` | immediate |
//! | `At(t)` | `Some(t)` | `None` | `t <= now` |
//! | `Repeating{first,every}` | `Some(first)` | `Some(first+every)` | `first <= now`, anchored |
//!
//! Seam: domain is data — `Store` owns rows and Events, scheduling owns
//! time, `Turn` owns policy. `TaskState`/`Schedule` never decide; callers match them.
//!
//! Rules: **one task concept — human, investigation, and delegated work are the same type.** **`Completed` has `TaskResult`, `Cancelled` has none.** **`Cancelled` is terminal and chain-ending.** **pick is time only; `await_result` is not a queue wait.** **`subscriber` derived from `Creator`, never chosen.** **only review completes; only unreachable fails without it.** **store is the only writer; `Tier`≠`TaskPriority`.**
//!
//! Defines: [`Task`], [`TaskState`]/[`TaskStateName`], [`TaskResult`], [`Schedule`], [`TaskPriority`], [`Creator`], [`NewTask`], [`TaskSummary`]

use super::ids::{ChannelId, RunId, SessionId, TaskId};
use super::text::{Brief, Title};
use super::time::{Duration, Timestamp};
use crate::roles::RoleName;

/// One piece of work.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Task {
	pub id: TaskId,
	/// The Run this Task belongs to. Spend is scoped to a Run; lessons and
	/// past tasks are searched across every Run.
	pub run: RunId,
	pub title: Title,
	/// The only thing the Worker gets. Must stand alone.
	pub brief: Brief,
	pub role: RoleName,
	pub state: TaskState,
	pub schedule: Schedule,
	/// Channel awaiting the Result, if any. Derived from [`Creator`] by the
	/// Store — `Some` for a `Comms` Session's channel, `None` for workers,
	/// `Cli`, and `Control`. A task without a subscriber still records its Result.
	pub subscriber: Option<ChannelId>,
	/// How urgently the swarm should spend a model call on this task.
	pub priority: TaskPriority,
	pub created_by: Creator,
	pub created_at: Timestamp,
}

/// Where a task is in its life, and data valid in each state.
///
/// [`TaskStateName`] is the same set without payloads — the tag persisted to
/// `tasks.state` and the only value a filter can name.
#[derive(
	Debug,
	Clone,
	PartialEq,
	Eq,
	serde::Serialize,
	serde::Deserialize,
	strum::EnumDiscriminants,
)]
#[serde(rename_all = "snake_case")]
#[strum_discriminants(name(TaskStateName))]
#[strum_discriminants(derive(
	strum::Display,
	strum::EnumString,
	strum::IntoStaticStr,
	strum::VariantArray
))]
#[strum_discriminants(strum(serialize_all = "snake_case"))]
pub enum TaskState {
	/// Waiting on the queue. Picked on one condition: time.
	Pending,
	/// Held by a session. Naming it lets cancellation reach `await_result`.
	Running { session: SessionId, started_at: Timestamp },
	/// Done, with a result — success or failure. Terminal.
	Completed { result: TaskResult, at: Timestamp },
	/// Stopped before a result. Terminal; no result; ends a repeating chain.
	Cancelled { at: Timestamp },
}

/// What a session produced for its task.
///
/// A failure is a result saying so, not the absence of one.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskResult {
	Succeeded(String),
	Failed(String),
}

/// When a task may run, and whether finishing it arms another.
#[derive(
	Debug,
	Clone,
	Copy,
	PartialEq,
	Eq,
	serde::Serialize,
	serde::Deserialize,
	strum::Display,
	strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Schedule {
	/// As soon as the queue reaches it.
	Now,
	/// Not before this instant.
	At(Timestamp),
	/// Chain: completing one creates the next, anchored to schedule not end time.
	Repeating { first: Timestamp, every: Duration },
}

/// How urgently the swarm should spend a call on this task.
///
/// Distinct from [`crate::scheduler::Tier`]: priority is the property of the
/// work, tier is the position in the queue.
#[derive(
	Debug,
	Clone,
	Copy,
	Default,
	PartialEq,
	Eq,
	PartialOrd,
	Ord,
	serde::Serialize,
	serde::Deserialize,
	strum::Display,
	strum::EnumString,
	strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TaskPriority {
	High,
	#[default]
	Normal,
	Low,
}

/// Who put this task on the queue.
#[derive(
	Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Creator {
	/// A session, through one of the create-task tools.
	Session(SessionId),
	/// A one-shot run from the command line.
	Cli,
	/// Another process, through the control socket.
	Control,
}

/// Everything needed to put a task on the queue. Store mints the id.
///
/// No `subscriber`: it follows from `created_by`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NewTask {
	pub title: Title,
	pub brief: Brief,
	pub role: RoleName,
	pub schedule: Schedule,
	pub priority: TaskPriority,
	pub created_by: Creator,
}

/// A task as `list_tasks` and the control socket report it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TaskSummary {
	pub id: TaskId,
	pub title: Title,
	pub role: RoleName,
	pub state: TaskState,
	pub schedule: Schedule,
	pub created_at: Timestamp,
}

impl TaskState {
	/// Whether this state is terminal — no further transition.
	pub fn is_terminal(&self) -> bool {
		matches!(
			self,
			TaskState::Completed { .. } | TaskState::Cancelled { .. }
		)
	}
}

impl TaskResult {
	/// Text of the result, success or failure.
	pub fn content(&self) -> &str {
		match self {
			TaskResult::Succeeded(text) => text,
			TaskResult::Failed(text) => text,
		}
	}
}

impl Schedule {
	/// Earliest instant this schedule may run, if any.
	pub fn not_before(&self, _created_at: Timestamp) -> Option<Timestamp> {
		match self {
			Schedule::Now => None,
			Schedule::At(t) => Some(*t),
			Schedule::Repeating { first, .. } => Some(*first),
		}
	}

	/// Schedule for the next occurrence, if repeating.
	///
	/// Anchored to schedule, not to when the run ended, so drift does not accumulate.
	pub fn next_occurrence(&self) -> Option<Schedule> {
		match self {
			Schedule::Repeating { first, every } => Some(Schedule::Repeating {
				first: first.plus(*every),
				every: *every,
			}),
			_ => None,
		}
	}

	/// Build a schedule from delay and repeat offsets in seconds.
	///
	/// Single parser for tool, control, and CLI so the three cannot drift.
	pub fn from_offsets(
		run_at_seconds: Option<i64>,
		repeat_seconds: Option<i64>,
		now: Timestamp,
	) -> Schedule {
		// Compute first instant
		let first = now.plus(Duration::from_secs(run_at_seconds.unwrap_or(0)));
		// Choose schedule
		match repeat_seconds {
			Some(secs) => {
				Schedule::Repeating { first, every: Duration::from_secs(secs) }
			},
			None if run_at_seconds.is_some() => Schedule::At(first),
			None => Schedule::Now,
		}
	}
}

impl Task {
	/// Text that crosses between agents as this task's answer.
	pub fn render_answer(&self) -> String {
		let content = match &self.state {
			TaskState::Completed { result, .. } => result.content(),
			_ => "",
		};
		format!("Answer to \"{}\":\n{}", self.title, content)
	}

	/// Notice sent where a result would have gone when cancelled.
	pub fn render_cancelled(&self) -> String {
		format!("Task \"{}\" was cancelled.", self.title)
	}
}
