//! Sandman's entry point. Three ways in.
//!
//! ```text
//! sandman
//!     Interactive. Two Channels at once — the terminal, and a browser on
//!     :8080 that also watches everything the Harness owns, live. A control
//!     socket is opened so another process can put work in.
//!
//! sandman run --role planning --title "..." --brief "..."
//!            [--at 600] [--every 86400] [--priority high|normal|low]
//!     One Task, run until nothing is left, then print the Results and what it
//!     cost. Its own Harness; no socket, no browser.
//!
//! sandman task --role planning --title "..." --brief "..."
//!             [--at 600] [--every 86400] [--priority high|normal|low]
//!     Put a Task into a Sandman that is already running, through its control
//!     socket, and print the id. This is how anything that is not a Channel —
//!     cron, an RSS script, a mail watcher — gets work in.
//!
//! sandman list [--state pending|running|completed|cancelled] [--count N]
//!     List a running Sandman's queue, over the control socket.
//!
//! sandman spend
//!     What a running Sandman has spent, over the control socket.
//! ```
//!
//! Common flags: `--db <path>` (default `sandman.sqlite`), `--log <path>`
//! (default `sandman.log`), `--socket <path>`, `--verbose`, `--break-lock`.
//!
//! One Sandman per database. `sandman` and `sandman run` each open their own,
//! and the second to start on a database is refused rather than allowed to
//! cancel the first one's work — see [`sandman::db::Lock`]. `task`, `list` and
//! `spend` open nothing: they go through the control socket, which is how you
//! reach a Sandman that is already running.
//!
//! Wiring lives here and only here: which [`sandman::model::Model`], which
//! [`sandman::tools::ToolRunner`], which [`sandman::domain::Clock`]. Everything
//! below takes what it needs and builds nothing itself, which is what lets the
//! bench assemble the same Harness with different pieces.
//!
//! Interactive mode opens three ways in: the terminal Channel, the browser
//! Channel with its Watcher UI on [`sandman::web::PORT`], and the control
//! socket.

use std::path::PathBuf;
use std::sync::Arc;

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

/// Where the state, the trace and the socket live, and how to open the first.
struct Paths {
	db: std::path::PathBuf,
	log: std::path::PathBuf,
	socket: std::path::PathBuf,
	/// Take the database's lock however it looks. See `--break-lock`.
	break_lock: bool,
}

/// The argv shape, read by `clap`. Kept private to `parse`: everywhere else in
/// this file works in terms of [`Command`], [`TaskArgs`] and [`Paths`], which
/// say what Sandman does rather than how the flags spelled it.
#[derive(clap::Parser)]
#[command(
	name = "sandman",
	about = "An agent swarm that coordinates through a shared queue"
)]
struct Cli {
	#[command(subcommand)]
	command: Option<Cmd>,

	/// Where the database lives.
	#[arg(long, global = true, default_value = "sandman.sqlite")]
	db: PathBuf,
	/// Where the trace goes.
	#[arg(long, global = true, default_value = "sandman.log")]
	log: PathBuf,
	/// Where the control socket lives. Defaults per [`sandman::control::socket_path`].
	#[arg(long, global = true)]
	socket: Option<PathBuf>,
	/// Write every body in the trace out whole, instead of eliding it.
	#[arg(long, global = true)]
	verbose: bool,
	/// Start even though the database looks locked.
	///
	/// A lock left by a dead Sandman is cleared on its own — the pid in it is
	/// checked. This is for the one case that check cannot settle, a pid the
	/// system has since given to something else. Using it while a Sandman is
	/// really running will cancel that Sandman's work.
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

fn parse(
	argv: &[String],
) -> Result<(Command, Paths, sandman::log::Verbosity), String> {
	use clap::Parser;

	// `Error::exit` prints to the right stream (stdout for `--help` and
	// `--version`, stderr otherwise) and leaves with the matching code, so a
	// bad or absent argv never reaches the rest of this function.
	let cli = Cli::try_parse_from(argv).unwrap_or_else(|e| e.exit());

	let paths = Paths {
		db: cli.db,
		log: cli.log,
		socket: cli.socket.unwrap_or_else(sandman::control::socket_path),
		break_lock: cli.break_lock,
	};
	let verbosity = if cli.verbose {
		sandman::log::Verbosity::Verbose
	} else {
		sandman::log::Verbosity::Terse
	};
	let command = match cli.command {
		None => Command::Interactive,
		Some(Cmd::Run(flags)) => Command::Run(flags.into()),
		Some(Cmd::Task(flags)) => Command::Task(flags.into()),
		Some(Cmd::List(flags)) => {
			Command::List { state: flags.state, count: flags.count }
		},
		Some(Cmd::Spend) => Command::Spend,
	};

	Ok((command, paths, verbosity))
}

/// Build a whole Sandman: database, Event stream, logger, model, tools,
/// scheduler, Harness.
async fn assemble(
	paths: &Paths,
	verbosity: sandman::log::Verbosity,
) -> Result<Arc<Harness>, String> {
	use sandman::db::Backing;
	use sandman::domain::{Clock, SystemClock};
	use sandman::event::Events;
	use sandman::log::Logger;
	use sandman::model::{Model, OpenRouter, MODEL};
	use sandman::scheduler::Scheduler;
	use sandman::store::Store;
	use sandman::tools::Registry;

	let clock: Arc<dyn Clock> = Arc::new(SystemClock);
	let now = clock.now();

	let events = Arc::new(Events::new(1024));

	let logger =
		Arc::new(Logger::create(&paths.log, verbosity).map_err(|e| {
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
	let store = Arc::new(
		Store::open(
			Backing::File(paths.db.clone()),
			events.clone(),
			MODEL,
			now,
		)
		.map_err(|e| format!("could not open {}: {e}", paths.db.display()))?,
	);
	logger.note(
		"sandman",
		&sandman::log::banner(&format!(
			"started, model {MODEL}, db {}",
			paths.db.display()
		)),
	);
	if let Some((from, to)) = store.migration() {
		logger.note("db", &format!("migrated schema v{from} to v{to}"));
	}

	let model: Arc<dyn Model> = Arc::new(OpenRouter::from_env());
	let scheduler =
		Arc::new(Scheduler::new(model, store.clone(), clock.clone()));
	let tools = Arc::new(Registry::all(events.clone()));

	Ok(Harness::new(store, events, scheduler, tools, clock))
}

/// Two Channels, a Watcher, and a control socket, running until the human
/// leaves.
async fn interactive(
	paths: Paths,
	verbosity: sandman::log::Verbosity,
) -> Result<(), String> {
	let harness = assemble(&paths, verbosity).await?;

	sandman::channels::stdio::attach(harness.clone())
		.await
		.map_err(|e| format!("could not open the terminal: {e}"))?;

	let web_channel = sandman::channels::web::attach(harness.clone())
		.await
		.map_err(|e| format!("could not open the browser channel: {e}"))?;
	let web_state = sandman::web::server::AppState {
		harness: harness.clone(),
		embedder: Arc::new(sandman::memory::OpenRouterEmbedder::from_env()),
		channel: web_channel,
	};
	tokio::spawn(async move {
		if let Err(e) =
			sandman::web::server::serve(web_state, sandman::web::PORT).await
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

	// No signal handler, deliberately. Ctrl+C and a kill both mean *abort*:
	// the process dies where it stands, in-flight model calls with it. What
	// that leaves half-written in the database — Tasks marked running,
	// Sessions still open, calls still out — the next start ends, in
	// `Store::open`. Leaving is `/quit` or Ctrl+D, which is the path below.
	harness.run(Drive::Full).await.map_err(|e| e.to_string())?;

	harness.wind_down(Duration::from_secs(30)).await;
	let _ = harness.store.end_run(harness.now());
	let _ = std::fs::remove_file(&paths.socket);

	Ok(())
}

/// One Task in its own Harness, until nothing is left. Prints every Task's
/// Result and what the run spent.
async fn one_shot(
	args: TaskArgs,
	paths: Paths,
	verbosity: sandman::log::Verbosity,
) -> Result<(), String> {
	let harness = assemble(&paths, verbosity).await?;

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

	let new = NewTask {
		title,
		brief,
		role,
		schedule,
		priority,
		created_by: Creator::Cli,
	};
	harness.create_task(new).map_err(|e| e.to_string())?;

	harness
		.run_until_idle(Drive::Full)
		.await
		.map_err(|e| e.to_string())?;

	let tasks = harness
		.store
		.tasks_of_run(harness.store.run())
		.map_err(|e| e.to_string())?;
	for task in &tasks {
		match &task.state {
			TaskState::Completed { result, .. } => {
				println!(
					"{} \"{}\": {}",
					task.id,
					task.title,
					result.content()
				);
			},
			TaskState::Cancelled { .. } => {
				println!("{} \"{}\": cancelled.", task.id, task.title);
			},
			TaskState::Pending | TaskState::Running { .. } => {},
		}
	}

	let spend = harness.spend().map_err(|e| e.to_string())?;
	println!(
		"Spent {} call(s), {} token(s), {}",
		spend.calls, spend.tokens, spend.cost
	);

	harness.wind_down(Duration::from_secs(30)).await;
	let _ = harness.store.end_run(harness.now());

	Ok(())
}

/// One Task into a running Sandman, over the control socket.
async fn into_running(args: TaskArgs, paths: Paths) -> Result<(), String> {
	let request = sandman::control::Request::CreateTask {
		role: args.role,
		title: args.title.unwrap_or_else(|| args.brief.clone()),
		brief: args.brief,
		run_at_seconds: args.at_seconds,
		repeat_seconds: args.every_seconds,
		priority: args.priority,
	};
	let response = sandman::control::send(&paths.socket, &request)
		.await
		.map_err(|e| e.to_string())?;

	match response {
		sandman::control::Response::Created { id } => {
			println!("{id}");
			Ok(())
		},
		sandman::control::Response::Error { message } => Err(message),
		_ => Err("the control socket answered a CreateTask with something \
		          else."
			.to_string()),
	}
}

/// A running Sandman's queue, over the control socket.
async fn list(
	state: Option<String>,
	count: Option<usize>,
	paths: Paths,
) -> Result<(), String> {
	let request = sandman::control::Request::ListTasks { state, count };
	let response = sandman::control::send(&paths.socket, &request)
		.await
		.map_err(|e| e.to_string())?;

	match response {
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
		sandman::control::Response::Error { message } => Err(message),
		_ => Err("the control socket answered a ListTasks with something \
		          else."
			.to_string()),
	}
}

/// What a running Sandman has spent, over the control socket.
async fn spend(paths: Paths) -> Result<(), String> {
	let response = sandman::control::send(
		&paths.socket,
		&sandman::control::Request::Spend,
	)
	.await
	.map_err(|e| e.to_string())?;

	match response {
		sandman::control::Response::Spent { calls, tokens, cost } => {
			println!("Spent {calls} call(s), {tokens} token(s), {cost}");
			Ok(())
		},
		sandman::control::Response::Error { message } => Err(message),
		_ => Err("the control socket answered a Spend with something else."
			.to_string()),
	}
}

#[tokio::main]
async fn main() {
	let argv: Vec<String> = std::env::args().collect();
	let (command, paths, verbosity) = match parse(&argv) {
		Ok(parsed) => parsed,
		Err(message) => {
			eprintln!("{message}");
			std::process::exit(1);
		},
	};

	let result = match command {
		Command::Interactive => interactive(paths, verbosity).await,
		Command::Run(args) => one_shot(args, paths, verbosity).await,
		Command::Task(args) => into_running(args, paths).await,
		Command::List { state, count } => list(state, count, paths).await,
		Command::Spend => spend(paths).await,
	};

	if let Err(message) = result {
		eprintln!("{message}");
		std::process::exit(1);
	}
}
