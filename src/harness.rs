//! Plain-code orchestrator that owns Tasks, Sessions and Channels through the Store.
//!
//! Construct: `Harness::new(store, events, scheduler, tools, clock, embedder, config) -> Arc<Self>`;
//! `attach(channel)` mints Channel and standing Comms Session; `ctx(id)` builds
//! `SessionCtx` threaded through every Session and tool.
//! Use: `step(drive)` starts one ready unit (Comms mail first, then next pending Task) → `bool`;
//! `run(drive)` loops `step` and waits on `Events` or due timers; `run_until_idle(drive)`
//! same until `!busy()`; `drive_worker`/`drive_comms` are the spawned loops.
//! Consumers: `session::turn` via `SessionCtx`, `worker::work_turn` and `comms::respond`
//! as driven policies, `channels::*` adapters via `attach`/`receive`/`forward_said`,
//! `control::serve`, `bench::Rig`, `web::server`, `bin/sandman` (only site that builds a Harness).
//! Seam: `Drive` selects what `step` may start:
//!
//! | `Drive` | Comms mail | Pending Tasks |
//! | --- | --- | --- |
//! | `Manual` | no | no |
//! | `CommsOnly` | yes | no |
//! | `Full` | yes | yes |
//!
//! Call trace: `run → step → drive_comms → comms::respond → session::turn`
//! and `step → drive_worker → worker::work_turn → session::turn`;
//! `complete_task → deliver → waiters::resolve`; `step → store.fire_cron` for a cron Task;
//! `cancel_task → cancel_tasks → waiters::resolve`.
//! Rules: **only Store touches the database; every mutation emits one Event.**
//! **a Turn decides nothing; ending policy lives in worker.rs/comms.rs, never session.rs.**
//! **one model call in flight, ordered by Tier then arrival.**
//! **one respond per Channel (`comms_driving`), one Worker per Task (`driving`).**
//! **scheduler and Events are broadcast; slow consumers lose Events, never block.**
//!
//! Defines: [`Harness`], [`Drive`], [`CancelOutcome`].

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use crate::domain::{
	ChannelId, ChannelKind, Clock, Incoming, IncomingFrom, Message, NewTask,
	Schedule, SessionId, Spend, TaskId, TaskResult, Timestamp, Utterance, Who,
};
use crate::event::{Event, Events};
use crate::scheduler::Scheduler;
use crate::store::{Store, StoreError};
use crate::tools::ToolRunner;
use crate::waiters::Waiters;

/// How much work `step` may start.
///
/// Controls whether pending Tasks and Comms mail are driven. Bench uses
/// `CommsOnly` to measure a Comms decision without executing Tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drive {
	/// Nothing starts by itself.
	Manual,
	/// Channels with mail are answered; pending Tasks stay queued.
	CommsOnly,
	/// Tasks run and Channels with mail are answered.
	Full,
}

/// Result of cancelling a Task.
///
/// Tells the caller whether the Task stopped, and whether it was running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelOutcome {
	NotFound,
	/// Already finished; nothing to stop.
	Completed,
	/// Already cancelled.
	Already,
	Cancelled {
		/// It was running, so its Session stops at its next decision.
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
	pub embedder: Arc<dyn crate::memory::Embedder>,
	pub waiters: Arc<Waiters>,
	/// Config this Sandman was built from.
	pub config: Arc<crate::config::Config>,

	/// Comms Session and transport for each open Channel.
	comms: Mutex<Vec<(ChannelId, Arc<dyn crate::comms::Channel>)>>,
	/// Worker Sessions with a Turn loop in flight. Ids only, to avoid cycles.
	driving: Mutex<HashSet<SessionId>>,
	/// Channels with a respond in flight. One at a time per Channel.
	comms_driving: Mutex<HashSet<ChannelId>>,
	running: std::sync::atomic::AtomicBool,
	/// Woken by `stop` to interrupt `run` sleep.
	woken: tokio::sync::Notify,
}

impl Harness {
	pub fn new(
		store: Arc<Store>,
		events: Arc<Events>,
		scheduler: Arc<Scheduler>,
		tools: Arc<dyn ToolRunner>,
		clock: Arc<dyn Clock>,
		embedder: Arc<dyn crate::memory::Embedder>,
		config: Arc<crate::config::Config>,
	) -> Arc<Self> {
		Arc::new(Harness {
			store,
			events,
			scheduler,
			tools,
			clock,
			embedder,
			config,
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

	/// Record a Task's Result and deliver it to its subscriber.
	///
	/// Resolves waiters. A cron Task never reaches here — it never runs.
	async fn complete_task(
		&self,
		id: TaskId,
		result: TaskResult,
	) -> Result<(), StoreError> {
		// Complete task
		let now = self.clock.now();
		self.store.complete_task(id, result, now)?;

		// Deliver answer
		let task = self
			.store
			.task(id)?
			.ok_or(StoreError::NoSuch { what: "task", id: id.to_string() })?;
		let answer = task.render_answer();

		self.deliver(id).await?;
		self.waiters.resolve(id, &answer);
		Ok(())
	}

	/// Cancel one Task.
	///
	/// Stops it pending or running and releases blocked waiters. Reaches that
	/// Task alone: cancelling a cron Task makes no further daughters and lets
	/// the ones already out finish; cancelling a daughter leaves its cron Task
	/// armed.
	pub async fn cancel_task(
		&self,
		id: TaskId,
	) -> Result<CancelOutcome, StoreError> {
		// Check current state
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

		// Cancel in store
		let now = self.clock.now();
		self.store.cancel_tasks(&[id], now)?;

		// Resolve waiters
		let notice = task.render_cancelled();
		self.waiters.resolve(id, &notice);
		if let Some(session) = running_session {
			self.waiters.resolve_held_by(session, &notice);
		}

		Ok(CancelOutcome::Cancelled { running })
	}

	/// Hand a Task's answer to its subscriber.
	///
	/// Only Comms Sessions subscribe; Workers wait directly.
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

	/// Open a Channel with its standing Comms Session.
	///
	/// Mints both ids and tracks the transport.
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

	/// Enqueue text on a Channel's Comms Session.
	///
	/// Records human utterances in the transcript and always enqueues mail.
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
		// Record transcript
		if from == IncomingFrom::Human {
			let _ = self.store.say(
				channel,
				Utterance { who: Who::Human, text: text.to_string(), at: now },
			);
		}
		// Enqueue mail
		let _ = self.store.receive_mail(
			session,
			Incoming { from, text: text.to_string(), at: now },
		);
	}

	/// List open Channels for schema generation.
	pub fn open_channels(&self) -> Vec<(ChannelId, ChannelKind)> {
		self.comms
			.lock()
			.unwrap()
			.iter()
			.map(|(id, channel)| (*id, channel.kind()))
			.collect()
	}

	/// Spend for this Run.
	pub fn spend(&self) -> Result<Spend, StoreError> {
		self.store.spend(self.store.run())
	}

	// --- Driving -----------------------------------------------------------

	/// Try to start one ready unit.
	///
	/// Tries Comms mail first, then the next pending Task. Returns whether
	/// anything started.
	pub async fn step(
		self: &Arc<Self>,
		drive: Drive,
	) -> Result<bool, StoreError> {
		if drive == Drive::Manual {
			return Ok(false);
		}

		// Try comms first
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

		// Try next Task
		let now = self.clock.now();
		let Some(task) = self.store.next_pending(now)? else {
			return Ok(false);
		};

		// A cron Task makes work instead of being work
		if matches!(task.schedule, Schedule::Cron { .. }) {
			self.store.fire_cron(&task, now)?;
			return Ok(true);
		}

		let ctx = self.ctx(SessionId(0));
		let session = crate::worker::new_worker_session(&ctx, &task).await?;
		self.store.start_task(task.id, session, now)?;
		self.driving.lock().unwrap().insert(session);

		let this = self.clone();
		let task_id = task.id;
		tokio::spawn(async move { this.drive_worker(session, task_id).await });
		Ok(true)
	}

	/// Run until stopped.
	///
	/// Starts ready work and waits on Events or due timers. Returns when
	/// `stop` is called.
	pub async fn run(self: &Arc<Self>, drive: Drive) -> Result<(), StoreError> {
		let mut events = self.events.subscribe();
		while self.running.load(Ordering::SeqCst) {
			// Start ready work
			while self.step(drive).await? {
				if !self.running.load(Ordering::SeqCst) {
					return Ok(());
				}
			}

			// Wait for wakeup
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
			// Drain remaining events
			while let Ok(event) = events.try_recv() {
				self.forward_said(Ok(event));
			}
		}
		Ok(())
	}

	/// Forward `Said` utterances to their transport.
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

	/// Drive until idle.
	///
	/// Starts ready work and waits for Events or timers. Returns when no
	/// Session, waiter or Task remains. Blocked and scheduled Tasks count as busy.
	pub async fn run_until_idle(
		self: &Arc<Self>,
		drive: Drive,
	) -> Result<(), StoreError> {
		let mut events = self.events.subscribe();
		loop {
			// Start ready work
			while self.step(drive).await? {}
			// Check if idle
			if !self.busy() {
				return Ok(());
			}

			// Wait for wakeup
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

	/// Drive a Worker until it completes or aborts.
	async fn drive_worker(self: &Arc<Self>, session: SessionId, task: TaskId) {
		// Loop turns
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
		// Remove from driving
		self.driving.lock().unwrap().remove(&session);
	}

	/// Drain a Channel's mailbox one respond at a time.
	async fn drive_comms(self: &Arc<Self>, channel: ChannelId) {
		// Drain mailbox
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
		// Remove from driving
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

	/// Stop starting new work.
	///
	/// Running loops finish their current turn.
	pub fn stop(&self) {
		self.running.store(false, Ordering::SeqCst);
		// Wake waiter
		self.woken.notify_one();
	}

	/// Stop new work, cancel remaining Tasks and wait for calls to settle.
	pub async fn wind_down(self: &Arc<Self>, timeout: crate::domain::Duration) {
		self.stop();

		// Cancel remaining Tasks
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

		// Wait for calls
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

	/// Build a `SessionCtx` for the given Session.
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

	/// Current time via the Harness clock.
	pub fn now(&self) -> Timestamp {
		self.clock.now()
	}
}
