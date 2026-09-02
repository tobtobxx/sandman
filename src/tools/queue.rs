//! Operating the queue. The `TaskManager` Role's two tools.
//!
//! `list_tasks` reads, `cancel_task` is the only write in Sandman that stops
//! work rather than starting it.
//!
//! Construct: `ListTasks`/`CancelTask` implement [`Tool`], built in
//! `Registry::all` with no state; named via `ToolName::ListTasks`/
//! `ToolName::CancelTask`.
//! Use: `Tool::call(ctx, args) -> String` — `ListTasks` parses optional
//! `state`/`count` → `Store::list_tasks(ListFilter)` → one line per hit;
//! `CancelTask` parses `task_id` → `Harness::cancel_task` → wording per
//! `CancelOutcome`.
//! Consumers: `roles.rs` assigns both to `TaskManager` (with `SearchTasks`/
//! `CreateTaskFull`); `Store` owns list filtering, `Harness` owns cancel policy
//! (`cancel_tasks` → `waiters::resolve`); bench replaces
//! `ToolRunner` to observe without touching the queue.
//!
//! Call trace: `turn → scheduler.request → tools.run(list_tasks) → store.list_tasks → String → loop`
//! and `turn → tools.run(cancel_task) → harness.cancel_task → store.cancel_tasks → waiters.resolve → String`.
//!
//! | `CancelOutcome` | `CancelTask` replies |
//! | --- | --- |
//! | `NotFound` | no such Task |
//! | `Completed` | already completed, nothing to stop |
//! | `Already` | already cancelled |
//! | `Cancelled{running}` | it stopped; notes if its Session halts at next decision |
//!
//! Rules: **cancel is terminal — no Result, and reaches the named Task alone.**
//! **pending never runs, running stops at next decision with no Result.**
//! **waiters told so `await_result` never hangs.**
//! **list order is newest first; state enum from `TaskStateName` so impossible states cannot be named.**
//! **list is read-only via `Store`, cancel is write via `Harness` — only tool that stops work.**
//!
//! Defines: [`ListTasks`], [`CancelTask`].

use std::str::FromStr;

use async_trait::async_trait;
use strum::{IntoDiscriminant, VariantArray};

use crate::domain::{TaskId, TaskStateName, TaskSummary, ToolSchema};
use crate::harness::CancelOutcome;
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;
use crate::store::ListFilter;

use super::{Tool, ToolError};

/// Enumerate the queue, newest first. Filters by state or count.
pub struct ListTasks;

/// Stop a Task by id. Terminates its repeating chain and releases waiters.
pub struct CancelTask;

#[async_trait]
impl Tool for ListTasks {
	fn name(&self) -> ToolName {
		ToolName::ListTasks
	}

	/// Build schema with state enum from `TaskStateName` and optional count.
	///
	/// State enumerates every valid Task state; no impossible value.
	fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
		// Collect states
		let states: Vec<&'static str> =
			TaskStateName::VARIANTS.iter().map(Into::into).collect();
		ToolSchema {
			name: self.name().to_string(),
			description: "Enumerate the queue, newest first.".to_string(),
			parameters: serde_json::json!({
				"type": "object",
				"properties": {
					"state": {
						"type": "string",
						"enum": states,
						"description": "Only Tasks in this state. Omit for every state.",
					},
					"count": {
						"type": "integer",
						"description": "Limit how many to return. Omit for no limit.",
					},
				},
				"required": [],
			}),
		}
	}

	/// Parse state and count, list matching Tasks and render them.
	///
	/// Returns an error sentence on invalid state.
	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		// Parse state filter
		let state = match args.get("state").and_then(|v| v.as_str()) {
			None => None,
			Some(given) => match given.parse() {
				Ok(state) => Some(state),
				Err(_) => {
					return ToolError::Rejected(format!(
						"`{given}` is not a Task state."
					))
					.to_string();
				},
			},
		};
		// Parse count limit
		let count = args
			.get("count")
			.and_then(|v| v.as_u64())
			.map(|n| n as usize);

		// List and render
		match ctx.harness.store.list_tasks(ListFilter { state, count }) {
			Ok(tasks) => render_list(&tasks),
			Err(e) => format!("Error: {e}"),
		}
	}
}

#[async_trait]
impl Tool for CancelTask {
	fn name(&self) -> ToolName {
		ToolName::CancelTask
	}

	/// Build schema requiring one `task_id` string.
	///
	/// Same id form as `list_tasks` output.
	fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
		ToolSchema {
			name: self.name().to_string(),
			description: "Stop a Task by id. Pending, running, or a whole \
			              repeating chain."
				.to_string(),
			parameters: serde_json::json!({
				"type": "object",
				"properties": {
					"task_id": {
						"type": "string",
						"description": "The Task's id, e.g. \"t-03\".",
					},
				},
				"required": ["task_id"],
			}),
		}
	}

	/// Parse `task_id`, cancel its chain and report outcome.
	///
	/// Returns Already/Completed/NotFound wording when nothing stops.
	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		// Parse task id
		let task = match args.get("task_id").and_then(|v| v.as_str()) {
			None => return ToolError::Missing { field: "task_id" }.to_string(),
			Some(s) => match TaskId::from_str(s) {
				Ok(id) => id,
				Err(_) => {
					return ToolError::NoSuchTask(s.to_string()).to_string();
				},
			},
		};

		// Cancel task
		match ctx.harness.cancel_task(task).await {
			// - Not found
			Ok(CancelOutcome::NotFound) => {
				ToolError::NoSuchTask(task.to_string()).to_string()
			},
			// - Already completed
			Ok(CancelOutcome::Completed) => {
				format!("{task} already completed. Nothing to stop.")
			},
			// - Already cancelled
			Ok(CancelOutcome::Already) => {
				format!("{task} was already cancelled.")
			},
			// - Cancelled
			Ok(CancelOutcome::Cancelled { running }) => {
				if running {
					format!(
						"Cancelled {task}. It was running; its Session will \
						 stop at its next decision point."
					)
				} else {
					format!("Cancelled {task}.")
				}
			},
			// - Store error
			Err(e) => format!("Error: {e}"),
		}
	}
}

/// Format Tasks as one line per Task: id, state, Role and Title.
///
/// Returns "No Tasks match." when empty.
fn render_list(tasks: &[TaskSummary]) -> String {
	if tasks.is_empty() {
		return "No Tasks match.".to_string();
	}
	tasks
		.iter()
		.map(|t| {
			format!(
				"{} [{}] {}: {}",
				t.id,
				t.state.discriminant(),
				t.role,
				t.title
			)
		})
		.collect::<Vec<_>>()
		.join("\n")
}
