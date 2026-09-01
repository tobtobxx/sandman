//! Paths and config loading for commands that touch the filesystem.
//!
//! What it is: `Paths` (db/log/socket from `config.toml`) and the two helpers
//! that keep `main` thin: `load_config_and_paths` reads the config file
//! referenced by `--config` and `make_room_for` ensures a parent dir exists.
//!
//! Construct: `load_config_and_paths(flag, break_lock, verbose)` → `(Paths,
//! Arc<Config>, Verbosity)`. Use: `main` calls it for `serve`/`task`/`list`/
//! `spend`; `serve::assemble` calls `make_room_for` before opening files.
//! Consumers: `main` (dispatch glue) and `serve`.
//!
//! Rules: **no flag for a path** — db/log/socket always come together from the
//! chosen `config.toml`. **no fallback config** inside this module: missing file
//! is `Err(Written)` from `Config::load`, which `main` surfaces.

use std::path::PathBuf;
use std::sync::Arc;

use sandman::config::Config;

/// Where the state, trace and socket live and how to open them.
///
/// All three come from `config.toml` together, selected by `--config`.
/// `break_lock` clears a stale `db::Lock` before opening.
pub struct Paths {
	pub db: std::path::PathBuf,
	pub log: std::path::PathBuf,
	pub socket: std::path::PathBuf,
	/// Clear a stale lock before opening. See `--break-lock`.
	pub break_lock: bool,
}

/// Load config and paths for commands that need them (serve, task, list, spend).
pub fn load_config_and_paths(
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

/// Ensure parent directory of a configured path exists.
pub fn make_room_for(path: &std::path::Path) -> Result<(), String> {
	let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
	else {
		return Ok(());
	};
	std::fs::create_dir_all(parent)
		.map_err(|e| format!("could not make {}: {e}", parent.display()))
}
