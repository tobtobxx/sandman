//! The Session: live agent context as data, not loop.
//!
//! Two shapes, one record: [`SessionKind::Worker`] holds Task+Role and ends
//! with it; [`SessionKind::Comms`] stands on a Channel with a mailbox and
//! never ends. [`SessionStatus`] tracks where the loop is; [`Reflection`]
//! keeps metacognition for inspection, never in context.
//!
//! Construct: `Store` mints `SessionId` from [`NewSession`] via
//! `db::counters::take` inside the inserting transaction; transcript grows via
//! `Store::append_message` (one row per [`Message`] at `(owner, idx)`); `calls`
//! and `reflections` appended by `session::turn` and `reflect` without holding
//! the Session.
//! Use: `session::turn(ctx: &SessionCtx, tier: Tier) -> Turn` loops until
//! `Reply::Text` or cancellation; `session::tell` injects mail/answers/feedback
//! as `Message::User`; [`SessionCtx`] (Store, Events, Scheduler, ToolRunner,
//! Clock, Harness as `Arc`) threads through all layers.
//!
//! Consumers and how they match the same types differently:
//!
//! | Type | `Store` (only writer) | `session::turn` (loop) | `worker.rs` / `comms.rs` (policy) | `reflect` |
//! | --- | --- | --- | --- | --- |
//! | `SessionKind` | persists `Worker{task,role}` vs `Comms{channel,mailbox}` | `Worker` → `Purpose::Work(role)` vs `Comms` → `Purpose::Comms`; tool schemas differ | never cross-reference | — |
//! | `SessionStatus` | persists `Waiting/Thinking/Tools/Reflecting` → `Finished/Failed/Cancelled` | sets `Thinking`/`Tools`/`Reflecting` per phase | `Worker: Waiting` between Turns vs `Comms: Idle`; `Cancelled` ends with no `TaskResult` | `Reflecting` while judging, recorded on judged Session |
//! | `Turn` | — | reports `Text/Silent/Unreachable/Cancelled` | `Worker` reviews `Text`/`Silent`, fails on `Unreachable`; `Comms` says `Text` to human | — |
//! | `ReflectionKind`/`Outcome`/`Nudge` | persists `Reflection` | fires Interrupt when `msgs - last >= interval` | `Review` may `Complete` a Task; `Interrupt` only `Feedback`/`Nothing` via `Nudge→Outcome` | `Outcome::Complete` vs `Nudge` asymmetry enforced by type |
//!
//! Rules: **data only — loop lives in `session.rs`, policy in `worker.rs`/`comms.rs`.**
//! **a Turn decides nothing — it reports, caller decides.** **Worker ends with its Task; Comms never ends.**
//! **one `Comms` per `Channel`; `Worker` has no mailbox, `Comms` has no `Task`.**
//! **transcript is a query per row, not a blob.** **reflections never enter context; only `Feedback` reaches it via `tell`.**
//! **metacognition fails open — `FailedOpen` never wedges a run.**
//!
//! Defines: [`Session`], [`SessionKind`], [`SessionStatus`], [`NewSession`],
//! [`Incoming`], [`IncomingFrom`], [`Reflection`], [`ReflectionKind`],
//! [`ReflectionResult`], [`Outcome`], [`Nudge`].

use super::ids::{CallId, ChannelId, RunId, SessionId, TaskId};
use super::message::Message;
use super::time::Timestamp;
use crate::roles::RoleName;

/// One live agent context. Holds transcript, reflections and call history.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Session {
	pub id: SessionId,
	pub run: RunId,
	pub kind: SessionKind,
	pub status: SessionStatus,
	/// Transcript, oldest first. One row per entry via `Store::append_message`.
	pub messages: Vec<Message>,
	/// Metacognitions in order, for inspection. Never in context; only `Feedback` reaches it via `tell`.
	pub reflections: Vec<Reflection>,
	/// Model calls made, newest last.
	pub calls: Vec<CallId>,
	pub started_at: Timestamp,
	pub ended_at: Option<Timestamp>,
}

/// Which shape this Session is. Carries what only that shape has.
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
pub enum SessionKind {
	/// Created from a Task; ends with it. Sees only its Brief.
	Worker { task: TaskId, role: RoleName },
	/// Standing on a Channel, one per Channel. Never from a Task, never reviewed, never ends.
	Comms {
		channel: ChannelId,
		/// Unread post. Drains at next Turn, never mid-thinking.
		mailbox: Vec<Incoming>,
	},
}

/// What a Session is doing now.
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
pub enum SessionStatus {
	/// A model call is out.
	Thinking,
	/// Running tool calls.
	Tools,
	/// Under metacognition. Review after Worker's text or Interrupt mid-turn.
	Reflecting,
	/// Worker between Turns.
	Waiting,
	/// Comms between Turns. Workers never idle.
	Idle,
	/// Done. Worker finished when review submitted answer.
	Finished,
	/// Stopped unrecoverably. In practice, model unreachable.
	Failed { reason: String },
	/// Abandoned by dead Run. Next `Store::open` ends still-open Sessions here.
	Cancelled,
}

/// Inputs to start a Session. `Store` mints `SessionId`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NewSession {
	pub kind: SessionKind,
	pub status: SessionStatus,
	/// Initial transcript: system prompt plus Brief for Workers.
	pub messages: Vec<Message>,
}

/// One piece of post for a Comms Session.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Incoming {
	pub from: IncomingFrom,
	pub text: String,
	pub at: Timestamp,
}

/// Who sent post to a Comms Session.
#[derive(
	Debug,
	Clone,
	Copy,
	PartialEq,
	Eq,
	serde::Serialize,
	serde::Deserialize,
	strum::Display,
	strum::EnumString,
	strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum IncomingFrom {
	Human,
	Swarm,
}

// --- Metacognition ---------------------------------------------------------

/// One metacognition kept for inspection. Never in Session context.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Reflection {
	pub kind: ReflectionKind,
	/// Model call that produced it. Recorded at queue time, before await.
	pub call: CallId,
	/// Message index it ran after. Lets watchers order it.
	pub after_message: usize,
	pub at: Timestamp,
	pub result: ReflectionResult,
}

/// Which metacognition. `Review` may complete the Task; `Interrupt` never may.
#[derive(
	Debug,
	Clone,
	Copy,
	PartialEq,
	Eq,
	serde::Serialize,
	serde::Deserialize,
	strum::Display,
	strum::EnumString,
	strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ReflectionKind {
	Review,
	Interrupt,
}

/// What came of one metacognitive call.
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
pub enum ReflectionResult {
	Ran {
		/// Metacognition reasoning, when exposed.
		reasoning: Option<String>,
		/// Full output: summary, feedback and lessons sections.
		content: String,
		outcome: Outcome,
	},
	/// Call could not be made. Fails open — never wedges a run.
	FailedOpen { error: String },
}

/// Which move a metacognition took.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
	/// Task answer from review's `<summary>`.
	Complete(String),
	/// Correction injected via `tell`. Takes another Turn.
	Feedback(String),
	/// Nothing actionable. Expected from interrupts.
	Nothing,
}

/// What an interrupt may return. Separate from `Outcome` so `Complete` is unaskable.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Nudge {
	Feedback(String),
	Nothing,
}

impl From<Nudge> for Outcome {
	fn from(n: Nudge) -> Outcome {
		match n {
			Nudge::Feedback(text) => Outcome::Feedback(text),
			Nudge::Nothing => Outcome::Nothing,
		}
	}
}

impl SessionKind {
	/// Task this Session holds, if Worker.
	pub fn task(&self) -> Option<TaskId> {
		match self {
			SessionKind::Worker { task, .. } => Some(*task),
			SessionKind::Comms { .. } => None,
		}
	}

	/// Channel this Session stands on, if Comms.
	pub fn channel(&self) -> Option<ChannelId> {
		match self {
			SessionKind::Comms { channel, .. } => Some(*channel),
			SessionKind::Worker { .. } => None,
		}
	}

	/// Role of this Worker's Task, if Worker.
	pub fn role(&self) -> Option<RoleName> {
		match self {
			SessionKind::Worker { role, .. } => Some(*role),
			SessionKind::Comms { .. } => None,
		}
	}
}

impl SessionStatus {
	/// Whether this status is terminal. No further transitions.
	pub fn is_terminal(&self) -> bool {
		matches!(
			self,
			SessionStatus::Finished
				| SessionStatus::Failed { .. }
				| SessionStatus::Cancelled
		)
	}
}
