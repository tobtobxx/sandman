//! Control socket: one-shot JSON entry for non-human callers.
//!
//! A `Channel` is a two-way human conversation — cron, RSS watchers, mail
//! scripts and shell one-liners have nothing to say back and get this instead:
//! one JSON line in, one line out, then close.
//!
//! Construct: `serve(harness, path)` binds a Unix socket at
//! `[sandman].control_socket`; `send(path, request)` is the client — what
//! `sandman task` calls.
//! Use: `Request` → `handle` → `Response` through `Harness`/`Store`; every
//! failure becomes `Response::Error` with a single message.
//! Consumers: the `sandman` binary spawns `serve`; external processes and
//! shell scripts drive `send`; `Harness` remains the sole `Store` writer so
//! every change emits an `Event`.
//!
//! | `Request` | `Harness`/`Store` call | `Response` |
//! |---|---|---|
//! | `CreateTask` | parse `RoleName`/`Title`/`Brief`/`Schedule::parse` → `Harness::create_task` | `Created` |
//! | `ListTasks` | parse `TaskStateName` → `Store::list_tasks` | `Tasks` |
//! | `Spend` | `Harness::spend` | `Spent` |
//! | bad JSON / bad fields | — | `Error` |
//!
//! Rules: never a second database writer — a direct insert would emit no
//! `Event` and blind the log, Watcher and replay.
//! Unix socket, `0o600`, never TCP; stale file removed, live socket →
//! `AddrInUse`. One request per connection; missing socket means no Sandman is
//! running.
//!
//! Defines: [`Request`], [`Response`], [`TaskLine`], [`serve`], [`send`].

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use strum::IntoDiscriminant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::domain::{
	Brief, Creator, NewTask, Schedule, Spend, TaskId, TaskPriority,
	TaskSummary, Title,
};
use crate::harness::Harness;
use crate::roles::RoleName;
use crate::store::ListFilter;

/// What a client may ask a running Sandman to do.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Request {
	/// Enqueue a Task as `Creator::Control`.
	CreateTask {
		role: String,
		title: String,
		brief: String,
		/// Seconds to wait before it may run.
		in_seconds: Option<i64>,
		/// Cron expression it comes round on. Not with `in_seconds`.
		cron: Option<String>,
		priority: Option<String>,
	},
	ListTasks {
		state: Option<String>,
		count: Option<usize>,
	},
	/// Return current spend for this run.
	Spend,
}

/// What the control socket returns.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Response {
	Created {
		id: String,
	},
	Tasks {
		tasks: Vec<TaskLine>,
	},
	Spent {
		calls: u32,
		tokens: u64,
		cost: String,
	},
	/// Single-message failure for any bad input or store error.
	Error {
		message: String,
	},
}

/// Flat task summary for shell callers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskLine {
	pub id: String,
	pub title: String,
	pub role: String,
	pub state: String,
	pub not_before: Option<i64>,
	pub created_at: i64,
}

/// Listen on `path` and answer requests until the harness stops.
///
/// Binds a Unix socket and spawns one task per connection.
/// Returns `AddrInUse` if a Sandman is already listening.
pub async fn serve(harness: Arc<Harness>, path: &Path) -> std::io::Result<()> {
	// Reject if already listening
	if UnixStream::connect(path).await.is_ok() {
		return Err(std::io::Error::new(
			std::io::ErrorKind::AddrInUse,
			format!("a Sandman is already listening on {}", path.display()),
		));
	}
	// Clean stale socket
	let _ = std::fs::remove_file(path);
	if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
		std::fs::create_dir_all(parent)?;
	}

	// Bind with restricted permissions
	let listener = UnixListener::bind(path)?;
	std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;

	// Accept loop
	loop {
		let (stream, _) = listener.accept().await?;
		let harness = harness.clone();
		tokio::spawn(async move {
			let _ = handle_connection(&harness, stream).await;
		});
	}
}

/// Handle one connection.
///
/// Reads one JSON line, dispatches it, and writes one response line.
async fn handle_connection(
	harness: &Arc<Harness>,
	stream: UnixStream,
) -> std::io::Result<()> {
	// Read request line
	let (reader, mut writer) = stream.into_split();
	let mut lines = BufReader::new(reader).lines();

	let Some(line) = lines.next_line().await? else {
		return Ok(());
	};
	// Dispatch request
	let response = match serde_json::from_str::<Request>(&line) {
		Ok(request) => handle(harness, request).await,
		Err(e) => Response::Error { message: format!("bad request: {e}") },
	};

	// Encode and reply
	let mut line = serde_json::to_string(&response)
		.unwrap_or_else(|e| format!("could not encode the response: {e}"));
	line.push('\n');
	writer.write_all(line.as_bytes()).await
}

/// Send one request and read the response.
///
/// Connects to `path`, writes one JSON line, and decodes the reply.
/// Fails if no Sandman is listening.
pub async fn send(path: &Path, request: &Request) -> std::io::Result<Response> {
	// Connect
	let stream = UnixStream::connect(path).await.map_err(|e| {
		std::io::Error::new(
			e.kind(),
			format!("no Sandman is listening on {}: {e}", path.display()),
		)
	})?;
	let (reader, mut writer) = stream.into_split();

	// Send request
	let mut line = serde_json::to_string(request)
		.map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
	line.push('\n');
	writer.write_all(line.as_bytes()).await?;
	writer.shutdown().await?;

	// Read response
	let mut lines = BufReader::new(reader).lines();
	match lines.next_line().await? {
		Some(line) => serde_json::from_str(&line).map_err(|e| {
			std::io::Error::new(std::io::ErrorKind::InvalidData, e)
		}),
		None => Err(std::io::Error::new(
			std::io::ErrorKind::UnexpectedEof,
			"the connection closed with no response",
		)),
	}
}

/// Dispatch one request to the harness.
///
/// Validates fields, calls `Harness`/`Store`, and maps every failure to
/// `Response::Error`.
async fn handle(harness: &Arc<Harness>, request: Request) -> Response {
	match request {
		// Create task - validate and enqueue
		Request::CreateTask {
			role,
			title,
			brief,
			in_seconds,
			cron,
			priority,
		} => {
			// Validate role
			let Ok(role) = role.parse::<RoleName>() else {
				return Response::Error {
					message: format!("`{role}` is not a Role."),
				};
			};
			// Validate title
			let title = match Title::try_from(title) {
				Ok(title) => title,
				Err(e) => return Response::Error { message: e.to_string() },
			};
			// Validate brief
			let brief = match Brief::try_from(brief) {
				Ok(brief) => brief,
				Err(e) => return Response::Error { message: e.to_string() },
			};
			// Validate priority
			let priority = match priority.as_deref() {
				None => TaskPriority::default(),
				Some(given) => match given.parse() {
					Ok(p) => p,
					Err(_) => {
						return Response::Error {
							message: format!(
								"`{given}` is not a priority. Use high, \
								 normal or low."
							),
						};
					},
				},
			};
			// Build schedule and create task
			let schedule = match Schedule::parse(
				in_seconds,
				cron.as_deref(),
				harness.now(),
			) {
				Ok(schedule) => schedule,
				Err(e) => return Response::Error { message: e.to_string() },
			};
			let new = NewTask {
				title,
				brief,
				role,
				schedule,
				priority,
				created_by: Creator::Control,
			};
			match harness.create_task(new) {
				Ok(id) => id.into(),
				Err(e) => Response::Error { message: e.to_string() },
			}
		},

		// List tasks - validate filter and query
		Request::ListTasks { state, count } => {
			// Validate state filter
			let state = match &state {
				None => None,
				Some(given) => match given.parse() {
					Ok(state) => Some(state),
					Err(_) => {
						return Response::Error {
							message: format!("`{given}` is not a Task state."),
						};
					},
				},
			};
			// Query store
			match harness.store.list_tasks(ListFilter { state, count }) {
				Ok(tasks) => Response::Tasks {
					tasks: tasks.into_iter().map(TaskLine::from).collect(),
				},
				Err(e) => Response::Error { message: e.to_string() },
			}
		},

		// Spend - sum completed calls
		Request::Spend => match harness.spend() {
			Ok(spend) => spend.into(),
			Err(e) => Response::Error { message: e.to_string() },
		},
	}
}

impl From<Spend> for Response {
	fn from(s: Spend) -> Response {
		Response::Spent {
			calls: s.calls,
			tokens: s.tokens,
			cost: s.cost.to_string(),
		}
	}
}

impl From<TaskSummary> for TaskLine {
	fn from(t: TaskSummary) -> TaskLine {
		TaskLine {
			id: t.id.to_string(),
			title: t.title.to_string(),
			role: t.role.to_string(),
			state: t.state.discriminant().to_string(),
			not_before: t.schedule.not_before().map(|ts| ts.0),
			created_at: t.created_at.0,
		}
	}
}

impl From<TaskId> for Response {
	fn from(id: TaskId) -> Response {
		Response::Created { id: id.to_string() }
	}
}
