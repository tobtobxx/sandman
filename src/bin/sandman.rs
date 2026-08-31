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
//! Common flags: `--config <path>`, `--verbose`, `--break-lock`.
//!
//! Everything else is `config.toml` — see [`sandman::config`]. Where the
//! database, the trace and the socket live is in there and nowhere else; a flag
//! for each would be a second place to say it and a second place to get it
//! wrong. Naming another configuration says all three at once. There is no
//! fallback under the configuration either: a Sandman that finds none writes
//! the default one and stops, because there is nothing sensible to run before a
//! human has read it.
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

/// Where the state, the trace and the socket live, and how to open the first.
///
/// All three come from `config.toml` and none has a flag. Reaching a second
/// Sandman is `--config`, which names all three together and cannot name one
/// Sandman's socket beside another's database.
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

/// Read the argv and the configuration it names, and settle what beats what.
///
/// The configuration is read for every command, including the three that only
/// talk to a socket: where that socket is, is in it.
fn parse(
	argv: &[String],
) -> Result<(Command, Paths, Arc<Config>, sandman::log::Verbosity), String> {
	use clap::Parser;

	// `Error::exit` prints to the right stream (stdout for `--help` and
	// `--version`, stderr otherwise) and leaves with the matching code, so a
	// bad or absent argv never reaches the rest of this function.
	let cli = Cli::try_parse_from(argv).unwrap_or_else(|e| e.exit());

	let path = Config::path(cli.config).map_err(|e| e.to_string())?;
	let config = Config::load(&path).map_err(|e| e.to_string())?;

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

/// Build a whole Sandman: database, Event stream, logger, models, embedder,
/// tools, scheduler, Harness.
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

	let clock: Arc<dyn Clock> = Arc::new(SystemClock);
	let now = clock.now();

	let events = Arc::new(Events::new(1024));

	// With no terminal Channel nothing else is using stdout, and a trace
	// nobody can see is worse than one that scrolls past.
	let echo = if config.channels.stdio {
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

	Ok(Harness::new(
		store, events, scheduler, tools, clock, embedder, config,
	))
}

/// Make the directory a configured path lives in, so naming somewhere that does
/// not exist yet is a configuration choice rather than a failure to start.
fn make_room_for(path: &std::path::Path) -> Result<(), String> {
	let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
	else {
		return Ok(());
	};
	std::fs::create_dir_all(parent)
		.map_err(|e| format!("could not make {}: {e}", parent.display()))
}

/// The Channels the configuration opens, a Watcher, and a control socket,
/// running until the human leaves.
///
/// The Watcher is served whether or not its chat window is a Channel: watching
/// is not talking, and a Sandman with no Channel at all is still worth
/// following. It does mean `[channels].stdio = false` leaves no `/quit` — a
/// signal is then the way out, which is what it already is for `sandman run`.
async fn interactive(
	paths: Paths,
	config: Arc<Config>,
	verbosity: sandman::log::Verbosity,
) -> Result<(), String> {
	let harness = assemble(&paths, config.clone(), verbosity).await?;

	if config.channels.stdio {
		sandman::channels::stdio::attach(harness.clone())
			.await
			.map_err(|e| format!("could not open the terminal: {e}"))?;
	}

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
	config: Arc<Config>,
	verbosity: sandman::log::Verbosity,
) -> Result<(), String> {
	let harness = assemble(&paths, config, verbosity).await?;

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
	let (command, paths, config, verbosity) = match parse(&argv) {
		Ok(parsed) => parsed,
		Err(message) => {
			eprintln!("{message}");
			std::process::exit(1);
		},
	};

	let result = match command {
		Command::Interactive => interactive(paths, config, verbosity).await,
		Command::Run(args) => one_shot(args, paths, config, verbosity).await,
		Command::Task(args) => into_running(args, paths).await,
		Command::List { state, count } => list(state, count, paths).await,
		Command::Spend => spend(paths).await,
	};

	if let Err(message) = result {
		eprintln!("{message}");
		std::process::exit(1);
	}
}
