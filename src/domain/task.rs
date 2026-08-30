//! The Task: the single unit of work.
//!
//! There is exactly one Task concept. A request from a human, an investigation,
//! and a piece of work handed between agents are all Tasks, and working on one
//! may produce more.
//!
//! Everything optional about a Task that depends on where it is in its life has
//! been folded into [`TaskState`], so the impossible combinations cannot be
//! built: a completed Task always has a Result, a pending one never does, a
//! running one always names the Session holding it, and a cancelled one has no
//! Result at all. [`Schedule`] does the same for timing — a repeating Task
//! cannot exist without the anchor its repetition counts from.
//!
//! Defines: [`Task`], [`TaskState`], [`TaskResult`], [`Schedule`],
//! [`TaskPriority`], [`Creator`], [`NewTask`], [`TaskSummary`].

use super::ids::{ChannelId, RunId, SessionId, TaskId};
use super::text::{Brief, Title};
use super::time::{Duration, Timestamp};
use crate::roles::RoleName;

/// One piece of work.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Task {
	pub id: TaskId,
	/// The Run this Task belongs to. Spend is scoped to a Run; the Lessons and
	/// past Tasks are searched across all of them.
	pub run: RunId,
	pub title: Title,
	/// The only thing the Worker gets. It must stand alone.
	pub brief: Brief,
	pub role: RoleName,
	pub state: TaskState,
	pub schedule: Schedule,
	/// The Channel to hand this Task's Result to, if anyone asked for it.
	///
	/// Only a Comms Session ever subscribes — a Worker waits for a child by
	/// calling `await_result`, which blocks inside the tool call rather than
	/// registering anything. Naming the Channel rather than the Session makes
	/// that invariant structural. A Task without a subscriber is work nobody is
	/// waiting on: its Result is recorded and nothing further happens.
	pub subscriber: Option<ChannelId>,
	/// How urgently the swarm should spend a model call on this Task's Worker.
	pub priority: TaskPriority,
	pub created_by: Creator,
	pub created_at: Timestamp,
}

/// Where a Task is in its life, and everything that depends on being there.
///
/// [`TaskStateName`] is the same set with the payloads taken off: the one word a
/// state goes into the database and onto the wire under, and the only thing a
/// filter can name. It is derived from this enum, so a state cannot exist that a
/// filter cannot ask for.
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
	/// Waiting on the queue. Being picked has exactly one condition: time.
	Pending,
	/// Held by a Session. Naming that Session is what lets a cancellation reach
	/// a Worker blocked in `await_result` without searching for its holder.
	Running { session: SessionId, started_at: Timestamp },
	/// Done, with a Result — whether the work succeeded or failed.
	Completed { result: TaskResult, at: Timestamp },
	/// Stopped before it produced a Result. Terminal, and the only state with
	/// no Result at all: a pending Task never runs, a running one ends at its
	/// Session's next decision point, and a repeating one stops as a chain.
	Cancelled { at: Timestamp },
}

/// What a Session produced for its Task.
///
/// A failure is a Result saying so, not the absence of one. The Harness writes
/// [`TaskResult::Failed`] when the model could not be reached; every other
/// Result is chosen by the metacognitive review from what the Worker wrote.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskResult {
	Succeeded(String),
	Failed(String),
}

/// When a Task may run, and whether finishing it arms another.
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
	/// A chain of ordinary Tasks: completing one creates the next, anchored to
	/// the schedule rather than to when the last one ended, so a late run does
	/// not push the next one back.
	Repeating { first: Timestamp, every: Duration },
}

/// How urgently the swarm should spend a model call on this Task's Worker.
///
/// Distinct from [`crate::scheduler::Tier`], which is where a call waits. This
/// is the property of the work; the Tier is the position in the queue.
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

/// Who put this Task on the queue.
#[derive(
	Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Creator {
	/// A Session, through one of the create-task tools.
	Session(SessionId),
	/// A one-shot run started from the command line.
	Cli,
	/// Another process, through the control socket.
	Control,
}

/// Everything needed to put a Task on the queue. The Store mints the id.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NewTask {
	pub title: Title,
	pub brief: Brief,
	pub role: RoleName,
	pub schedule: Schedule,
	pub subscriber: Option<ChannelId>,
	pub priority: TaskPriority,
	pub created_by: Creator,
}

/// A Task as the control socket and `list_tasks` report it: enough to recognise
/// one and to cancel it, without its whole Brief and Result.
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
	/// Whether nothing further will happen to a Task in this state.
	pub fn is_terminal(&self) -> bool {
		matches!(
			self,
			TaskState::Completed { .. } | TaskState::Cancelled { .. }
		)
	}
}

impl TaskResult {
	/// The text itself, whichever way it went.
	pub fn content(&self) -> &str {
		match self {
			TaskResult::Succeeded(text) => text,
			TaskResult::Failed(text) => text,
		}
	}
}

impl Schedule {
	/// The earliest instant a Task on this schedule may run.
	pub fn not_before(&self, _created_at: Timestamp) -> Option<Timestamp> {
		match self {
			Schedule::Now => None,
			Schedule::At(t) => Some(*t),
			Schedule::Repeating { first, .. } => Some(*first),
		}
	}

	/// The schedule the next occurrence takes, if this one repeats. Anchored to
	/// the schedule, not to when the finishing run happened to end.
	pub fn next_occurrence(&self) -> Option<Schedule> {
		match self {
			Schedule::Repeating { first, every } => Some(Schedule::Repeating {
				first: first.plus(*every),
				every: *every,
			}),
			_ => None,
		}
	}

	/// Build from a tool call's, a control request's or the command line's pair
	/// of seconds-from-now offsets: a delay before the first run, and how often
	/// it repeats after that. One parser, so the three cannot drift apart.
	pub fn from_offsets(
		run_at_seconds: Option<i64>,
		repeat_seconds: Option<i64>,
		now: Timestamp,
	) -> Schedule {
		let first = now.plus(Duration::from_secs(run_at_seconds.unwrap_or(0)));
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
	/// This Task's Result as the text that crosses between agents: an answer
	/// with nothing but the Task it answers and what was found.
	pub fn render_answer(&self) -> String {
		let content = match &self.state {
			TaskState::Completed { result, .. } => result.content(),
			_ => "",
		};
		format!("Answer to \"{}\":\n{}", self.title, content)
	}

	/// The notice a cancellation sends where a Result would have gone, so
	/// whoever waited on this Task does not hang on it.
	pub fn render_cancelled(&self) -> String {
		format!("Task \"{}\" was cancelled.", self.title)
	}
}
