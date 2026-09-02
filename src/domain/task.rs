//! One entry on the time-ordered queue.
//!
//! A `Task` is the single unit of work — human request, investigation, and
//! delegated work are the same type. `TaskState` and `Schedule` make invalid
//! states unrepresentable: a completed task always has a result, a pending one
//! never does, a running one always names its `Session`, a cron task always
//! knows when it next comes round.
//!
//! Construct via [`NewTask`] at [`crate::store::Store::create_task`] — the Store
//! mints [`TaskId`] and stamps `created_at` in the same transaction, derives
//! `subscriber` from [`Creator`] (so no caller chooses it), and emits
//! `TaskCreated`. No id without a row; no second way in.
//!
//! Use through [`crate::store::Store`]: `next_pending(now)` picks by
//! `not_before`, `start_task` `Pending→Running`, `complete_task` and
//! `cancel_tasks` reach terminals, `fire_cron` copies a cron Task into a
//! daughter and `Schedule::re_armed` sets when the next one is due.
//! `Schedule::parse` is the single parser for tool, control, and CLI.
//!
//! Consumers — same `TaskState`/`Schedule` handled differently:
//!
//! | State | Store (only writer) | Harness / Worker / Comms | Review / Events |
//! | --- | --- | --- | --- |
//! | `Pending` | enqueues with `not_before` | `next_pending` picks by time | `TaskCreated` |
//! | `Running` | records `session`+`started_at` | worker drives `Turn`s; cancel checked each turn | `TaskStateChanged` |
//! | `Completed` | persists `result`+`at`; terminal | review `Complete`→`Succeeded`, `Unreachable`→`Failed` | `TaskStateChanged`+delivery |
//! | `Cancelled` | terminal, no result | session ends at next decision; waiters resolved | `TaskStateChanged`+`render_cancelled` |
//!
//! | Schedule | `not_before` | `re_armed(now)` | pick |
//! | --- | --- | --- | --- |
//! | `Now` | `None` | `None` | runs immediately |
//! | `At(t)` | `Some(t)` | `None` | runs at `t <= now` |
//! | `Cron{expr,next}` | `Some(next)` | next match after `now` | makes a daughter, never runs |
//!
//! Seam: domain is data — `Store` owns rows and Events, scheduling owns
//! time, `Turn` owns policy. `TaskState`/`Schedule` never decide; callers match them.
//!
//! Rules: **one task concept — human, investigation, and delegated work are the same type.** **`Completed` has `TaskResult`, `Cancelled` has none.** **`Cancelled` is terminal and reaches one Task only — a daughter's end is not its cron Task's.** **a cron Task never runs; it only makes daughters.** **pick is time only; `await_result` is not a queue wait.** **`subscriber` derived from `Creator`, never chosen.** **only review completes; only unreachable fails without it.** **store is the only writer; `Tier`≠`TaskPriority`.**
//!
//! Defines: [`Task`], [`TaskState`]/[`TaskStateName`], [`TaskResult`], [`Schedule`], [`CronExpr`], [`ScheduleError`], [`TaskPriority`], [`Creator`], [`NewTask`], [`TaskSummary`]

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
	/// Stopped before a result. Terminal; no result. Reaches this Task only —
	/// cancelling a cron Task stops further daughters, not the ones running.
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

/// When a task may run, and whether it makes work of its own.
#[derive(
	Debug,
	Clone,
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
	/// Never runs. Each time `next` comes round the Harness copies this Task
	/// into a daughter with `Schedule::Now` and re-arms; this one stays
	/// `Pending` for as long as it stands. Cancel it to stop the daughters.
	Cron { expr: CronExpr, next: Timestamp },
}

/// A cron expression, parsed once.
///
/// Five fields as `crontab` writes them (`0 9 * * *`), or six with leading
/// seconds. Read in local time, so "every morning at nine" means nine where
/// the swarm runs. Serialises as the bare string it was built from.
///
/// Boxed: the parsed form is a few hundred bytes of bit sets, and every Task
/// carries a `Schedule` whether it is a cron or not.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CronExpr(Box<croner::Cron>);

/// Why an asked-for schedule is not one.
///
/// Wording is model-facing — returned as tool output on `create_task_full`.
#[derive(Debug, thiserror::Error)]
pub enum ScheduleError {
	#[error(
		"`in_seconds` and `cron` are two different schedules. Give one, not both."
	)]
	Both,
	#[error("`{expr}` is not a cron expression: {why}")]
	NotACron { expr: String, why: String },
	#[error("`{expr}` never comes round again")]
	NeverFires { expr: String },
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
	///
	/// For a `Cron` this is when the next daughter is due, not when the Task
	/// itself runs — it never does.
	pub fn not_before(&self) -> Option<Timestamp> {
		match self {
			Schedule::Now => None,
			Schedule::At(t) => Some(*t),
			Schedule::Cron { next, .. } => Some(*next),
		}
	}

	/// The same cron schedule armed for the first occurrence after `now`.
	///
	/// `None` once the expression has no future occurrence left, and for a
	/// schedule that is not a cron.
	pub fn re_armed(&self, now: Timestamp) -> Option<Schedule> {
		match self {
			Schedule::Cron { expr, .. } => Some(Schedule::Cron {
				expr: expr.clone(),
				next: expr.next_after(now)?,
			}),
			_ => None,
		}
	}

	/// Build a schedule from the two things a caller may ask for.
	///
	/// Single parser for tool, control, and CLI so the three cannot drift.
	pub fn parse(
		in_seconds: Option<i64>,
		cron: Option<&str>,
		now: Timestamp,
	) -> Result<Schedule, ScheduleError> {
		match (in_seconds, cron) {
			(Some(_), Some(_)) => Err(ScheduleError::Both),
			(_, Some(expr)) => {
				// Parse expression, then arm it
				let expr = CronExpr::try_from(expr.to_string())?;
				let next = expr.next_after(now).ok_or_else(|| {
					ScheduleError::NeverFires { expr: expr.to_string() }
				})?;
				Ok(Schedule::Cron { expr, next })
			},
			(Some(secs), None) => {
				Ok(Schedule::At(now.plus(Duration::from_secs(secs))))
			},
			(None, None) => Ok(Schedule::Now),
		}
	}
}

impl CronExpr {
	/// First instant this expression matches strictly after `now`.
	///
	/// `None` when nothing matches any more — a year-bounded expression that
	/// has run out.
	pub fn next_after(&self, now: Timestamp) -> Option<Timestamp> {
		use chrono::TimeZone;
		let from = chrono::Local.timestamp_millis_opt(now.0).single()?;
		let next = self.0.find_next_occurrence(&from, false).ok()?;
		Some(Timestamp(next.timestamp_millis()))
	}
}

impl TryFrom<String> for CronExpr {
	type Error = ScheduleError;

	fn try_from(expr: String) -> Result<Self, ScheduleError> {
		match expr.parse::<croner::Cron>() {
			Ok(cron) => Ok(CronExpr(Box::new(cron))),
			Err(e) => Err(ScheduleError::NotACron { expr, why: e.to_string() }),
		}
	}
}

impl From<CronExpr> for String {
	fn from(expr: CronExpr) -> String {
		expr.to_string()
	}
}

impl std::fmt::Display for CronExpr {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.0.as_str())
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
