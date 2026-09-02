//! Serve command — the only place that builds a Harness.
//!
//! What it is: `assemble` wires `Store`, `Events`, `Logger`, `Scheduler`/
//! `Models`, `Registry` and `Embedder` into an `Arc<Harness>`; `interactive`
//! attaches channels, spawns the web UI and control socket, then drives the
//! harness until the user quits.
//!
//! Construct: `assemble(&Paths, Arc<Config>, Verbosity) → (Arc<Harness>,
//! Arc<Logger>)` — the `Logger` comes back out because the Matrix Channel
//! writes its transport trouble there instead of onto the terminal.
//! Use: `main` on `Cmd::Serve` → `load_config_and_paths` → `interactive` →
//! `harness.run(Drive::Full)` → `wind_down` → `end_run` → remove socket.
//! Consumers: `main` only.
//!
//! Call trace:
//! ```text
//! interactive → assemble → attach stdio/web/matrix → spawn web::serve + control::serve
//!             → harness.run → wind_down → end_run → remove socket
//! ```
//!
//! Rules: **wiring lives here and only here** — model/tool/clock/embedder
//! choice is not scattered. **one Sandman per database** — second `serve` on
//! same db is refused by `db::Lock`; `task`/`list`/`spend` never open the DB.

use std::sync::Arc;

use sandman::config::Config;
use sandman::domain::Duration;
use sandman::harness::{Drive, Harness};
use sandman::log::Logger;

use crate::paths::{Paths, make_room_for};

/// Assemble a full Harness from config.
///
/// Opens Store, Events, Logger, Scheduler and registry. Returns the Harness
/// and the Logger, which the Matrix Channel writes its transport trouble to.
pub async fn assemble(
	paths: &Paths,
	config: Arc<Config>,
	verbosity: sandman::log::Verbosity,
) -> Result<(Arc<Harness>, Arc<Logger>), String> {
	use sandman::db::Backing;
	use sandman::domain::{Clock, SystemClock};
	use sandman::event::Events;
	use sandman::log::Echo;
	use sandman::memory::{Embedder, OpenRouterEmbedder};
	use sandman::model::Models;
	use sandman::scheduler::Scheduler;
	use sandman::store::Store;
	use sandman::tools::Registry;

	let clock: Arc<dyn Clock> = Arc::new(SystemClock);
	let now = clock.now();
	let events = Arc::new(Events::new(1024));

	let echo = if config.channels.stdio.enable {
		Echo::Quiet
	} else {
		Echo::Stdout
	};

	make_room_for(&paths.log)?;
	let logger =
		Arc::new(Logger::create(&paths.log, verbosity, echo).map_err(|e| {
			format!("could not open {}: {e}", paths.log.display())
		})?);
	{
		let logger = logger.clone();
		let events = events.clone();
		tokio::spawn(async move { logger.follow(&events).await });
	}

	if paths.break_lock {
		sandman::db::Lock::clear(&paths.db).map_err(|e| {
			format!("could not clear the lock on {}: {e}", paths.db.display())
		})?;
	}
	// Probe each distinct *used* model with a tiny max_tokens=1 request.
	// Only models referenced by `models.*`, `metacognition.model` or
	// `bench.grader` are probed — an unused `[model.*]` table does not
	// block startup. Fail fast before touching the DB so a bad model
	// leaves no Run behind.
	{
		use sandman::model::{Model, OpenRouter};
		use std::collections::HashSet;
		use strum::VariantArray;
		let mut seen: HashSet<sandman::config::ModelSpec> = HashSet::new();
		let mut distinct: Vec<&sandman::config::ModelSpec> = Vec::new();
		for spec in [
			config.for_all(),
			config.for_comms(),
			config.for_metacognition(),
			config.for_grader(),
		]
		.into_iter()
		.chain(
			sandman::roles::RoleName::VARIANTS
				.iter()
				.map(|r| config.for_role(*r)),
		) {
			if seen.insert(spec.clone()) {
				distinct.push(spec);
			}
		}
		distinct.sort_by(|a, b| a.model.cmp(&b.model));
		for spec in distinct {
			logger.note(
				"model",
				&format!("probing {} at {} ...", spec.model, spec.endpoint),
			);
			let m = OpenRouter::from_spec(spec);
			match m.probe().await {
				Ok(()) => {
					logger.note("model", &format!("{} ok", spec.model));
				},
				Err(e) => {
					let msg = format!(
						"model {} at {} failed probe: {e}",
						spec.model, spec.endpoint
					);
					logger.note("model", &msg);
					return Err(msg);
				},
			}
		}
	}
	let model_name = config.for_all().model.clone();
	make_room_for(&paths.db)?;
	let store = Arc::new(
		Store::open(
			Backing::File(paths.db.clone()),
			events.clone(),
			&model_name,
			now,
		)
		.map_err(|e| format!("could not open {}: {e}", paths.db.display()))?,
	);

	logger.note(
		"sandman",
		&sandman::log::banner(&format!(
			"started, model {model_name}, db {}",
			paths.db.display()
		)),
	);
	if let Some((from, to)) = store.migration() {
		logger.note("db", &format!("migrated schema v{from} to v{to}"));
	}

	let scheduler = Arc::new(Scheduler::new(
		Models::from_config(&config),
		store.clone(),
		clock.clone(),
	));
	let tools = Arc::new(Registry::all(events.clone()));
	let embedder: Arc<dyn Embedder> =
		Arc::new(OpenRouterEmbedder::from_spec(&config.embedding));

	Ok((
		Harness::new(store, events, scheduler, tools, clock, embedder, config),
		logger,
	))
}

/// Run the swarm until the human quits.
///
/// Attaches configured Channels, serves web UI and control socket,
/// then drives Harness.
pub async fn interactive(
	paths: Paths,
	config: Arc<Config>,
	verbosity: sandman::log::Verbosity,
) -> Result<(), String> {
	let (harness, logger) = assemble(&paths, config.clone(), verbosity).await?;

	// Matrix first: it is the one that can fail, and a failure should not
	// leave a prompt behind on the terminal.
	if config.channels.matrix.enable {
		sandman::channels::matrix::attach(
			harness.clone(),
			&config.channels.matrix,
			logger.clone(),
		)
		.await
		.map_err(|e| format!("could not open the Matrix channel: {e}"))?;
	}

	if config.channels.stdio.enable {
		sandman::channels::stdio::attach(harness.clone())
			.await
			.map_err(|e| format!("could not open the terminal: {e}"))?;
	}

	let web_channel = if config.channels.web.enable {
		Some(
			sandman::channels::web::attach(harness.clone())
				.await
				.map_err(|e| {
					format!("could not open the browser channel: {e}")
				})?,
		)
	} else {
		None
	};

	let web_state = sandman::web::server::AppState {
		harness: harness.clone(),
		channel: web_channel,
	};
	let address = config.sandman.webui_address;
	let port = config.sandman.webui_port;
	tokio::spawn(async move {
		if let Err(e) =
			sandman::web::server::serve(web_state, address, port).await
		{
			eprintln!("web UI stopped: {e}");
		}
	});

	let socket_harness = harness.clone();
	let socket_path = paths.socket.clone();
	tokio::spawn(async move {
		if let Err(e) =
			sandman::control::serve(socket_harness, &socket_path).await
		{
			eprintln!("control socket stopped: {e}");
		}
	});

	harness.run(Drive::Full).await.map_err(|e| e.to_string())?;

	harness.wind_down(Duration::from_secs(30)).await;
	let _ = harness.store.end_run(harness.now());
	let _ = std::fs::remove_file(&paths.socket);

	Ok(())
}
