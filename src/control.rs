//! The control socket: how another process puts work into a running Sandman.
//!
//! A Channel is a two-way connection to a human. Cron, an RSS script, a mail
//! watcher and a shell one-liner are none of those — they have nothing to say
//! back to and nothing to be told. They get this instead: one line of JSON in,
//! one line out, and the connection closes.
//!
//! It is a **socket rather than a second writer to the database**, and that is
//! the whole design decision here. A process inserting a row directly would
//! bypass the Store, so no [`crate::event::Event`] would be emitted for it — the
//! log, the Watcher and anything replaying the stream would all have the same
//! blind spot. One writer is the property the Store was shaped around, and this
//! keeps it.
//!
//! A Unix domain socket with restrictive permissions, never a TCP port: this is a
//! write path into a running swarm.
//!
//! Defines: [`Request`], [`Response`], [`serve`], [`send`], [`socket_path`].

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
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

/// What another process may ask for.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Request {
	/// Put a Task on the queue. Recorded as
	/// [`crate::domain::Creator::Control`], so where it came from is not lost.
	CreateTask {
		role: String,
		title: String,
		brief: String,
		/// Seconds to wait before it may run.
		run_at_seconds: Option<i64>,
		/// Seconds between occurrences.
		repeat_seconds: Option<i64>,
		priority: Option<String>,
	},
	ListTasks {
		state: Option<String>,
		count: Option<usize>,
	},
	/// What the running Sandman has spent.
	Spend,
}

/// What it gets back.
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
	/// Everything that went wrong, said in one sentence.
	Error {
		message: String,
	},
}

/// A Task as the socket reports it: flat, so a shell script can read it without
/// knowing the domain.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskLine {
	pub id: String,
	pub title: String,
	pub role: String,
	pub state: String,
	pub not_before: Option<i64>,
	pub created_at: i64,
}

/// Where the socket lives: `$SANDMAN_SOCKET`, else `$XDG_RUNTIME_DIR/sandman.sock`,
/// else a path beside the database.
pub fn socket_path() -> PathBuf {
	if let Ok(path) = std::env::var("SANDMAN_SOCKET") {
		return PathBuf::from(path);
	}
	if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
		return PathBuf::from(dir).join("sandman.sock");
	}
	PathBuf::from("sandman.sock")
}

/// Listen, and answer requests until the Harness stops.
///
/// Removes a stale socket file left by a killed process, and creates the new one
/// readable and writable only by its owner.
pub async fn serve(harness: Arc<Harness>, path: &Path) -> std::io::Result<()> {
	if UnixStream::connect(path).await.is_ok() {
		return Err(std::io::Error::new(
			std::io::ErrorKind::AddrInUse,
			format!("a Sandman is already listening on {}", path.display()),
		));
	}
	let _ = std::fs::remove_file(path);
	if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
		std::fs::create_dir_all(parent)?;
	}

	let listener = UnixListener::bind(path)?;
	std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;

	loop {
		let (stream, _) = listener.accept().await?;
		let harness = harness.clone();
		tokio::spawn(async move {
			let _ = handle_connection(&harness, stream).await;
		});
	}
}

/// One connection: one request line in, one response line out, then close.
async fn handle_connection(
	harness: &Arc<Harness>,
	stream: UnixStream,
) -> std::io::Result<()> {
	let (reader, mut writer) = stream.into_split();
	let mut lines = BufReader::new(reader).lines();

	let Some(line) = lines.next_line().await? else {
		return Ok(());
	};
	let response = match serde_json::from_str::<Request>(&line) {
		Ok(request) => handle(harness, request).await,
		Err(e) => Response::Error { message: format!("bad request: {e}") },
	};

	let mut line = serde_json::to_string(&response)
		.unwrap_or_else(|e| format!("could not encode the response: {e}"));
	line.push('\n');
	writer.write_all(line.as_bytes()).await
}

/// Send one request from the client side and read the answer.
///
/// What `sandman task` does. A missing socket means no Sandman is running, and
/// that is the error the caller gets.
pub async fn send(path: &Path, request: &Request) -> std::io::Result<Response> {
	let stream = UnixStream::connect(path).await.map_err(|e| {
		std::io::Error::new(
			e.kind(),
			format!("no Sandman is listening on {}: {e}", path.display()),
		)
	})?;
	let (reader, mut writer) = stream.into_split();

	let mut line = serde_json::to_string(request)
		.map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
	line.push('\n');
	writer.write_all(line.as_bytes()).await?;
	writer.shutdown().await?;

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

/// Turn one request into what the Harness does about it.
async fn handle(harness: &Arc<Harness>, request: Request) -> Response {
	match request {
		Request::CreateTask {
			role,
			title,
			brief,
			run_at_seconds,
			repeat_seconds,
			priority,
		} => {
			let Ok(role) = role.parse::<RoleName>() else {
				return Response::Error {
					message: format!("`{role}` is not a Role."),
				};
			};
			let title = match Title::try_from(title) {
				Ok(title) => title,
				Err(e) => return Response::Error { message: e.to_string() },
			};
			let brief = match Brief::try_from(brief) {
				Ok(brief) => brief,
				Err(e) => return Response::Error { message: e.to_string() },
			};
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
			let schedule = Schedule::from_offsets(
				run_at_seconds,
				repeat_seconds,
				harness.now(),
			);
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

		Request::ListTasks { state, count } => {
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
			match harness.store.list_tasks(ListFilter { state, count }) {
				Ok(tasks) => Response::Tasks {
					tasks: tasks.into_iter().map(TaskLine::from).collect(),
				},
				Err(e) => Response::Error { message: e.to_string() },
			}
		},

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
			not_before: t.schedule.not_before(t.created_at).map(|ts| ts.0),
			created_at: t.created_at.0,
		}
	}
}

impl From<TaskId> for Response {
	fn from(id: TaskId) -> Response {
		Response::Created { id: id.to_string() }
	}
}
