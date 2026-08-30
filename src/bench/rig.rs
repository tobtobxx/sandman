//! One Sandman under test, and everything needed to drive and read it.
//!
//! A Rig is a whole Harness: a private in-memory database, its own Event stream,
//! its own scheduler, its own log file in a temporary directory that is removed
//! with it. Nothing in it is process-global, which is why a case is a test rather
//! than a process.
//!
//! What a case chooses is how much of it is real: real prompts, real model, real
//! clock, unless it says otherwise in its own first lines. The tools are the
//! exception — they are replaced in every case, because a bench is one Session's
//! decisions and the interceptor is what keeps it to one.
//!
//! Waiting is [`Rig::until`]: it follows the Event stream, re-checks the
//! predicate whenever something changed, and evaluates every tripwire on the way
//! past. A tripped wire comes back as [`Trip`], which a test propagates with `?`.
//!
//! Winding down is not optional and not the caller's problem to remember.
//! [`Rig::wind_down`] cancels everything unfinished and waits for the last
//! in-flight call so its cost reaches the record; `Drop` aborts the driver tasks
//! as a backstop, so a case that panics cannot leave a Harness spending.
//!
//! Defines: [`Rig`], [`RigBuilder`], [`ModelChoice`], [`ClockChoice`], [`Watch`].

use std::sync::Arc;

use super::{CheckResult, Interceptor, Trip};
use crate::db::Backing;
use crate::domain::{
	CallId, CallStatus, ChannelId, ChannelKind, Clock, Duration, FixedClock,
	IncomingFrom, NewLesson, NewTask, SessionId, SessionStatus, Spend,
	SystemClock, Task, TaskId, TaskState, Timestamp, Utterance,
};
use crate::event::Events;
use crate::harness::{Drive, Harness};
use crate::log::{Logger, Verbosity};
use crate::model::{Model, OpenRouter};
use crate::scheduler::Scheduler;
use crate::store::Store;
use crate::tools::{Registry, ToolRunner};

/// How long a Rig waits before a case trips on its own clock, unless a case
/// says otherwise with [`RigBuilder::timeout`]. Generous for a real model over
/// the wire, short enough that a hung case does not stall a whole bench run.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Where the model's answers come from.
pub enum ModelChoice {
	/// The real model, over the real wire. What a bench case measures.
	Real,
	/// Replies written by the test, in order. For exercising the Harness itself
	/// — the turn loop, the scheduler, the review — without spending anything.
	Scripted(Vec<crate::domain::Completion>),
	/// Anything else the test wants to supply.
	Custom(Arc<dyn crate::model::Model>),
}

/// Where time comes from.
pub enum ClockChoice {
	/// The real clock. What a case asserting on the model's judgement of time
	/// must use: faking it would bench a Sandman that does not exist.
	Real,
	/// Stopped, so every timestamp in a run is the same and comparable.
	Fixed(Timestamp),
	/// Only moves when the test moves it. For a case that needs a scheduled Task
	/// to actually fire — which is a case about the Harness, not the model, and
	/// should say so.
	Manual(Arc<crate::domain::ManualClock>),
}

/// Builds a Rig. Everything defaults to real except the tools, which default to
/// [`super::ToolsChoice::Deny`]: a case pays for what it asks for.
pub struct RigBuilder {
	model: ModelChoice,
	tools: super::ToolsChoice,
	clock: ClockChoice,
	drive: Drive,
	channels: Vec<crate::domain::ChannelKind>,
	timeout: Option<Duration>,
	log: crate::log::Verbosity,
}

/// One Sandman under test.
pub struct Rig {
	pub harness: Arc<Harness>,
	pub store: Arc<Store>,
	pub interceptor: Arc<super::Interceptor>,
	/// The Channel a case's script speaks on, if it opened one.
	pub channel: Option<ChannelId>,
	events: tokio::sync::broadcast::Receiver<crate::event::Event>,
	tripwires: Vec<Tripwire>,
	started_at: Timestamp,
	deadline: Option<Timestamp>,
	dir: tempfile::TempDir,
	drivers: Vec<tokio::task::JoinHandle<()>>,
}

/// A condition evaluated continuously: "this must never happen".
struct Tripwire {
	name: String,
	pred: Box<dyn Fn(&Watch) -> CheckResult + Send + Sync>,
}

/// What a tripwire may look at.
///
/// The Store alone is not enough for a unit bench: a case that answers
/// `create_task` itself leaves no row behind, so "a second Task spawning" can
/// only be seen in the calls.
pub struct Watch<'a> {
	pub store: &'a Store,
	pub calls: &'a [super::RecordedToolCall],
}

/// The bench's own Channel: a case's script, not a real transport.
///
/// `send` does nothing — the transcript already lives in the Store, and a case
/// reads it back with [`Rig::transcript`] rather than watching a terminal or a
/// socket.
struct BenchChannel {
	id: std::sync::OnceLock<ChannelId>,
	kind: ChannelKind,
}

impl crate::comms::Channel for BenchChannel {
	fn id(&self) -> ChannelId {
		*self
			.id
			.get()
			.expect("id is set right after attach mints it")
	}

	fn kind(&self) -> ChannelKind {
		self.kind
	}

	fn send(&self, _text: &str) {}
}

impl Default for RigBuilder {
	fn default() -> Self {
		RigBuilder {
			model: ModelChoice::Real,
			tools: super::ToolsChoice::Deny,
			clock: ClockChoice::Real,
			drive: Drive::Manual,
			channels: Vec::new(),
			timeout: None,
			log: Verbosity::Terse,
		}
	}
}

impl RigBuilder {
	pub fn model(mut self, choice: ModelChoice) -> Self {
		self.model = choice;
		self
	}

	pub fn tools(mut self, choice: super::ToolsChoice) -> Self {
		self.tools = choice;
		self
	}

	pub fn clock(mut self, choice: ClockChoice) -> Self {
		self.clock = choice;
		self
	}

	/// How much the Harness starts by itself. A case wants the least that gets
	/// its one Session running: [`Drive::CommsOnly`] for a Comms Session, and
	/// [`Drive::Full`] for a seeded Task, whose children the interceptor answers
	/// rather than lets run.
	pub fn drive(mut self, drive: Drive) -> Self {
		self.drive = drive;
		self
	}

	/// Open a Channel a script can speak on.
	pub fn channel(mut self, kind: crate::domain::ChannelKind) -> Self {
		self.channels.push(kind);
		self
	}

	/// The whole case must finish inside this, or it trips.
	pub fn timeout(mut self, within: Duration) -> Self {
		self.timeout = Some(within);
		self
	}

	pub async fn build(self) -> Result<Rig, Trip> {
		let setup = |e: std::io::Error| Trip::Tripwire {
			name: "setup".to_string(),
			detail: e.to_string(),
		};

		let clock: Arc<dyn Clock> = match self.clock {
			ClockChoice::Real => Arc::new(SystemClock),
			ClockChoice::Fixed(at) => Arc::new(FixedClock(at)),
			ClockChoice::Manual(manual) => manual,
		};
		let now = clock.now();

		let events = Arc::new(Events::new(1024));
		let subscription = events.subscribe();

		let dir = tempfile::TempDir::new().map_err(setup)?;
		let log_path = dir.path().join("sandman.log");
		let logger =
			Arc::new(Logger::create(&log_path, self.log).map_err(setup)?);
		let mut drivers = Vec::new();
		{
			let logger = logger.clone();
			let events = events.clone();
			drivers.push(tokio::spawn(
				async move { logger.follow(&events).await },
			));
		}

		let model: Arc<dyn Model> = match self.model {
			ModelChoice::Real => Arc::new(OpenRouter::from_env()),
			ModelChoice::Scripted(replies) => {
				Arc::new(super::script::ScriptedModel::new(replies))
			},
			ModelChoice::Custom(model) => model,
		};
		let model_name = model.name().to_string();

		let store = Arc::new(
			Store::open(Backing::Memory, events.clone(), &model_name, now)
				.map_err(Trip::from)?,
		);

		let scheduler =
			Arc::new(Scheduler::new(model, store.clone(), clock.clone()));

		let registry: Arc<dyn ToolRunner> =
			Arc::new(Registry::all(events.clone()));
		let interceptor = Arc::new(Interceptor::new(registry, self.tools));
		let tools: Arc<dyn ToolRunner> = interceptor.clone();

		let harness =
			Harness::new(store.clone(), events, scheduler, tools, clock);

		let mut channel = None;
		for kind in self.channels {
			let transport =
				Arc::new(BenchChannel { id: std::sync::OnceLock::new(), kind });
			let id = harness
				.attach(transport.clone())
				.await
				.map_err(Trip::from)?;
			transport
				.id
				.set(id)
				.expect("attach runs once per BenchChannel");
			channel = Some(id);
		}

		{
			let driven = harness.clone();
			let drive = self.drive;
			drivers.push(tokio::spawn(async move {
				let _ = driven.run(drive).await;
			}));
		}

		let timeout = self.timeout.unwrap_or(DEFAULT_TIMEOUT);

		Ok(Rig {
			harness,
			store,
			interceptor,
			channel,
			events: subscription,
			tripwires: Vec::new(),
			started_at: now,
			deadline: Some(now.plus(timeout)),
			dir,
			drivers,
		})
	}
}

impl Rig {
	pub fn builder() -> RigBuilder {
		RigBuilder::default()
	}

	/// When this Rig's clock started, for [`super::report::assemble`] to weigh
	/// the run's wall time against.
	pub fn started_at(&self) -> Timestamp {
		self.started_at
	}

	// --- Filling the state --------------------------------------------------

	/// Put a Task on the queue as though the command line had.
	///
	/// An ordinary Store write through the ordinary path: nothing here reaches
	/// around the Harness to plant a row.
	pub fn seed_task(&self, new: NewTask) -> Result<TaskId, Trip> {
		Ok(self.harness.create_task(new)?)
	}

	/// Write a lesson, so a case can test what the `memory` Role finds without
	/// first making a swarm earn one.
	pub fn seed_lesson(
		&self,
		new: NewLesson,
	) -> Result<crate::domain::LessonId, Trip> {
		let now = self.harness.now();
		Ok(self.store.keep_lesson(new, now)?)
	}

	/// What the human on the bench Channel says.
	pub async fn send(&self, text: &str) {
		let channel = self
			.channel
			.expect("a case that calls send() opened a channel first");
		self.harness
			.receive(channel, text, IncomingFrom::Human)
			.await;
	}

	// --- Driving ------------------------------------------------------------

	/// Add a condition that must hold for the whole run.
	pub fn tripwire(
		&mut self,
		name: &str,
		pred: impl Fn(&Watch) -> CheckResult + Send + Sync + 'static,
	) {
		self.tripwires
			.push(Tripwire { name: name.to_string(), pred: Box::new(pred) });
	}

	fn check_tripwires(&self) -> Result<(), Trip> {
		let calls = self.interceptor.calls();
		let watch = Watch { store: &self.store, calls: &calls };
		for tripwire in &self.tripwires {
			let result = (tripwire.pred)(&watch);
			if !result.ok {
				return Err(Trip::Tripwire {
					name: tripwire.name.clone(),
					detail: result.detail,
				});
			}
		}
		Ok(())
	}

	/// Wait until something is true.
	///
	/// Follows the Event stream: the predicate is re-checked when something
	/// changed, and every tripwire with it. No polling, and no clock involved
	/// except the case's own bound.
	pub async fn until(
		&mut self,
		what: &str,
		pred: impl Fn(&Store) -> bool,
	) -> Result<(), Trip> {
		loop {
			self.check_tripwires()?;
			if pred(&self.store) {
				return Ok(());
			}

			let now = self.harness.now();
			if let Some(deadline) = self.deadline {
				if now >= deadline {
					return Err(Trip::Timeout { what: what.to_string() });
				}
			}

			let wait = self.deadline.map(|d| now.until(d));
			let sleep = tokio::time::sleep(match wait {
				Some(d) => std::time::Duration::from_millis(d.0.max(1) as u64),
				None => std::time::Duration::from_secs(3600),
			});
			tokio::pin!(sleep);
			tokio::select! {
				_ = &mut sleep => {},
				recv = self.events.recv() => {
					use tokio::sync::broadcast::error::RecvError;
					if let Err(RecvError::Closed) = recv {
						return Err(Trip::Timeout { what: what.to_string() });
					}
					// Ok(_) and Lagged both just mean "something happened, or
					// might have" — either way the loop goes round and
					// re-checks, which is what survives a Lagged.
				},
			}
		}
	}

	/// Wait for a fixed span, still watching tripwires. For the rare case that
	/// has to let a Session keep going for a while.
	pub async fn idle_for(&mut self, span: Duration) -> Result<(), Trip> {
		let target = self.harness.now().plus(span);
		loop {
			self.check_tripwires()?;
			let now = self.harness.now();
			if now >= target {
				return Ok(());
			}
			if let Some(deadline) = self.deadline {
				if now >= deadline {
					let seconds =
						(deadline.0 - self.started_at.0).max(0) / 1000;
					return Err(Trip::CaseTimeout { seconds: seconds as u64 });
				}
			}

			let remaining = now.until(target);
			let sleep = tokio::time::sleep(std::time::Duration::from_millis(
				remaining.0.max(1) as u64,
			));
			tokio::pin!(sleep);
			tokio::select! {
				_ = &mut sleep => {},
				recv = self.events.recv() => {
					use tokio::sync::broadcast::error::RecvError;
					if let Err(RecvError::Closed) = recv {
						return Ok(());
					}
				},
			}
		}
	}

	/// True when a Comms Session has finished answering: mail read, idle, and at
	/// least one model call made since `since_calls`.
	///
	/// Take the count *before* sending, or it is true before the run has done
	/// anything.
	pub fn comms_idle(&self, since_calls: usize) -> Result<bool, Trip> {
		let Some(channel) = self.channel else {
			return Ok(false);
		};
		let Some(session) = self.store.channel_session(channel)? else {
			return Ok(false);
		};
		let Some(row) = self.store.session(session)? else {
			return Ok(false);
		};
		let idle = matches!(row.status, SessionStatus::Idle);
		let has_mail = self.store.has_mail(session)?;
		Ok(idle && !has_mail && row.calls.len() > since_calls)
	}

	/// Say this on the bench Channel, and wait for the Comms Session to finish
	/// answering it — mail read, idle, one more model call than before. The
	/// ceremony every conversational case needs, written once here instead of
	/// by hand in each one.
	pub async fn converse(&mut self, text: &str) -> Result<(), Trip> {
		let channel = self
			.channel
			.expect("a case that calls converse() opened a channel first");
		let session = self
			.store
			.channel_session(channel)?
			.expect("a comms session stands on this channel");
		let baseline = self
			.store
			.session(session)?
			.map(|s| s.calls.len())
			.unwrap_or(0);

		self.send(text).await;
		self.until("the comms session finishes replying", move |store| {
			let Ok(Some(row)) = store.session(session) else {
				return false;
			};
			let idle = matches!(row.status, SessionStatus::Idle);
			let has_mail = matches!(store.has_mail(session), Ok(true));
			idle && !has_mail && row.calls.len() > baseline
		})
		.await
	}

	/// Wait for a seeded Task to reach a terminal Result, success or failure —
	/// what a case that seeded one wants before reading [`Rig::tasks`] for the
	/// outcome.
	pub async fn await_task(&mut self, task: TaskId) -> Result<(), Trip> {
		self.until("the Task completes", move |store| {
			matches!(
				store.task(task),
				Ok(Some(t)) if matches!(t.state, TaskState::Completed { .. })
			)
		})
		.await
	}

	// --- Reading ------------------------------------------------------------

	/// Every tool call any Session made, in order, with what it answered. The
	/// unit bench's whole point.
	pub fn tool_calls(&self) -> Vec<super::RecordedToolCall> {
		self.interceptor.calls()
	}

	/// What the human on the bench Channel has seen and said.
	pub fn transcript(&self) -> Result<Vec<Utterance>, Trip> {
		let channel = self
			.channel
			.expect("a case reading the transcript opened a channel first");
		Ok(self.store.transcript(channel)?)
	}

	pub fn tasks(&self) -> Result<Vec<Task>, Trip> {
		Ok(self.store.tasks_of_run(self.store.run())?)
	}

	pub fn spend(&self) -> Result<Spend, Trip> {
		Ok(self.harness.spend()?)
	}

	/// Model calls that never came back, with the wire error — how an expired key
	/// or a dead endpoint becomes visible without opening the log. The checks
	/// alone would only say "no reply".
	pub fn failed_calls(
		&self,
	) -> Result<Vec<(CallId, SessionId, String)>, Trip> {
		let snapshot = self.store.snapshot()?;
		Ok(snapshot
			.calls
			.into_iter()
			.filter_map(|call| match call.status {
				CallStatus::Failed { error, .. } => {
					Some((call.id, call.session, error))
				},
				_ => None,
			})
			.collect())
	}

	// --- Ending -------------------------------------------------------------

	/// Stop everything and make sure nothing can still spend.
	///
	/// Cancels every unfinished Task, then waits for the last in-flight call to
	/// land so the cost record stays honest.
	pub async fn wind_down(&mut self) {
		self.harness.wind_down(Duration::from_secs(30)).await;
		let _ = self.store.end_run(self.harness.now());
		for handle in self.drivers.drain(..) {
			handle.abort();
		}
	}

	/// Write the artifacts: `result.json`, `sandman.log` and `store.sqlite` —
	/// the whole database of the run, with every Session's transcript and every
	/// model call's request and reply, queryable afterwards with `sqlite3`.
	///
	/// Takes the directory rather than assuming the working one, which is what
	/// lets several cases write artifacts from one process.
	pub fn save_to(&self, dir: &std::path::Path) -> std::io::Result<()> {
		std::fs::create_dir_all(dir)?;
		self.store
			.save_copy(&dir.join("store.sqlite"))
			.map_err(|e| {
				std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
			})?;
		let log_path = self.dir.path().join("sandman.log");
		if log_path.exists() {
			std::fs::copy(&log_path, dir.join("sandman.log"))?;
		}
		Ok(())
	}
}

impl Drop for Rig {
	/// The backstop. A case that panicked never reached `wind_down`, and a
	/// driver task left running would keep spending against a Harness nothing
	/// is watching.
	fn drop(&mut self) {
		for handle in self.drivers.drain(..) {
			handle.abort();
		}
	}
}
