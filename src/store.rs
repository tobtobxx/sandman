//! Everything the Harness owns, behind one vocabulary.
//!
//! The Store is the only thing that touches the database, and the only thing
//! that emits state [`Event`]s. Its interface is domain-shaped — `start_task`,
//! `complete_task`, `append_message` — rather than field-shaped, so a caller
//! says what happened and the Store decides what that means in rows and in the
//! trace.
//!
//! Two properties hold structurally rather than by discipline:
//!
//! **A change without an Event cannot be written.** The connection is private
//! and there is no method that mutates without emitting. In the prototype the
//! Watcher was kept in step by comparing JSON against a shadow twice a second,
//! because announcing at the site of a change fails silently the first time
//! someone forgets. Here there is no site to forget at.
//!
//! **A lock is never held across an await.** Every method takes `&self`, does
//! its work, and returns; nothing hands a guard to a caller. The Mutex is
//! `std::sync::Mutex` deliberately — if a future ever needed to be awaited
//! inside one of these methods, it would not compile, which is the warning we
//! want.
//!
//! Reads return owned values. A model call already carries a detached copy of
//! the messages it was built from, so this is what the system did anyway.
//!
//! Defines: [`Store`], [`StoreError`], [`Snapshot`], [`ListFilter`].

use std::sync::Arc;

use crate::db::{Backing, DbError};
use crate::domain::{
	CallId, CallStatus, ChannelId, ChannelKind, ChannelRecord, Incoming,
	Lesson, LessonId, LlmCall, Message, NewCall, NewLesson, NewSession,
	NewTask, Reflection, Run, RunId, Session, SessionId, SessionStatus, Spend,
	Task, TaskId, TaskResult, TaskState, TaskSummary, Timestamp, Utterance,
};
use crate::event::Events;

/// All of Sandman's state.
pub struct Store {
	conn: std::sync::Mutex<rusqlite::Connection>,
	events: Arc<Events>,
	run: RunId,
}

/// What can go wrong asking the Store for something.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
	#[error(transparent)]
	Db(#[from] DbError),
	#[error("there is no {what} {id}")]
	NoSuch { what: &'static str, id: String },
	/// The belt behind every caller's own check. A cancelled Task must never
	/// complete, and no Task may complete twice.
	#[error("task {task} is {state}, not running; it will not be completed")]
	NotRunning { task: TaskId, state: &'static str },
}

/// Every entity, for a Watcher's first frame. After this a Watcher follows the
/// Event stream and never asks again.
#[derive(Debug, Clone)]
pub struct Snapshot {
	pub run: Run,
	pub tasks: Vec<Task>,
	pub sessions: Vec<Session>,
	pub calls: Vec<LlmCall>,
	pub channels: Vec<ChannelRecord>,
	pub lessons: Vec<Lesson>,
}

/// What `list_tasks` narrows the queue to.
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
	pub state: Option<&'static str>,
	pub recurring: bool,
	pub count: Option<usize>,
}

impl Store {
	/// Open a Store, migrate the database, and start a new Run.
	///
	/// Every Task, Session, call and lesson written afterwards belongs to that
	/// Run. Spend is scoped to it; the Lessons and past Tasks are not.
	pub fn open(
		_backing: Backing,
		_events: Arc<Events>,
		_model: &str,
		_now: Timestamp,
	) -> Result<Self, StoreError> {
		unimplemented!()
	}

	/// The Run this Store is writing.
	pub fn run(&self) -> RunId {
		unimplemented!()
	}

	/// Mark the Run finished. A Run whose process was killed simply never gets
	/// this.
	pub fn end_run(&self, _now: Timestamp) -> Result<(), StoreError> {
		unimplemented!()
	}

	// --- Tasks -------------------------------------------------------------

	/// Put a Task on the queue. Emits [`crate::event::Event::TaskCreated`].
	pub fn create_task(
		&self,
		_new: NewTask,
		_now: Timestamp,
	) -> Result<TaskId, StoreError> {
		unimplemented!()
	}

	/// Hand a Pending Task to the Session that will do it.
	pub fn start_task(
		&self,
		_id: TaskId,
		_session: SessionId,
		_now: Timestamp,
	) -> Result<(), StoreError> {
		unimplemented!()
	}

	/// Record a Task's Result. Fails if the Task is not Running — which is what
	/// stops a cancelled Task completing and a repeating chain re-arming after
	/// it was stopped.
	pub fn complete_task(
		&self,
		_id: TaskId,
		_result: TaskResult,
		_now: Timestamp,
	) -> Result<(), StoreError> {
		unimplemented!()
	}

	/// Stop Tasks. Cancelling is terminal: a pending Task never runs, a running
	/// one ends at its Session's next decision point with no Result, and a
	/// repeating one stops as a chain.
	pub fn cancel_tasks(
		&self,
		_ids: &[TaskId],
		_now: Timestamp,
	) -> Result<(), StoreError> {
		unimplemented!()
	}

	pub fn task(&self, _id: TaskId) -> Result<Option<Task>, StoreError> {
		unimplemented!()
	}

	/// Just the state, for the cancellation check at the top of a turn — the
	/// hottest read in the system.
	pub fn task_state(
		&self,
		_id: TaskId,
	) -> Result<Option<TaskState>, StoreError> {
		unimplemented!()
	}

	/// The first Pending Task whose time has come.
	///
	/// Time is the one condition on being picked. Waiting on other work is not
	/// here: a Session holds for another Session, through `await_result`, after
	/// a Task has already started.
	pub fn next_pending(
		&self,
		_now: Timestamp,
	) -> Result<Option<Task>, StoreError> {
		unimplemented!()
	}

	/// How long until the earliest scheduled Task can run, if one is waiting.
	pub fn next_due_in(
		&self,
		_now: Timestamp,
	) -> Result<Option<crate::domain::Duration>, StoreError> {
		unimplemented!()
	}

	/// Every occurrence of one repeating chain that has not finished — so
	/// cancelling one occurrence can stop the chain.
	pub fn chain_of(&self, _id: TaskId) -> Result<Vec<TaskId>, StoreError> {
		unimplemented!()
	}

	pub fn list_tasks(
		&self,
		_filter: ListFilter,
	) -> Result<Vec<TaskSummary>, StoreError> {
		unimplemented!()
	}

	/// Every Task in this Run, newest last. What a one-shot run prints.
	pub fn tasks_of_run(&self, _run: RunId) -> Result<Vec<Task>, StoreError> {
		unimplemented!()
	}

	/// Every Task ever, for a search by meaning. Not scoped to a Run: what the
	/// swarm already asked and answered is worth finding whenever it happened.
	pub fn all_tasks(&self) -> Result<Vec<Task>, StoreError> {
		unimplemented!()
	}

	// --- Sessions ----------------------------------------------------------

	pub fn start_session(
		&self,
		_new: NewSession,
		_now: Timestamp,
	) -> Result<SessionId, StoreError> {
		unimplemented!()
	}

	pub fn set_status(
		&self,
		_id: SessionId,
		_status: SessionStatus,
	) -> Result<(), StoreError> {
		unimplemented!()
	}

	pub fn end_session(
		&self,
		_id: SessionId,
		_status: SessionStatus,
		_now: Timestamp,
	) -> Result<(), StoreError> {
		unimplemented!()
	}

	/// Append one message and return its index in the conversation.
	pub fn append_message(
		&self,
		_id: SessionId,
		_message: Message,
	) -> Result<usize, StoreError> {
		unimplemented!()
	}

	/// The whole conversation, as a model call needs it.
	pub fn messages(&self, _id: SessionId) -> Result<Vec<Message>, StoreError> {
		unimplemented!()
	}

	/// How many messages this Session has. What the interrupt counts.
	pub fn message_count(&self, _id: SessionId) -> Result<usize, StoreError> {
		unimplemented!()
	}

	pub fn record_reflection(
		&self,
		_id: SessionId,
		_r: Reflection,
	) -> Result<(), StoreError> {
		unimplemented!()
	}

	/// The most recent metacognition, whichever kind — what the interrupt counts
	/// from.
	pub fn last_reflection(
		&self,
		_id: SessionId,
	) -> Result<Option<Reflection>, StoreError> {
		unimplemented!()
	}

	pub fn session(
		&self,
		_id: SessionId,
	) -> Result<Option<Session>, StoreError> {
		unimplemented!()
	}

	/// Put something in a Comms Session's mailbox.
	pub fn receive_mail(
		&self,
		_id: SessionId,
		_incoming: Incoming,
	) -> Result<(), StoreError> {
		unimplemented!()
	}

	/// Take everything unread from a mailbox, marking it read in the same
	/// transaction. Post that lands mid-turn waits for the next one.
	pub fn take_mail(
		&self,
		_id: SessionId,
	) -> Result<Vec<Incoming>, StoreError> {
		unimplemented!()
	}

	pub fn has_mail(&self, _id: SessionId) -> Result<bool, StoreError> {
		unimplemented!()
	}

	// --- Model calls -------------------------------------------------------

	/// Record a call as it joins the scheduler's queue, before it is sent, so
	/// waiting is as visible as working.
	pub fn queue_call(
		&self,
		_new: NewCall,
		_now: Timestamp,
	) -> Result<CallId, StoreError> {
		unimplemented!()
	}

	pub fn set_call_status(
		&self,
		_id: CallId,
		_status: CallStatus,
	) -> Result<(), StoreError> {
		unimplemented!()
	}

	pub fn call(&self, _id: CallId) -> Result<Option<LlmCall>, StoreError> {
		unimplemented!()
	}

	/// What this Run has cost. Summed from the calls that finished, never
	/// accumulated, so it cannot drift from them.
	pub fn spend(&self, _run: RunId) -> Result<Spend, StoreError> {
		unimplemented!()
	}

	/// Whether any call is still queued or in flight — what a wind-down waits
	/// for, so the last call's cost reaches the record.
	pub fn calls_outstanding(&self) -> Result<bool, StoreError> {
		unimplemented!()
	}

	// --- Channels ----------------------------------------------------------

	pub fn open_channel(
		&self,
		_kind: ChannelKind,
		_session: SessionId,
	) -> Result<ChannelId, StoreError> {
		unimplemented!()
	}

	pub fn say(
		&self,
		_channel: ChannelId,
		_utterance: Utterance,
	) -> Result<(), StoreError> {
		unimplemented!()
	}

	pub fn transcript(
		&self,
		_channel: ChannelId,
	) -> Result<Vec<Utterance>, StoreError> {
		unimplemented!()
	}

	pub fn channels(&self) -> Result<Vec<ChannelRecord>, StoreError> {
		unimplemented!()
	}

	/// The Comms Session standing on a Channel.
	pub fn channel_session(
		&self,
		_channel: ChannelId,
	) -> Result<Option<SessionId>, StoreError> {
		unimplemented!()
	}

	// --- Lessons -----------------------------------------------------------

	pub fn keep_lesson(
		&self,
		_new: NewLesson,
		_now: Timestamp,
	) -> Result<LessonId, StoreError> {
		unimplemented!()
	}

	/// Every lesson ever written, across every Run.
	pub fn all_lessons(&self) -> Result<Vec<Lesson>, StoreError> {
		unimplemented!()
	}

	/// A cached embedding, and where to put one. Kept out of the entity tables:
	/// a vector is several hundred floats no human reads.
	pub fn vector(
		&self,
		_key: &str,
		_model: &str,
	) -> Result<Option<Vec<f32>>, StoreError> {
		unimplemented!()
	}

	pub fn put_vector(
		&self,
		_key: &str,
		_model: &str,
		_v: &[f32],
	) -> Result<(), StoreError> {
		unimplemented!()
	}

	// --- Watching ----------------------------------------------------------

	/// Everything, for a Watcher's first frame.
	pub fn snapshot(&self) -> Result<Snapshot, StoreError> {
		unimplemented!()
	}

	/// Copy the whole database to a file while it is in use. How a bench case
	/// keeps its artifact.
	pub fn save_copy(&self, _to: &std::path::Path) -> Result<(), StoreError> {
		unimplemented!()
	}
}
