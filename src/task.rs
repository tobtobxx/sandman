//! Control-socket clients: task, list, spend.
//!
//! What it is: thin `control::send` wrappers that never open the database
//! themselves. `into_running` creates a Task in a live Sandman, `list` shows
//! its queue, `spend` shows its cost.
//!
//! Construct: callers build a `control::Request` and `send` it over
//! `Paths.socket`. Use: `main` on `Cmd::Task`/`List`/`Spend` → `load_config_
//! and_paths` for the socket path → these helpers → print reply.
//! Consumers: `main` only.
//!
//! Rules: **never a second writer** — direct DB access from these commands
//! would bypass `Store` and emit no `Event`. Every mutation goes through the
//! running harness via the socket.

use crate::cli::TaskArgs;
use crate::paths::Paths;

/// Send a Task into a running Sandman via the control socket.
pub async fn into_running(args: TaskArgs, paths: Paths) -> Result<(), String> {
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

/// List a running Sandman's queue via the control socket.
pub async fn list(
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

/// Print what a running Sandman has spent via the control socket.
pub async fn spend(paths: Paths) -> Result<(), String> {
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
