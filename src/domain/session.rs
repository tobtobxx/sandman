//! The Session: a live agent context the Harness owns.
//!
//! A Session comes in two shapes, and they differ in what they are attached to
//! rather than in how they run. A Worker Session is created from a Task and ends
//! when that Task does. A Comms Session stands on a Channel, keeps the
//! conversation across messages, and never ends. [`SessionKind`] holds what is
//! true of one shape and not the other, so a Worker has no mailbox to read and a
//! Comms Session has no Task to complete.
//!
//! This file is the record. The loop that drives a Session lives in
//! `session.rs`, `worker.rs` and `comms.rs` — the data is in the Store because
//! its whole life has to be watchable while it happens, and a loop that awaits
//! cannot hold it.
//!
//! Defines: [`Session`], [`SessionKind`], [`SessionStatus`], [`NewSession`],
//! [`Incoming`], [`IncomingFrom`], [`Reflection`], [`ReflectionKind`],
//! [`ReflectionResult`], [`Outcome`], [`Nudge`].

use super::ids::{CallId, ChannelId, RunId, SessionId, TaskId};
use super::message::Message;
use super::time::Timestamp;
use crate::roles::RoleName;

/// One live agent context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
	pub id: SessionId,
	pub run: RunId,
	pub kind: SessionKind,
	pub status: SessionStatus,
	/// The whole conversation, oldest first. Persisted message by message, so a
	/// transcript is a query rather than a rewritten blob.
	pub messages: Vec<Message>,
	/// Every metacognition this Session passed through, oldest first. Kept for
	/// inspection: the Session cannot see what was written about it, and only
	/// the feedback ever reaches the conversation, as a message of its own.
	pub reflections: Vec<Reflection>,
	/// The model calls this Session made, newest last.
	pub calls: Vec<CallId>,
	pub started_at: Timestamp,
	pub ended_at: Option<Timestamp>,
}

/// Which shape of Session this is, and what only that shape has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionKind {
	/// Created from a Task, ends when that Task completes. It sees the Brief and
	/// nothing of the work that led to it.
	Worker { task: TaskId, role: RoleName },
	/// Standing on a Channel, one per Channel. It is never created from a Task,
	/// never reviewed, and never completes.
	Comms {
		channel: ChannelId,
		/// What has arrived and has not been read yet. Post that lands while
		/// the Session is mid-turn waits here until the next one, so nothing
		/// arrives in the middle of its thinking.
		mailbox: Vec<Incoming>,
	},
}

/// What a Session is doing now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
	/// A model call is out.
	Thinking,
	/// Running tool calls.
	Tools,
	/// Under metacognition: a Worker being reviewed on its last reply, or any
	/// Session being interrupted mid-turn.
	Reflecting,
	/// A Worker between Turns.
	Waiting,
	/// A Comms Session between Turns. Workers never reach this.
	Idle,
	/// Done. A Worker finishes when its review submits an answer; kept in the
	/// database for inspection.
	Finished,
	/// Stopped by something that could not be recovered from — in practice, a
	/// model that could not be reached.
	Failed { reason: String },
}

/// Everything needed to start a Session. The Store mints the id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSession {
	pub kind: SessionKind,
	pub status: SessionStatus,
	/// The system prompt and whatever the Session starts knowing — for a Worker,
	/// its Brief.
	pub messages: Vec<Message>,
}

/// One piece of post for a Comms Session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
	pub from: IncomingFrom,
	pub text: String,
	pub at: Timestamp,
}

/// Who sent a piece of post: the human on the Channel, or the swarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncomingFrom {
	Human,
	Swarm,
}

// --- Metacognition ---------------------------------------------------------

/// One metacognition of a Session, kept for inspection.
///
/// It is never part of the Session's context. The Session cannot see what was
/// written about it, and only the feedback it produced ever reaches the
/// conversation, as a message of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reflection {
	pub kind: ReflectionKind,
	/// The model call that produced it, so its full request can be opened. Not
	/// optional: the call is recorded the moment it joins the queue, before
	/// anything is awaited.
	pub call: CallId,
	/// Where in the Session's messages this ran, so a Watcher can put it back in
	/// order.
	pub after_message: usize,
	pub at: Timestamp,
	pub result: ReflectionResult,
}

/// Which metacognition this was.
///
/// A review runs after a Worker's plain-text turn and may write the Task's
/// answer. An interrupt runs mid-turn, on a message count, and never can — the
/// Session it is watching has not offered an answer. That rule is enforced at
/// the seam by [`Nudge`]; the record itself stays one shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReflectionKind {
	Review,
	Interrupt,
}

/// What came of one metacognitive call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectionResult {
	Ran {
		/// The metacognition's own reasoning, when the model exposes it.
		reasoning: Option<String>,
		/// What it wrote, whole: its summary, feedback and lessons sections.
		content: String,
		outcome: Outcome,
	},
	/// The call could not be made. Metacognition fails open, always: broken
	/// metacognition must never be what wedges a run.
	FailedOpen { error: String },
}

/// Which move a metacognition took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
	/// The Task's answer, as the review's `<summary>` wrote it.
	Complete(String),
	/// Correction, injected into the Session's context; it takes another turn.
	Feedback(String),
	/// Nothing actionable. The expected outcome of an interrupt.
	Nothing,
}

/// What an interrupt may come back with.
///
/// A separate type from [`Outcome`], not a subset checked at runtime: an
/// interrupt that cannot return a completion cannot be asked to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nudge {
	Feedback(String),
	Nothing,
}

impl From<Nudge> for Outcome {
	fn from(_n: Nudge) -> Outcome {
		unimplemented!()
	}
}

impl SessionKind {
	/// The Task this Session holds, if it is a Worker.
	pub fn task(&self) -> Option<TaskId> {
		unimplemented!()
	}

	/// The Channel this Session stands on, if it is a Comms Session.
	pub fn channel(&self) -> Option<ChannelId> {
		unimplemented!()
	}

	/// The Role of the Task this Session holds, if it is a Worker.
	pub fn role(&self) -> Option<RoleName> {
		unimplemented!()
	}

	pub fn discriminant(&self) -> &'static str {
		unimplemented!()
	}
}

impl SessionStatus {
	pub fn discriminant(&self) -> &'static str {
		unimplemented!()
	}

	pub fn is_terminal(&self) -> bool {
		unimplemented!()
	}
}
