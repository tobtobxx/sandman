//! Process boundary and wiring. Only place that builds a Harness.
//!
//! What it is: argv and config in, Harness or control-socket request out.
//! Wiring lives here and only here — which `Model`, `ToolRunner`, `Clock` and
//! `Embedder` is chosen here; everything below receives `Arc`s. Bench reuses
//! the same Harness with different pieces through these seams.
//!
//! Use: `main` matches `Cmd` and delegates:
//!
//! | Command | Builds Store | Talks to | Drives |
//! |---|---|---|---|
//! | Serve | yes | Channels + web UI + socket | `harness.run` until quit |
//! | Bench | no (private per run) | — | runs `Case::run` per case |
//! | Task / List / Spend | no | `control::send` to running Harness | print reply |
//!
//! Call trace (serve):
//! ```text
//! main → paths::load_config_and_paths → serve::interactive → serve::assemble
//!      → attach stdio/web → spawn web::serve + control::serve
//!      → harness.run → wind_down → end_run → remove socket
//! ```
//!
//! Rules:
//! - **One Sandman per database** — second `sandman serve` on same db is refused by `db::Lock`; `task`/`list`/`spend` never open the DB.
//! - **No second writer** — cross-process entry is `control::Request` via socket; it goes through `Store` and emits `Event`s.
//! - **No flag for a path** — db, log, socket come from `config.toml` together, selected by `--config`.
//! - **No fallback config** — missing file writes default and stops.
//! - **No signal handler** — Ctrl+C aborts; half-written state is cleaned by `Store::open` on next start; graceful exit is `/quit`.

mod bench_driver;
mod cli;
mod paths;
mod serve;
mod task;

use clap::Parser;

use cli::{Cli, Cmd};

#[tokio::main]
async fn main() {
	let cli = Cli::try_parse().unwrap_or_else(|e| e.exit());

	let Cli { command, config: config_flag, verbose, break_lock } = cli;
	let load = || paths::load_config_and_paths(config_flag.clone(), break_lock, verbose);
	let result = match command {
		Cmd::Serve => {
			let (paths, config, verbosity) = match load() {
				Ok(v) => v,
				Err(message) => {
					eprintln!("{message}");
					std::process::exit(1);
				},
			};
			serve::interactive(paths, config, verbosity).await
		},
		Cmd::Task(flags) => {
			let (paths, _, _) = match load() {
				Ok(v) => v,
				Err(message) => {
					eprintln!("{message}");
					std::process::exit(1);
				},
			};
			task::into_running(flags.into(), paths).await
		},
		Cmd::List(flags) => {
			let (paths, _, _) = match load() {
				Ok(v) => v,
				Err(message) => {
					eprintln!("{message}");
					std::process::exit(1);
				},
			};
			task::list(flags.state, flags.count, paths).await
		},
		Cmd::Spend => {
			let (paths, _, _) = match load() {
				Ok(v) => v,
				Err(message) => {
					eprintln!("{message}");
					std::process::exit(1);
				},
			};
			task::spend(paths).await
		},
		Cmd::Bench(flags) => bench_driver::bench(flags, config_flag).await,
	};

	if let Err(message) = result {
		eprintln!("{message}");
		std::process::exit(1);
	}
}
