//! The Harness is Sandman itself: the whole of the code we write, within which
//! agents run.
//!
//! It owns every Task, Result, Session, model call, Channel and lesson — through
//! the Store — and agents never manage that state themselves. If an agent seems
//! to need somewhere to keep something, the Harness should keep it instead.
//!
//! **Orchestration is plain code.** Nothing in the swarm decides what runs next.
//! The Harness picks Tasks and creates Sessions mechanically, and that choice
//! lives in one place so it can later become something else.
//!
//! Scheduling here is not round-robin. Each live Session runs its own Turn loop
//! concurrently with the rest, and every model call those loops make waits on the
//! one scheduler. So this loop only *starts* work that is not yet in motion — a
//! Pending Task whose time has come, or a Comms Session with mail — and the
//! Sessions keep turning between starts.
//!
//! What the Harness still owns directly: the Task lifecycle, delivering an answer
//! to whoever subscribed, releasing Sessions blocked in `await_result`, and
//! keeping one respond alive per Channel.
//!
//! Defines: [`Harness`], [`Drive`], [`CancelOutcome`].

use std::sync::Arc;

use crate::domain::{
	ChannelId, Clock, NewTask, SessionId, Spend, TaskId, TaskResult, Timestamp,
};
use crate::event::Events;
use crate::scheduler::Scheduler;
use crate::store::{Store, StoreError};
use crate::tools::ToolRunner;
use crate::waiters::Waiters;

/// How much of the swarm the Harness is allowed to start.
///
/// A first-class parameter rather than something a caller reimplements. A bench
/// case that only wants to know what a Comms Session *decides* runs
/// [`Drive::CommsOnly`]: Tasks it creates sit on the queue and are never
/// executed, so the case costs exactly what the Comms Session costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drive {
	/// Nothing starts by itself. The caller drives every step.
	Manual,
	/// Channels with mail are answered; Pending Tasks are left alone.
	CommsOnly,
	/// Everything: Tasks are picked up, Workers run, the swarm behaves.
	Full,
}

/// What cancelling did, for the tool to put into words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelOutcome {
	NotFound,
	/// Already finished; there was nothing to stop.
	Completed,
	/// Already cancelled.
	Already,
	Cancelled {
		/// The Tasks that stopped, the named one first, its chain after.
		ids: Vec<TaskId>,
		/// At least one of them was running.
		running: bool,
	},
}

/// Sandman.
pub struct Harness {
	pub store: Arc<Store>,
	pub events: Arc<Events>,
	pub scheduler: Arc<Scheduler>,
	pub tools: Arc<dyn ToolRunner>,
	pub clock: Arc<dyn Clock>,
	pub waiters: Arc<Waiters>,

	/// The Comms Session on each open Channel, and its transport.
	comms: std::sync::Mutex<Vec<(ChannelId, Arc<dyn crate::comms::Channel>)>>,
	/// Worker Sessions whose Turn loop is currently running. Ids, not Sessions:
	/// a Session's state is in the Store, and holding the objects here is what
	/// would make the Harness and its Sessions reference each other in a cycle.
	driving: std::sync::Mutex<std::collections::HashSet<SessionId>>,
	/// Channels whose respond loop is currently running. Only one respond is
	/// ever in flight per Channel.
	comms_driving: std::sync::Mutex<std::collections::HashSet<ChannelId>>,
	running: std::sync::atomic::AtomicBool,
}

impl Harness {
	pub fn new(
		_store: Arc<Store>,
		_events: Arc<Events>,
		_scheduler: Arc<Scheduler>,
		_tools: Arc<dyn ToolRunner>,
		_clock: Arc<dyn Clock>,
	) -> Arc<Self> {
		unimplemented!()
	}

	// --- Tasks -------------------------------------------------------------

	/// Put a Task on the queue.
	pub fn create_task(&self, _new: NewTask) -> Result<TaskId, StoreError> {
		unimplemented!()
	}

	/// Record a Task's Result, hand it to whoever asked, release anyone blocked
	/// on it, and re-arm it if it repeats.
	///
	/// A repeating Task is never finished for good: completing it creates the
	/// next occurrence — same Title, Brief, Role and subscriber — anchored to the
	/// schedule rather than to when this one ended, so a late run does not push
	/// the next one back.
	async fn complete_task(
		&self,
		_id: TaskId,
		_result: TaskResult,
	) -> Result<(), StoreError> {
		unimplemented!()
	}

	/// Stop a Task, and the whole chain if it repeats.
	///
	/// Cancelling one occurrence must stop every occurrence that shares the
	/// schedule, or a running one would re-arm the next when it finished.
	///
	/// Nothing touches a running Task's Session directly: its loop reads the
	/// cancelled state at its next decision point and ends without a Result. A
	/// Session blocked in `await_result` is released here, because it would
	/// otherwise never reach that check.
	pub async fn cancel_task(
		&self,
		_id: TaskId,
	) -> Result<CancelOutcome, StoreError> {
		unimplemented!()
	}

	/// Hand a Task's answer to whoever asked for it.
	///
	/// Only a Comms Session subscribes — a Worker waits for a child itself — so
	/// this is the mailbox path alone. Nothing was registered when the
	/// subscription was made and nothing fires early: delivery happens when the
	/// Result exists.
	async fn deliver(&self, _task: TaskId) -> Result<(), StoreError> {
		unimplemented!()
	}

	// --- Channels ----------------------------------------------------------

	/// Open a Channel: a transport, a Comms Session standing on it, and a
	/// transcript.
	pub async fn attach(
		&self,
		_channel: Arc<dyn crate::comms::Channel>,
	) -> Result<ChannelId, StoreError> {
		unimplemented!()
	}

	/// Something arrived on a Channel, from its human or from the swarm.
	pub async fn receive(
		&self,
		_channel: ChannelId,
		_text: &str,
		_from: crate::domain::IncomingFrom,
	) {
		unimplemented!()
	}

	/// The open Channels, for the `message_human` schema, so the model can only
	/// name one that exists.
	pub fn open_channels(
		&self,
	) -> Vec<(ChannelId, crate::domain::ChannelKind)> {
		unimplemented!()
	}

	/// What this Run has cost so far.
	pub fn spend(&self) -> Result<Spend, StoreError> {
		unimplemented!()
	}

	// --- Driving -----------------------------------------------------------

	/// One step of starting.
	///
	/// Only kicks off work that is not yet in motion; the Sessions themselves
	/// keep running between steps. A Channel with mail is answered first — and at
	/// the call level comms is the top tier anyway — then a Pending Task whose
	/// time has come starts a Worker.
	///
	/// Returns whether it started anything, so a caller knows to look again.
	pub async fn step(&self, _drive: Drive) -> Result<bool, StoreError> {
		unimplemented!()
	}

	/// Run until stopped. What an interactive Sandman does.
	pub async fn run(
		self: &Arc<Self>,
		_drive: Drive,
	) -> Result<(), StoreError> {
		unimplemented!()
	}

	/// Run until nothing is left to do. What a one-shot run does.
	///
	/// A Session blocked in `await_result` counts as busy: it is suspended, not
	/// done, and the child it waits on is still work. A Task waiting on its own
	/// time is also still work, so this waits for it rather than returning.
	pub async fn run_until_idle(
		self: &Arc<Self>,
		_drive: Drive,
	) -> Result<(), StoreError> {
		unimplemented!()
	}

	/// A Worker Session's own loop: take a Turn, handle what it earned, repeat
	/// until the Session is done or aborted.
	///
	/// Many of these run at once; the scheduler, not this loop, decides whose
	/// model call is in flight. A Turn that blocks in `await_result` suspends
	/// inside the tool call, so this loop sees it as one long turn.
	async fn drive_worker(
		self: &Arc<Self>,
		_session: SessionId,
		_task: TaskId,
	) {
		unimplemented!()
	}

	/// Drain a Channel's mailbox one respond at a time.
	async fn drive_comms(self: &Arc<Self>, _channel: ChannelId) {
		unimplemented!()
	}

	/// Whether any Session loop is still turning.
	pub fn busy(&self) -> bool {
		unimplemented!()
	}

	/// Stop starting new work. Loops already running finish their turn.
	pub fn stop(&self) {
		unimplemented!()
	}

	/// Stop everything and make sure nothing can still spend: cancel every Task
	/// that has not finished, then wait for the last in-flight model call to land
	/// so its cost reaches the record.
	pub async fn wind_down(
		self: &Arc<Self>,
		_timeout: crate::domain::Duration,
	) {
		unimplemented!()
	}

	/// The context a Session and its tools run against.
	pub fn ctx(
		self: &Arc<Self>,
		_session: SessionId,
	) -> crate::session::SessionCtx {
		unimplemented!()
	}

	pub fn now(&self) -> Timestamp {
		unimplemented!()
	}
}
