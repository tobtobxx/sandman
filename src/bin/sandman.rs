//! Process boundary and wiring. Only place that builds a Harness.
//!
//! What it is: argv and config in, Harness or control-socket request out.
//! Wiring lives here and only here — which `Model`, `ToolRunner`, `Clock` and
//! `Embedder` is chosen here; everything below receives `Arc`s. Bench reuses
//! the same Harness with different pieces through these seams.
//!
//! Construct: `parse` reads argv → `Config` → `Paths`+`Command`; `assemble`
//! builds `Arc<Harness>` from `Paths`+`Config` (Store, Events, Logger,
//! Scheduler/Models, Registry, Embedder).
//!
//! Use: `main` matches `Command` and delegates:
//!
//! | Command | Builds Store | Talks to | Drives |
//! |---|---|---|---|
//! | Serve | yes | Channels + web UI + socket | `harness.run` until quit |
//! | Bench | no (private per run) | — | runs `Case::run` per case |
//! | Task / List / Spend | no | `control::send` to running Harness | print reply |
//!
//! Call trace (serve):
//! ```text
//! main → parse → assemble → attach stdio/web → spawn web::serve + control::serve
//!      → harness.run → wind_down → end_run → remove socket
//! ```
//!
//! Rules:
//! - **One Sandman per database** — second `sandman serve` on same db is refused by `db::Lock`; `task`/`list`/`spend` never open the DB.
//! - **No second writer** — cross-process entry is `control::Request` via socket; it goes through `Store` and emits `Event`s.
//! - **No flag for a path** — db, log, socket come from `config.toml` together, selected by `--config`.
//! - **No fallback config** — missing file writes default and stops.
//! - **No signal handler** — Ctrl+C aborts; half-written state is cleaned by `Store::open` on next start; graceful exit is `/quit`.

use std::path::PathBuf;
use std::sync::Arc;

use sandman::config::Config;
use sandman::domain::Duration;
use sandman::harness::{Drive, Harness};

/// What both Task-creating commands take.
struct TaskArgs {
	role: String,
	title: Option<String>,
	brief: String,
	at_seconds: Option<i64>,
	every_seconds: Option<i64>,
	priority: Option<String>,
}

/// Where the state, trace and socket live and how to open them.
///
/// All three come from `config.toml` together, selected by `--config`.
/// `break_lock` clears a stale `db::Lock` before opening.
struct Paths {
	db: std::path::PathBuf,
	log: std::path::PathBuf,
	socket: std::path::PathBuf,
	/// Clear a stale lock before opening. See `--break-lock`.
	break_lock: bool,
}

/// Clap argv shape.
#[derive(clap::Parser)]
#[command(
	name = "sandman",
	about = "An agent swarm that coordinates through a shared queue",
	arg_required_else_help = true,
	subcommand_required = true
)]
struct Cli {
	#[command(subcommand)]
	command: Cmd,

	/// Which configuration to read. Defaults per [`Config::path`].
	#[arg(long, global = true)]
	config: Option<PathBuf>,
	/// Write every body in the trace out whole, instead of eliding it.
	#[arg(long, global = true)]
	verbose: bool,
	/// Start even though the database looks locked.
	#[arg(long, global = true)]
	break_lock: bool,
}

#[derive(clap::Subcommand)]
enum Cmd {
	/// Run the swarm, attach channels, serve the web UI and control socket until quit.
	Serve,
	/// Put a Task into a Sandman that is already running.
	Task(TaskFlags),
	/// List a running Sandman's queue.
	List(ListFlags),
	/// What a running Sandman has spent.
	Spend,
	/// Run bench cases against a real model.
	Bench(BenchFlags),
}

#[derive(clap::Args)]
struct TaskFlags {
	#[arg(long)]
	role: String,
	#[arg(long)]
	title: Option<String>,
	#[arg(long)]
	brief: String,
	/// Seconds from now before this Task may run.
	#[arg(long = "at")]
	at: Option<i64>,
	/// Seconds between occurrences, anchored to `--at`.
	#[arg(long = "every")]
	every: Option<i64>,
	#[arg(long)]
	priority: Option<String>,
}

#[derive(clap::Args)]
struct ListFlags {
	/// Only Tasks in this state. Omit for every state.
	#[arg(long)]
	state: Option<String>,
	/// Limit how many to list. Omit for no limit.
	#[arg(long)]
	count: Option<usize>,
}

#[derive(clap::Args)]
struct BenchFlags {
	/// Say which cases there are, and run none of them.
	#[arg(long)]
	list: bool,
	/// Only these cases. Repeatable; omit for every case.
	#[arg(long = "case")]
	case: Vec<String>,
	/// Run each case this many times, for variance.
	#[arg(long, default_value_t = 1)]
	times: usize,
	/// One case at a time instead of all at once.
	#[arg(long)]
	serial: bool,
	/// Where the artifacts go.
	#[arg(long, default_value = "bench/runs")]
	out: std::path::PathBuf,
}

impl From<TaskFlags> for TaskArgs {
	fn from(f: TaskFlags) -> TaskArgs {
		TaskArgs {
			role: f.role,
			title: f.title,
			brief: f.brief,
			at_seconds: f.at,
			every_seconds: f.every,
			priority: f.priority,
		}
	}
}

/// Load config and paths for commands that need them (serve, task, list, spend).
fn load_config_and_paths(
	config_flag: Option<PathBuf>,
	break_lock: bool,
	verbose: bool,
) -> Result<(Paths, Arc<Config>, sandman::log::Verbosity), String> {
	let path = Config::path(config_flag).map_err(|e| e.to_string())?;
	let config = Config::load(&path).map_err(|e| e.to_string())?;
	let paths = Paths {
		db: config.sandman.sqlite_path.clone(),
		log: config.sandman.log_path.clone(),
		socket: config.sandman.control_socket.clone(),
		break_lock,
	};
	let verbosity = if verbose {
		sandman::log::Verbosity::Verbose
	} else {
		sandman::log::Verbosity::Terse
	};
	Ok((paths, Arc::new(config), verbosity))
}

/// Assemble a full Harness from config.
///
/// Opens Store, Events, Logger, Scheduler and registry. Returns `Arc<Harness>`.
async fn assemble(
	paths: &Paths,
	config: Arc<Config>,
	verbosity: sandman::log::Verbosity,
) -> Result<Arc<Harness>, String> {
	use sandman::db::Backing;
	use sandman::domain::{Clock, SystemClock};
	use sandman::event::Events;
	use sandman::log::{Echo, Logger};
	use sandman::memory::{Embedder, OpenRouterEmbedder};
	use sandman::model::Models;
	use sandman::scheduler::Scheduler;
	use sandman::store::Store;
	use sandman::tools::Registry;

	// Create clock and events
	let clock: Arc<dyn Clock> = Arc::new(SystemClock);
	let now = clock.now();
	let events = Arc::new(Events::new(1024));

	// Choose echo target
	let echo = if config.channels.stdio {
		Echo::Quiet
	} else {
		Echo::Stdout
	};

	// Start logger
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

	// Open store
	if paths.break_lock {
		sandman::db::Lock::clear(&paths.db).map_err(|e| {
			format!("could not clear the lock on {}: {e}", paths.db.display())
		})?;
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

	// Log start and migration
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

	// Build scheduler and tools
	let scheduler = Arc::new(Scheduler::new(
		Models::from_config(&config),
		store.clone(),
		clock.clone(),
	));
	let tools = Arc::new(Registry::all(events.clone()));
	let embedder: Arc<dyn Embedder> =
		Arc::new(OpenRouterEmbedder::from_spec(&config.embedding));

	// Build harness
	Ok(Harness::new(
		store, events, scheduler, tools, clock, embedder, config,
	))
}

/// Ensure parent directory of a configured path exists.
fn make_room_for(path: &std::path::Path) -> Result<(), String> {
	let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
	else {
		return Ok(());
	};
	std::fs::create_dir_all(parent)
		.map_err(|e| format!("could not make {}: {e}", parent.display()))
}

/// Run interactive mode until the human quits.
///
/// Attaches configured Channels, serves web UI and control socket,
/// then drives Harness.
async fn interactive(
	paths: Paths,
	config: Arc<Config>,
	verbosity: sandman::log::Verbosity,
) -> Result<(), String> {
	// Assemble harness
	let harness = assemble(&paths, config.clone(), verbosity).await?;

	// Attach terminal channel
	if config.channels.stdio {
		sandman::channels::stdio::attach(harness.clone())
			.await
			.map_err(|e| format!("could not open the terminal: {e}"))?;
	}

	// Attach browser channel
	let web_channel = if config.channels.web {
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

	// Spawn web server
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

	// Spawn control socket
	let socket_harness = harness.clone();
	let socket_path = paths.socket.clone();
	tokio::spawn(async move {
		if let Err(e) =
			sandman::control::serve(socket_harness, &socket_path).await
		{
			eprintln!("control socket stopped: {e}");
		}
	});

	// Drive harness to quit
	harness.run(Drive::Full).await.map_err(|e| e.to_string())?;

	// Wind down
	harness.wind_down(Duration::from_secs(30)).await;
	let _ = harness.store.end_run(harness.now());
	let _ = std::fs::remove_file(&paths.socket);

	Ok(())
}

/// Send a Task into a running Sandman via the control socket.
async fn into_running(args: TaskArgs, paths: Paths) -> Result<(), String> {
	// Build request
	let request = sandman::control::Request::CreateTask {
		role: args.role,
		title: args.title.unwrap_or_else(|| args.brief.clone()),
		brief: args.brief,
		run_at_seconds: args.at_seconds,
		repeat_seconds: args.every_seconds,
		priority: args.priority,
	};

	// Send request
	let response = sandman::control::send(&paths.socket, &request)
		.await
		.map_err(|e| e.to_string())?;

	// Handle response
	match response {
		// Created - print id
		sandman::control::Response::Created { id } => {
			println!("{id}");
			Ok(())
		},
		// Error - propagate message
		sandman::control::Response::Error { message } => Err(message),
		// Unexpected - wrong shape
		_ => Err("the control socket answered a CreateTask with something \
		          else."
			.to_string()),
	}
}

/// List a running Sandman's queue via the control socket.
async fn list(
	state: Option<String>,
	count: Option<usize>,
	paths: Paths,
) -> Result<(), String> {
	// Send request
	let request = sandman::control::Request::ListTasks { state, count };
	let response = sandman::control::send(&paths.socket, &request)
		.await
		.map_err(|e| e.to_string())?;

	// Handle response
	match response {
		// Tasks - print each
		sandman::control::Response::Tasks { tasks } => {
			if tasks.is_empty() {
				println!("No Tasks match.");
			}
			for task in tasks {
				println!(
					"{} [{}] {}: {}",
					task.id, task.state, task.role, task.title
				);
			}
			Ok(())
		},
		// Error - propagate message
		sandman::control::Response::Error { message } => Err(message),
		// Unexpected - wrong shape
		_ => Err("the control socket answered a ListTasks with something \
		          else."
			.to_string()),
	}
}

/// Print what a running Sandman has spent via the control socket.
async fn spend(paths: Paths) -> Result<(), String> {
	// Send request
	let response = sandman::control::send(
		&paths.socket,
		&sandman::control::Request::Spend,
	)
	.await
	.map_err(|e| e.to_string())?;

	// Handle response
	match response {
		// Spent - print totals
		sandman::control::Response::Spent { calls, tokens, cost } => {
			println!("Spent {calls} call(s), {tokens} token(s), {cost}");
			Ok(())
		},
		// Error - propagate message
		sandman::control::Response::Error { message } => Err(message),
		// Unexpected - wrong shape
		_ => Err("the control socket answered a Spend with something else."
			.to_string()),
	}
}

// ---------------------------------------------------------------------------
// bench subcommand — previously bin/bench.rs
// ---------------------------------------------------------------------------

use sandman::bench::report::{self, CaseSummary, RunReport};
use sandman::bench::{Case, CASES};

/// List every case.
///
/// Prints padded name and dim description, one per line.
fn bench_list() {
	let on = sandman::bench::color::enabled();
	let width = CASES.iter().map(|c| c.name.len()).max().unwrap_or(0);
	for case in CASES {
		let name = format!("{:width$}", case.name);
		println!(
			"{}  {}",
			sandman::bench::color::bold(on, &name),
			sandman::bench::color::dim(on, case.description),
		);
	}
}

/// Run one case and persist its artifacts.
///
/// Executes `case.run()` and writes `result.json`/`store.sqlite`/`sandman.log`.
/// Returns `RunReport`; only artifact write failure becomes `Err`.
async fn bench_run_once(
	case: &Case,
	dir: &std::path::Path,
) -> Result<RunReport, String> {
	let (rig, report) = (case.run)().await;
	if let Some(rig) = rig {
		report::write_artifacts(&report, &rig, dir).map_err(|e| {
			format!("could not write artifacts to {}: {e}", dir.display())
		})?;
	}
	Ok(report)
}

/// Announce a run before awaiting it.
///
/// Prevents silence that reads as hang while a real model call is in flight.
fn bench_announce(on: bool, case: &str, k: usize, times: usize) {
	let what = if times > 1 {
		format!("{case} (run {k}/{times})")
	} else {
		case.to_string()
	};
	println!("{} {what}", sandman::bench::color::cyan(on, "→"));
}

/// Summarize runs per case.
///
/// Computes pass count, mean wall time, and combined cost in first-seen order.
fn bench_summarize(reports: &[(String, RunReport)]) -> Vec<CaseSummary> {
	let mut order: Vec<String> = Vec::new();
	let mut by_case: std::collections::HashMap<String, Vec<&RunReport>> =
		std::collections::HashMap::new();
	for (name, report) in reports {
		by_case
			.entry(name.clone())
			.or_insert_with(|| {
				order.push(name.clone());
				Vec::new()
			})
			.push(report);
	}
	order
		.into_iter()
		.map(|name| {
			let runs = &by_case[&name];
			let passed = runs.iter().filter(|r| r.pass).count();
			let mean_wall_ms =
				runs.iter().map(|r| r.wall_ms).sum::<i64>() / runs.len() as i64;
			let total_cost =
				runs.iter().fold(sandman::domain::Cost(0), |acc, r| {
					acc + r.spend.cost + r.grader_cost
				});
			CaseSummary {
				case: name,
				runs: runs.len(),
				passed,
				mean_wall_ms,
				total_cost,
			}
		})
		.collect()
}

/// Execute the bench subcommand.
async fn bench(flags: BenchFlags) -> Result<(), String> {
	if flags.list {
		bench_list();
		return Ok(());
	}

	let times = flags.times.max(1);

	// Resolve cases
	let cases: Vec<&'static Case> = if flags.case.is_empty() {
		CASES.iter().collect()
	} else {
		flags
			.case
			.iter()
			.map(|name| {
				sandman::bench::cases::find(name).ok_or_else(|| {
					let known: Vec<&str> =
						CASES.iter().map(|c| c.name).collect();
					format!(
						"`{name}` is not a case. Known cases: {}",
						known.join(", ")
					)
				})
			})
			.collect::<Result<Vec<_>, String>>()?
	};

	let on = sandman::bench::color::enabled();
	let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
	let total = cases.len() * times;
	println!(
		"{}",
		sandman::bench::color::bold(
			on,
			&format!(
				"Running {} case(s), {} time(s) each — {total} run(s) total",
				cases.len(),
				times
			)
		)
	);

	let mut reports: Vec<(String, RunReport)> = Vec::new();
	let mut done = 0usize;

	if flags.serial {
		for &case in &cases {
			for k in 1..=times {
				bench_announce(on, case.name, k, times);
				let dir = report::run_dir(&flags.out, &stamp, case.name, k);
				done += 1;
				match bench_run_once(case, &dir).await {
					Ok(report) => {
						print!(
							"{} ",
							sandman::bench::color::dim(
								on,
								&format!("[{done}/{total}]")
							)
						);
						report::print_run(&report);
						reports.push((case.name.to_string(), report));
					},
					Err(e) => eprintln!("{}: {e}", case.name),
				}
			}
		}
	} else {
		let mut set = tokio::task::JoinSet::new();
		for &case in &cases {
			for k in 1..=times {
				bench_announce(on, case.name, k, times);
				let dir = report::run_dir(&flags.out, &stamp, case.name, k);
				set.spawn(
					async move { (case.name, bench_run_once(case, &dir).await) },
				);
			}
		}
		while let Some(outcome) = set.join_next().await {
			done += 1;
			match outcome {
				Ok((name, Ok(report))) => {
					print!(
						"{} ",
						sandman::bench::color::dim(
							on,
							&format!("[{done}/{total}]")
						)
					);
					report::print_run(&report);
					reports.push((name.to_string(), report));
				},
				Ok((name, Err(e))) => eprintln!("{name}: {e}"),
				Err(e) => eprintln!("a case task panicked: {e}"),
			}
		}
	}

	report::print_summary(&bench_summarize(&reports));
	Ok(())
}

#[tokio::main]
async fn main() {
	use clap::Parser;

	let cli = Cli::try_parse().unwrap_or_else(|e| e.exit());

	let Cli { command, config: config_flag, verbose, break_lock } = cli;
	let load = || load_config_and_paths(config_flag.clone(), break_lock, verbose);
	let result = match command {
		Cmd::Serve => {
			let (paths, config, verbosity) =
				match load() {
					Ok(v) => v,
					Err(message) => {
						eprintln!("{message}");
						std::process::exit(1);
					},
				};
			interactive(paths, config, verbosity).await
		},
		Cmd::Task(flags) => {
			let (paths, _, _) = match load() {
				Ok(v) => v,
				Err(message) => {
					eprintln!("{message}");
					std::process::exit(1);
				},
			};
			into_running(flags.into(), paths).await
		},
		Cmd::List(flags) => {
			let (paths, _, _) = match load() {
				Ok(v) => v,
				Err(message) => {
					eprintln!("{message}");
					std::process::exit(1);
				},
			};
			list(flags.state, flags.count, paths).await
		},
		Cmd::Spend => {
			let (paths, _, _) = match load() {
				Ok(v) => v,
				Err(message) => {
					eprintln!("{message}");
					std::process::exit(1);
				},
			};
			spend(paths).await
		},
		Cmd::Bench(flags) => bench(flags).await,
	};

	if let Err(message) = result {
		eprintln!("{message}");
		std::process::exit(1);
	}
}
