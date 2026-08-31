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
//! | Interactive | yes | Channels + web UI + socket | `harness.run` until quit |
//! | Run | yes | — | `create_task` then `run_until_idle`, print results |
//! | Task / List / Spend | no | `control::send` to running Harness | print reply |
//!
//! Call trace (interactive):
//! ```text
//! main → parse → assemble → attach stdio/web → spawn web::serve + control::serve
//!      → harness.run → wind_down → end_run → remove socket
//! ```
//!
//! Rules:
//! - **One Sandman per database** — second `sandman`/`run` on same db is refused by `db::Lock`; `task`/`list`/`spend` never open the DB.
//! - **No second writer** — cross-process entry is `control::Request` via socket; it goes through `Store` and emits `Event`s.
//! - **No flag for a path** — db, log, socket come from `config.toml` together, selected by `--config`.
//! - **No fallback config** — missing file writes default and stops.
//! - **No signal handler** — Ctrl+C aborts; half-written state is cleaned by `Store::open` on next start; graceful exit is `/quit`.

use std::path::PathBuf;
use std::sync::Arc;

use sandman::config::Config;
use sandman::domain::{
	Brief, Creator, Duration, NewTask, Schedule, TaskPriority, TaskState, Title,
};
use sandman::harness::{Drive, Harness};
use sandman::roles::RoleName;

/// Which way in this invocation is.
enum Command {
	Interactive,
	/// A one-shot Task in its own Harness.
	Run(TaskArgs),
	/// A Task into a Sandman already running.
	Task(TaskArgs),
	/// A running Sandman's queue, over the control socket.
	List {
		state: Option<String>,
		count: Option<usize>,
	},
	/// What a running Sandman has spent, over the control socket.
	Spend,
}

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

/// Clap argv shape. Private to `parse`; rest of file uses `Command`/`TaskArgs`/`Paths`.
#[derive(clap::Parser)]
#[command(
	name = "sandman",
	about = "An agent swarm that coordinates through a shared queue"
)]
struct Cli {
	#[command(subcommand)]
	command: Option<Cmd>,

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
	/// One Task, run until nothing is left, then print its Results and what it cost.
	Run(TaskFlags),
	/// Put a Task into a Sandman that is already running.
	Task(TaskFlags),
	/// List a running Sandman's queue.
	List(ListFlags),
	/// What a running Sandman has spent.
	Spend,
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

/// Parse argv and load the config it names.
///
/// Returns command, paths and verbosity for `main` to dispatch.
/// Reads config for every command, including socket-only ones.
fn parse(
	argv: &[String],
) -> Result<(Command, Paths, Arc<Config>, sandman::log::Verbosity), String> {
	use clap::Parser;

	// Parse argv or exit
	let cli = Cli::try_parse_from(argv).unwrap_or_else(|e| e.exit());

	// Load config
	let path = Config::path(cli.config).map_err(|e| e.to_string())?;
	let config = Config::load(&path).map_err(|e| e.to_string())?;

	// Build paths and verbosity
	let paths = Paths {
		db: config.sandman.sqlite_path.clone(),
		log: config.sandman.log_path.clone(),
		socket: config.sandman.control_socket.clone(),
		break_lock: cli.break_lock,
	};
	let verbosity = if cli.verbose {
		sandman::log::Verbosity::Verbose
	} else {
		sandman::log::Verbosity::Terse
	};

	// Map to command
	let command = match cli.command {
		None => Command::Interactive,
		Some(Cmd::Run(flags)) => Command::Run(flags.into()),
		Some(Cmd::Task(flags)) => Command::Task(flags.into()),
		Some(Cmd::List(flags)) => {
			Command::List { state: flags.state, count: flags.count }
		},
		Some(Cmd::Spend) => Command::Spend,
	};

	Ok((command, paths, Arc::new(config), verbosity))
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

/// Run one Task to completion in its own Harness.
///
/// Creates the Task, drains to idle, prints results and spend.
async fn one_shot(
	args: TaskArgs,
	paths: Paths,
	config: Arc<Config>,
	verbosity: sandman::log::Verbosity,
) -> Result<(), String> {
	// Assemble harness
	let harness = assemble(&paths, config, verbosity).await?;

	// Parse role and fields
	let role = args
		.role
		.parse::<RoleName>()
		.map_err(|_| format!("`{}` is not a Role.", args.role))?;
	let title =
		Title::try_from(args.title.unwrap_or_else(|| args.brief.clone()))
			.map_err(|e| e.to_string())?;
	let brief = Brief::try_from(args.brief).map_err(|e| e.to_string())?;
	let priority = match args.priority.as_deref() {
		None => TaskPriority::default(),
		Some(given) => given.parse().map_err(|_| {
			format!("`{given}` is not a priority. Use high, normal or low.")
		})?,
	};
	let schedule = Schedule::from_offsets(
		args.at_seconds,
		args.every_seconds,
		harness.now(),
	);

	// Create task
	let new = NewTask {
		title,
		brief,
		role,
		schedule,
		priority,
		created_by: Creator::Cli,
	};
	harness.create_task(new).map_err(|e| e.to_string())?;

	// Drain to idle
	harness
		.run_until_idle(Drive::Full)
		.await
		.map_err(|e| e.to_string())?;

	// Print results
	let tasks = harness
		.store
		.tasks_of_run(harness.store.run())
		.map_err(|e| e.to_string())?;
	for task in &tasks {
		match &task.state {
			// Completed - print result
			TaskState::Completed { result, .. } => {
				println!(
					"{} \"{}\": {}",
					task.id,
					task.title,
					result.content()
				);
			},
			// Cancelled - note it
			TaskState::Cancelled { .. } => {
				println!("{} \"{}\": cancelled.", task.id, task.title);
			},
			// Pending or running - unreachable after drain
			TaskState::Pending | TaskState::Running { .. } => {},
		}
	}

	// Print spend
	let spend = harness.spend().map_err(|e| e.to_string())?;
	println!(
		"Spent {} call(s), {} token(s), {}",
		spend.calls, spend.tokens, spend.cost
	);

	// Wind down
	harness.wind_down(Duration::from_secs(30)).await;
	let _ = harness.store.end_run(harness.now());

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

#[tokio::main]
async fn main() {
	// Parse argv and config
	let argv: Vec<String> = std::env::args().collect();
	let (command, paths, config, verbosity) = match parse(&argv) {
		Ok(parsed) => parsed,
		Err(message) => {
			eprintln!("{message}");
			std::process::exit(1);
		},
	};

	// Dispatch by command
	let result = match command {
		Command::Interactive => interactive(paths, config, verbosity).await,
		Command::Run(args) => one_shot(args, paths, config, verbosity).await,
		Command::Task(args) => into_running(args, paths).await,
		Command::List { state, count } => list(state, count, paths).await,
		Command::Spend => spend(paths).await,
	};

	// Report errors
	if let Err(message) = result {
		eprintln!("{message}");
		std::process::exit(1);
	}
}
