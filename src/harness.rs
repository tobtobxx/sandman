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

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use crate::domain::{
	ChannelId, ChannelKind, Clock, Incoming, IncomingFrom, Message, NewTask,
	SessionId, Spend, TaskId, TaskResult, Timestamp, Utterance, Who,
};
use crate::event::{Event, Events};
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
	comms: Mutex<Vec<(ChannelId, Arc<dyn crate::comms::Channel>)>>,
	/// Worker Sessions whose Turn loop is currently running. Ids, not Sessions:
	/// a Session's state is in the Store, and holding the objects here is what
	/// would make the Harness and its Sessions reference each other in a cycle.
	driving: Mutex<HashSet<SessionId>>,
	/// Channels whose respond loop is currently running. Only one respond is
	/// ever in flight per Channel.
	comms_driving: Mutex<HashSet<ChannelId>>,
	running: std::sync::atomic::AtomicBool,
	/// Woken by [`Harness::stop`], so [`Harness::run`]'s wait does not sit out
	/// its sleep or wait for an unrelated Event once nothing is left to do.
	woken: tokio::sync::Notify,
}

impl Harness {
	pub fn new(
		store: Arc<Store>,
		events: Arc<Events>,
		scheduler: Arc<Scheduler>,
		tools: Arc<dyn ToolRunner>,
		clock: Arc<dyn Clock>,
	) -> Arc<Self> {
		Arc::new(Harness {
			store,
			events,
			scheduler,
			tools,
			clock,
			waiters: Arc::new(Waiters::new()),
			comms: Mutex::new(Vec::new()),
			driving: Mutex::new(HashSet::new()),
			comms_driving: Mutex::new(HashSet::new()),
			running: std::sync::atomic::AtomicBool::new(true),
			woken: tokio::sync::Notify::new(),
		})
	}

	// --- Tasks -------------------------------------------------------------

	/// Put a Task on the queue.
	pub fn create_task(&self, new: NewTask) -> Result<TaskId, StoreError> {
		let now = self.clock.now();
		self.store.create_task(new, now)
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
		id: TaskId,
		result: TaskResult,
	) -> Result<(), StoreError> {
		let now = self.clock.now();
		self.store.complete_task(id, result, now)?;

		let task = self
			.store
			.task(id)?
			.ok_or(StoreError::NoSuch { what: "task", id: id.to_string() })?;
		let answer = task.render_answer();

		self.deliver(id).await?;
		self.waiters.resolve(id, &answer);

		if let Some(schedule) = task.schedule.next_occurrence() {
			let next = NewTask {
				title: task.title,
				brief: task.brief,
				role: task.role,
				schedule,
				subscriber: task.subscriber,
				priority: task.priority,
				created_by: task.created_by,
			};
			self.store.create_task(next, now)?;
		}
		Ok(())
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
		id: TaskId,
	) -> Result<CancelOutcome, StoreError> {
		let Some(task) = self.store.task(id)? else {
			return Ok(CancelOutcome::NotFound);
		};
		let running_session = match task.state {
			crate::domain::TaskState::Completed { .. } => {
				return Ok(CancelOutcome::Completed);
			},
			crate::domain::TaskState::Cancelled { .. } => {
				return Ok(CancelOutcome::Already);
			},
			crate::domain::TaskState::Running { session, .. } => Some(session),
			crate::domain::TaskState::Pending => None,
		};
		let running = running_session.is_some();

		let mut chain = self.store.chain_of(id)?;
		chain.retain(|&t| t != id);
		chain.insert(0, id);

		let now = self.clock.now();
		self.store.cancel_tasks(&chain, now)?;

		let notice = task.render_cancelled();
		for &chained in &chain {
			self.waiters.resolve(chained, &notice);
		}
		if let Some(session) = running_session {
			self.waiters.resolve_held_by(session, &notice);
		}

		Ok(CancelOutcome::Cancelled { ids: chain, running })
	}

	/// Hand a Task's answer to whoever asked for it.
	///
	/// Only a Comms Session subscribes — a Worker waits for a child itself — so
	/// this is the mailbox path alone. Nothing was registered when the
	/// subscription was made and nothing fires early: delivery happens when the
	/// Result exists.
	async fn deliver(&self, task: TaskId) -> Result<(), StoreError> {
		let Some(task) = self.store.task(task)? else {
			return Ok(());
		};
		if let Some(channel) = task.subscriber {
			self.receive(channel, &task.render_answer(), IncomingFrom::Swarm)
				.await;
		}
		Ok(())
	}

	// --- Channels ----------------------------------------------------------

	/// Open a Channel: a transport, a Comms Session standing on it, and a
	/// transcript.
	pub async fn attach(
		&self,
		channel: Arc<dyn crate::comms::Channel>,
	) -> Result<ChannelId, StoreError> {
		let now = self.clock.now();
		let messages = vec![Message::System {
			content: crate::prompts::COMMS_SESSION.to_string(),
		}];
		let (_session, id) =
			self.store.open_comms(channel.kind(), messages, now)?;
		self.comms.lock().unwrap().push((id, channel));
		Ok(id)
	}

	/// Something arrived on a Channel, from its human or from the swarm.
	pub async fn receive(
		&self,
		channel: ChannelId,
		text: &str,
		from: IncomingFrom,
	) {
		let Ok(Some(session)) = self.store.channel_session(channel) else {
			return;
		};
		let now = self.clock.now();
		if from == IncomingFrom::Human {
			let _ = self.store.say(
				channel,
				Utterance { who: Who::Human, text: text.to_string(), at: now },
			);
		}
		let _ = self.store.receive_mail(
			session,
			Incoming { from, text: text.to_string(), at: now },
		);
	}

	/// The open Channels, for the `message_human` schema, so the model can only
	/// name one that exists.
	pub fn open_channels(&self) -> Vec<(ChannelId, ChannelKind)> {
		self.comms
			.lock()
			.unwrap()
			.iter()
			.map(|(id, channel)| (*id, channel.kind()))
			.collect()
	}

	/// What this Run has cost so far.
	pub fn spend(&self) -> Result<Spend, StoreError> {
		self.store.spend(self.store.run())
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
	pub async fn step(
		self: &Arc<Self>,
		drive: Drive,
	) -> Result<bool, StoreError> {
		if drive == Drive::Manual {
			return Ok(false);
		}

		let channels: Vec<ChannelId> = self
			.comms
			.lock()
			.unwrap()
			.iter()
			.map(|(id, _)| *id)
			.collect();
		for channel in channels {
			let Ok(Some(session)) = self.store.channel_session(channel) else {
				continue;
			};
			if !matches!(self.store.has_mail(session), Ok(true)) {
				continue;
			}
			if !self.comms_driving.lock().unwrap().insert(channel) {
				continue;
			}
			let this = self.clone();
			tokio::spawn(async move { this.drive_comms(channel).await });
			return Ok(true);
		}

		if drive == Drive::CommsOnly {
			return Ok(false);
		}

		let now = self.clock.now();
		let Some(task) = self.store.next_pending(now)? else {
			return Ok(false);
		};

		// `new_worker_session` never reads `ctx.id`; the real id does not
		// exist until the Session row it creates is written.
		let ctx = self.ctx(SessionId(0));
		let session = crate::worker::new_worker_session(&ctx, &task).await?;
		self.store.start_task(task.id, session, now)?;
		self.driving.lock().unwrap().insert(session);

		let this = self.clone();
		let task_id = task.id;
		tokio::spawn(async move { this.drive_worker(session, task_id).await });
		Ok(true)
	}

	/// Run until stopped. What an interactive Sandman does.
	pub async fn run(self: &Arc<Self>, drive: Drive) -> Result<(), StoreError> {
		let mut events = self.events.subscribe();
		while self.running.load(Ordering::SeqCst) {
			while self.step(drive).await? {
				if !self.running.load(Ordering::SeqCst) {
					return Ok(());
				}
			}

			let now = self.clock.now();
			let wait = self.store.next_due_in(now)?;
			let sleep = tokio::time::sleep(match wait {
				Some(d) => std::time::Duration::from_millis(d.0.max(0) as u64),
				None => std::time::Duration::from_secs(3600),
			});
			tokio::pin!(sleep);
			tokio::select! {
				_ = &mut sleep => {},
				event = events.recv() => self.forward_said(event),
				_ = self.woken.notified() => {},
			}
			// Drain whatever else arrived without waiting again, so a burst
			// of Events does not cost a re-check each.
			while let Ok(event) = events.try_recv() {
				self.forward_said(Ok(event));
			}
		}
		Ok(())
	}

	/// If an Event says something to a human, hand it to that Channel's
	/// transport. The Comms Session that wrote it does not know which
	/// transport it sits on — see `comms.rs` — so this is where the two meet.
	fn forward_said(
		&self,
		event: Result<Event, tokio::sync::broadcast::error::RecvError>,
	) {
		if let Ok(Event::Said { channel, utterance }) = event {
			if utterance.who == Who::Sandman {
				if let Some((_, transport)) = self
					.comms
					.lock()
					.unwrap()
					.iter()
					.find(|(id, _)| *id == channel)
				{
					transport.send(&utterance.text);
				}
			}
		}
	}

	/// Run until nothing is left to do. What a one-shot run does.
	///
	/// A Session blocked in `await_result` counts as busy: it is suspended, not
	/// done, and the child it waits on is still work. A Task waiting on its own
	/// time is also still work, so this waits for it rather than returning.
	pub async fn run_until_idle(
		self: &Arc<Self>,
		drive: Drive,
	) -> Result<(), StoreError> {
		let mut events = self.events.subscribe();
		loop {
			while self.step(drive).await? {}
			if !self.busy() {
				return Ok(());
			}

			let now = self.clock.now();
			let wait = self.store.next_due_in(now)?;
			let sleep = tokio::time::sleep(match wait {
				Some(d) => std::time::Duration::from_millis(d.0.max(0) as u64),
				None => std::time::Duration::from_secs(3600),
			});
			tokio::pin!(sleep);
			tokio::select! {
				_ = &mut sleep => {},
				event = events.recv() => self.forward_said(event),
			}
		}
	}

	/// A Worker Session's own loop: take a Turn, handle what it earned, repeat
	/// until the Session is done or aborted.
	///
	/// Many of these run at once; the scheduler, not this loop, decides whose
	/// model call is in flight. A Turn that blocks in `await_result` suspends
	/// inside the tool call, so this loop sees it as one long turn.
	async fn drive_worker(self: &Arc<Self>, session: SessionId, task: TaskId) {
		let ctx = self.ctx(session);
		loop {
			match crate::worker::work_turn(&ctx).await {
				crate::worker::Worked::Continue => continue,
				crate::worker::Worked::Done(result) => {
					let _ = self.complete_task(task, result).await;
					break;
				},
				crate::worker::Worked::Aborted => break,
			}
		}
		self.driving.lock().unwrap().remove(&session);
	}

	/// Drain a Channel's mailbox one respond at a time.
	async fn drive_comms(self: &Arc<Self>, channel: ChannelId) {
		if let Ok(Some(session)) = self.store.channel_session(channel) {
			let ctx = self.ctx(session);
			loop {
				crate::comms::respond(&ctx).await;
				match self.store.has_mail(session) {
					Ok(true) => continue,
					_ => break,
				}
			}
		}
		self.comms_driving.lock().unwrap().remove(&channel);
	}

	/// Whether any Session loop is still turning.
	pub fn busy(&self) -> bool {
		if !self.driving.lock().unwrap().is_empty() {
			return true;
		}
		if !self.comms_driving.lock().unwrap().is_empty() {
			return true;
		}
		if self.waiters.any() {
			return true;
		}
		match self.store.tasks_of_run(self.store.run()) {
			Ok(tasks) => tasks.iter().any(|t| !t.state.is_terminal()),
			Err(_) => true,
		}
	}

	/// Stop starting new work. Loops already running finish their turn.
	pub fn stop(&self) {
		self.running.store(false, Ordering::SeqCst);
		// `notify_one`, not `notify_waiters`: `run`'s select may not have
		// reached its wait yet, and only `notify_one` leaves a permit for a
		// waiter that has not registered yet.
		self.woken.notify_one();
	}

	/// Stop everything and make sure nothing can still spend: cancel every Task
	/// that has not finished, then wait for the last in-flight model call to land
	/// so its cost reaches the record.
	pub async fn wind_down(self: &Arc<Self>, timeout: crate::domain::Duration) {
		self.stop();

		if let Ok(tasks) = self.store.tasks_of_run(self.store.run()) {
			let ids: Vec<TaskId> = tasks
				.iter()
				.filter(|t| !t.state.is_terminal())
				.map(|t| t.id)
				.collect();
			for id in ids {
				let _ = self.cancel_task(id).await;
			}
		}

		let deadline = self.clock.now().plus(timeout);
		loop {
			let outstanding = self.scheduler.waiting().await > 0
				|| self.store.calls_outstanding().unwrap_or(false);
			if !outstanding || self.clock.now() >= deadline {
				break;
			}
			tokio::time::sleep(std::time::Duration::from_millis(20)).await;
		}
	}

	/// The context a Session and its tools run against.
	pub fn ctx(
		self: &Arc<Self>,
		session: SessionId,
	) -> crate::session::SessionCtx {
		crate::session::SessionCtx {
			id: session,
			store: self.store.clone(),
			events: self.events.clone(),
			scheduler: self.scheduler.clone(),
			tools: self.tools.clone(),
			clock: self.clock.clone(),
			harness: self.clone(),
		}
	}

	pub fn now(&self) -> Timestamp {
		self.clock.now()
	}
}
