//! CLI shape for the sandman binary.
//!
//! What it is: the clap types that define `sandman <subcommand>` and its
//! flags. No I/O, no config access, no `exit` — just shape.
//!
//! Construct: `Cli::try_parse()` in `main`. Use: `main` matches `Cli.command`
//! and dispatches to `serve`, `task` or `bench_driver`.
//! Consumers: `main` (dispatch) and `paths`/`task`/`bench_driver` for the
//! flag types they act on.
//!
//! Rules: **subcommand required** — no default (previous `Interactive`/`None`
//! is now `Serve`). **global config flags** (`--config`, `--verbose`,
//! `--break-lock`) are on `Cli` so every command sees them.

use std::path::PathBuf;

/// What both Task-creating commands take.
pub struct TaskArgs {
	pub role: String,
	pub title: Option<String>,
	pub brief: String,
	pub in_seconds: Option<i64>,
	pub cron: Option<String>,
	pub priority: Option<String>,
}

/// Clap argv shape.
#[derive(clap::Parser)]
#[command(
	name = "sandman",
	about = "An agent swarm that coordinates through a shared queue",
	arg_required_else_help = true,
	subcommand_required = true
)]
pub struct Cli {
	#[command(subcommand)]
	pub command: Cmd,

	/// Which configuration to read. Defaults per [`Config::path`].
	#[arg(long, global = true)]
	pub config: Option<PathBuf>,
	/// Write every body in the trace out whole, instead of eliding it.
	#[arg(long, global = true)]
	pub verbose: bool,
	/// Start even though the database looks locked.
	#[arg(long, global = true)]
	pub break_lock: bool,
}

#[derive(clap::Subcommand)]
pub enum Cmd {
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
pub struct TaskFlags {
	#[arg(long)]
	pub role: String,
	#[arg(long)]
	pub title: Option<String>,
	#[arg(long)]
	pub brief: String,
	/// Seconds from now before this Task may run.
	#[arg(long = "in")]
	pub in_: Option<i64>,
	/// Cron expression this Task comes round on. Not with `--in`.
	#[arg(long)]
	pub cron: Option<String>,
	#[arg(long)]
	pub priority: Option<String>,
}

#[derive(clap::Args)]
pub struct ListFlags {
	/// Only Tasks in this state. Omit for every state.
	#[arg(long)]
	pub state: Option<String>,
	/// Limit how many to list. Omit for no limit.
	#[arg(long)]
	pub count: Option<usize>,
}

#[derive(clap::Args)]
pub struct BenchFlags {
	/// Say which cases there are, and run none of them.
	#[arg(long)]
	pub list: bool,
	/// Only these cases. Repeatable; omit for every case.
	#[arg(long = "case")]
	pub case: Vec<String>,
	/// Run each case this many times, for variance.
	#[arg(long, default_value_t = 1)]
	pub times: usize,
	/// One case at a time instead of all at once.
	#[arg(long)]
	pub serial: bool,
	/// Where the artifacts go.
	#[arg(long, default_value = "bench/runs")]
	pub out: std::path::PathBuf,
}

impl From<TaskFlags> for TaskArgs {
	fn from(f: TaskFlags) -> TaskArgs {
		TaskArgs {
			role: f.role,
			title: f.title,
			brief: f.brief,
			in_seconds: f.in_,
			cron: f.cron,
			priority: f.priority,
		}
	}
}
