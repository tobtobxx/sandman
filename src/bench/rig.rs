//! One Sandman under test — private Harness, Store, scheduler, Event stream and log.
//!
//! Construct: `RigBuilder::default` → `RigBuilder::{model,clock,tools,drive,channel,config}` → `build() → Rig`.
//! Use: seed state (`seed_task`, `seed_lesson`, `send`), drive (`until`, `idle_for`, `converse`, `await_task`) with tripwires, read (`tool_calls`, `transcript`, `tasks`, `spend`, `failed_calls`), end (`wind_down`, `save_to`).
//! Consumers: `cases` via `Case::run`, `report::assemble` (winds down and reports), `bin/bench` and `tests/cases.rs`.
//!
//! Seams — real unless replaced:
//! | Seam | Real | Bench |
//! | --- | --- | --- |
//! | Model | OpenRouter (`Models::from_config`) | `ScriptedModel` / custom `Model` |
//! | ToolRunner | `Registry` | `Interceptor` (always wrapped) |
//! | Clock | `SystemClock` | `FixedClock` / `ManualClock` |
//! | Embedder | `OpenRouterEmbedder` | test-supplied via config |
//!
//! Rules:
//! - **One Rig is one Sandman** — private in-memory DB, Events, scheduler, log dir; nothing process-global.
//! - **`Drive` controls what starts** — `Manual`/`CommsOnly`/`Full`; bench is one Session by construction.
//! - **`until` is event-driven** — predicate and tripwires re-checked on each Event, no polling.
//! - **The case bound is `bench.timeout`** — one deadline from config, the same for every case.
//! - **Wind-down is mandatory** — `wind_down` cancels unfinished Tasks and waits for in-flight calls; `Drop` aborts drivers.
//! - **Channels are bench transports** — `BenchChannel::send` is no-op, transcript lives in `Store`.
//!
//! Defines: `Rig`, `RigBuilder`, `ModelChoice`, `ClockChoice`, `Watch`.

use std::sync::Arc;

use super::{CheckResult, Interceptor, Trip};
use crate::config::Config;
use crate::db::Backing;
use crate::domain::{
	CallId, CallStatus, ChannelId, ChannelKind, Clock, Duration, FixedClock,
	IncomingFrom, NewLesson, NewTask, SessionId, SessionStatus, Spend,
	SystemClock, Task, TaskId, TaskState, Timestamp, Utterance,
};
use crate::event::Events;
use crate::harness::{Drive, Harness};
use crate::log::{Echo, Logger, Verbosity};
use crate::model::{Model, Models};
use crate::scheduler::Scheduler;
use crate::store::Store;
use crate::tools::{Registry, ToolRunner};

/// Where model replies come from.
pub enum ModelChoice {
	/// Real model over the wire.
	Real,
	/// Replies in order, no spend.
	Scripted(Vec<crate::domain::Completion>),
	/// Caller-supplied model.
	Custom(Arc<dyn crate::model::Model>),
}

/// Where time comes from.
pub enum ClockChoice {
	/// Real system clock.
	Real,
	/// Stopped clock, every timestamp equal.
	Fixed(Timestamp),
	/// Advances only when test advances it.
	Manual(Arc<crate::domain::ManualClock>),
}

/// Builds a Rig. Defaults to real except tools (`Deny`).
pub struct RigBuilder {
	model: ModelChoice,
	tools: super::ToolsChoice,
	clock: ClockChoice,
	drive: Drive,
	channels: Vec<crate::domain::ChannelKind>,
	log: crate::log::Verbosity,
	config: Option<Arc<Config>>,
}

/// One Sandman under test.
pub struct Rig {
	pub harness: Arc<Harness>,
	pub store: Arc<Store>,
	pub interceptor: Arc<super::Interceptor>,
	/// Config this Rig was built from. Grader reads it here, not from Harness.
	pub config: Arc<Config>,
	/// Bench Channel the script speaks on, if opened.
	pub channel: Option<ChannelId>,
	events: tokio::sync::broadcast::Receiver<crate::event::Event>,
	tripwires: Vec<Tripwire>,
	started_at: Timestamp,
	/// Case bound from `bench.timeout`. Every wait stops here.
	deadline: Timestamp,
	dir: tempfile::TempDir,
	drivers: Vec<tokio::task::JoinHandle<()>>,
}

/// Condition evaluated on every Event. Trips the case when violated.
struct Tripwire {
	name: String,
	pred: Box<dyn Fn(&Watch) -> CheckResult + Send + Sync>,
}

/// Snapshot tripwires read: Store plus intercepted tool calls.
pub struct Watch<'a> {
	pub store: &'a Store,
	pub calls: &'a [super::RecordedToolCall],
}

/// Bench Channel for case scripts. `send` is no-op; transcript lives in Store.
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
			log: Verbosity::Terse,
			config: None,
		}
	}
}

impl RigBuilder {
	pub fn model(mut self, choice: ModelChoice) -> Self {
		self.model = choice;
		self
	}

	/// Use given config instead of machine file.
	pub fn config(mut self, config: Arc<Config>) -> Self {
		self.config = Some(config);
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

	/// How much Harness starts by itself.
	pub fn drive(mut self, drive: Drive) -> Self {
		self.drive = drive;
		self
	}

	/// Open a bench Channel the script can speak on.
	pub fn channel(mut self, kind: crate::domain::ChannelKind) -> Self {
		self.channels.push(kind);
		self
	}

	/// Build Rig with private DB, scheduler, Harness and log.
	pub async fn build(self) -> Result<Rig, Trip> {
		// Resolve config
		let setup = |e: std::io::Error| Trip::Tripwire {
			name: "setup".to_string(),
			detail: e.to_string(),
		};

		let config = match self.config {
			Some(config) => config,
			None => {
				let read = Config::path(None).and_then(|p| Config::read(&p));
				Arc::new(read.map_err(|e| Trip::Tripwire {
					name: "setup".to_string(),
					detail: e.to_string(),
				})?)
			},
		};

		// Resolve clock
		let clock: Arc<dyn Clock> = match self.clock {
			ClockChoice::Real => Arc::new(SystemClock),
			ClockChoice::Fixed(at) => Arc::new(FixedClock(at)),
			ClockChoice::Manual(manual) => manual,
		};
		let now = clock.now();

		// Create event stream
		let events = Arc::new(Events::new(1024));
		let subscription = events.subscribe();

		// Create temp log
		let dir = tempfile::TempDir::new().map_err(setup)?;
		let log_path = dir.path().join("sandman.log");
		let logger = Arc::new(
			Logger::create(&log_path, self.log, Echo::Quiet).map_err(setup)?,
		);
		let mut drivers = Vec::new();

		// Spawn log driver
		{
			let logger = logger.clone();
			let events = events.clone();
			drivers.push(tokio::spawn(
				async move { logger.follow(&events).await },
			));
		}

		// Resolve models
		let (models, model_name) = match self.model {
			ModelChoice::Real => {
				(Models::from_config(&config), config.for_all().model.clone())
			},
			ModelChoice::Scripted(replies) => {
				let model: Arc<dyn Model> =
					Arc::new(super::script::ScriptedModel::new(replies));
				let name = model.name().to_string();
				(Models::uniform(model), name)
			},
			ModelChoice::Custom(model) => {
				let name = model.name().to_string();
				(Models::uniform(model), name)
			},
		};

		// Open store
		let store = Arc::new(
			Store::open(Backing::Memory, events.clone(), &model_name, now)
				.map_err(Trip::from)?,
		);

		// Build scheduler
		let scheduler =
			Arc::new(Scheduler::new(models, store.clone(), clock.clone()));

		// Wrap tool runner
		let registry: Arc<dyn ToolRunner> =
			Arc::new(Registry::all(events.clone()));
		let interceptor = Arc::new(Interceptor::new(registry, self.tools));
		let tools: Arc<dyn ToolRunner> = interceptor.clone();

		// Build harness
		let embedder: Arc<dyn crate::memory::Embedder> = Arc::new(
			crate::memory::OpenRouterEmbedder::from_spec(&config.embedding),
		);
		let harness = Harness::new(
			store.clone(),
			events,
			scheduler,
			tools,
			clock,
			embedder,
			config.clone(),
		);

		// Attach bench channels
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

		// Spawn harness driver
		{
			let driven = harness.clone();
			let drive = self.drive;
			drivers.push(tokio::spawn(async move {
				let _ = driven.run(drive).await;
			}));
		}

		// Assemble rig
		let timeout = Duration::from_secs(config.bench.timeout);

		Ok(Rig {
			harness,
			store,
			interceptor,
			config,
			channel,
			events: subscription,
			tripwires: Vec::new(),
			started_at: now,
			deadline: now.plus(timeout),
			dir,
			drivers,
		})
	}
}

impl Rig {
	pub fn builder() -> RigBuilder {
		RigBuilder::default()
	}

	/// Clock time when Rig started.
	pub fn started_at(&self) -> Timestamp {
		self.started_at
	}

	// --- Filling the state --------------------------------------------------

	/// Queue a Task as if from command line. Returns id.
	pub fn seed_task(&self, new: NewTask) -> Result<TaskId, Trip> {
		Ok(self.harness.create_task(new)?)
	}

	/// Insert a lesson for `memory` tests. Returns id.
	pub fn seed_lesson(
		&self,
		new: NewLesson,
	) -> Result<crate::domain::LessonId, Trip> {
		let now = self.harness.now();
		Ok(self.store.keep_lesson(new, now)?)
	}

	/// Send text on bench Channel as human.
	pub async fn send(&self, text: &str) {
		let channel = self
			.channel
			.expect("a case that calls send() opened a channel first");
		self.harness
			.receive(channel, text, IncomingFrom::Human)
			.await;
	}

	// --- Driving ------------------------------------------------------------

	/// Add condition that must hold for whole run.
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

	/// Wait until predicate holds. Re-checks on each Event and evaluates tripwires.
	pub async fn until(
		&mut self,
		what: &str,
		pred: impl Fn(&Store) -> bool,
	) -> Result<(), Trip> {
		loop {
			// Check tripwires and predicate
			self.check_tripwires()?;
			if pred(&self.store) {
				return Ok(());
			}

			// Enforce deadline
			let now = self.harness.now();
			if now >= self.deadline {
				return Err(Trip::Timeout { what: what.to_string() });
			}

			// Wait for event or deadline
			let wait = now.until(self.deadline);
			let sleep = tokio::time::sleep(std::time::Duration::from_millis(
				wait.0.max(1) as u64,
			));
			tokio::pin!(sleep);
			tokio::select! {
				_ = &mut sleep => {},
				recv = self.events.recv() => {
					use tokio::sync::broadcast::error::RecvError;
					if let Err(RecvError::Closed) = recv {
						return Err(Trip::Timeout { what: what.to_string() });
					}
					// Re-check on any event
				},
			}
		}
	}

	/// Wait for span while still watching tripwires.
	pub async fn idle_for(&mut self, span: Duration) -> Result<(), Trip> {
		let target = self.harness.now().plus(span);
		loop {
			// Check tripwires
			self.check_tripwires()?;
			let now = self.harness.now();
			if now >= target {
				return Ok(());
			}

			// Enforce case bound
			if now >= self.deadline {
				let seconds =
					(self.deadline.0 - self.started_at.0).max(0) / 1000;
				return Err(Trip::CaseTimeout { seconds: seconds as u64 });
			}

			// Wait for event or target
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
					// Re-check on any event
				},
			}
		}
	}

	/// True when Comms Session is idle, has no mail, and made a call since baseline.
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

	/// Send text and wait for Comms Session to finish replying.
	pub async fn converse(&mut self, text: &str) -> Result<(), Trip> {
		// Record baseline
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

		// Send and wait
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

	/// Wait for Task to reach terminal state.
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

	/// Every tool call in order, with answer.
	pub fn tool_calls(&self) -> Vec<super::RecordedToolCall> {
		self.interceptor.calls()
	}

	/// Transcript of bench Channel.
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

	/// Model calls that never returned, with wire error.
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

	/// Cancel unfinished Tasks and wait for in-flight calls to settle.
	pub async fn wind_down(&mut self) {
		// Cancel tasks and settle calls
		self.harness.wind_down(Duration::from_secs(30)).await;
		let _ = self.store.end_run(self.harness.now());
		// Abort drivers
		for handle in self.drivers.drain(..) {
			handle.abort();
		}
	}

	/// Write `store.sqlite` and `sandman.log` into dir.
	pub fn save_to(&self, dir: &std::path::Path) -> std::io::Result<()> {
		// Create dir and copy store
		std::fs::create_dir_all(dir)?;
		self.store
			.save_copy(&dir.join("store.sqlite"))
			.map_err(|e| {
				std::io::Error::other(e.to_string())
			})?;
		// Copy log if present
		let log_path = self.dir.path().join("sandman.log");
		if log_path.exists() {
			std::fs::copy(&log_path, dir.join("sandman.log"))?;
		}
		Ok(())
	}
}

impl Drop for Rig {
	/// Abort drivers if case panicked before `wind_down`.
	fn drop(&mut self) {
		for handle in self.drivers.drain(..) {
			handle.abort();
		}
	}
}
