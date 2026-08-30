//! Running the queue. The `task_manager` Role's tools.
//!
//! `cancel_task` is the only write in Sandman that stops work rather than
//! starting it, and it is terminal: a pending Task never runs, a running one
//! ends at its Session's next decision point with no Result, and a repeating one
//! stops as a chain — otherwise a running occurrence would re-arm the next when
//! it finished.
//!
//! Whoever was waiting on a cancelled Task is told, so nothing hangs on work
//! that will never produce a Result.
//!
//! Defines: [`ListTasks`], [`CancelTask`].

use std::str::FromStr;

use async_trait::async_trait;

use crate::domain::{TaskId, TaskSummary, ToolSchema};
use crate::harness::CancelOutcome;
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;
use crate::store::ListFilter;

use super::{Tool, ToolError};

/// Enumerate the queue, newest first. Filters by state, or to repeating work.
pub struct ListTasks;

/// Stop a Task by id.
pub struct CancelTask;

#[async_trait]
impl Tool for ListTasks {
	fn name(&self) -> ToolName {
		ToolName::ListTasks
	}

	fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
		ToolSchema {
			name: self.name().to_string(),
			description: "Enumerate the queue, newest first.".to_string(),
			parameters: serde_json::json!({
				"type": "object",
				"properties": {
					"state": {
						"type": "string",
						"enum": ["pending", "running", "completed", "cancelled"],
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

	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		let state = match args.get("state").and_then(|v| v.as_str()) {
			None => None,
			Some("pending") => Some("pending"),
			Some("running") => Some("running"),
			Some("completed") => Some("completed"),
			Some("cancelled") => Some("cancelled"),
			Some(other) => {
				return ToolError::Rejected(format!(
					"`{other}` is not a Task state."
				))
				.to_string();
			},
		};
		let count = args
			.get("count")
			.and_then(|v| v.as_u64())
			.map(|n| n as usize);

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

	/// Says what actually happened in words: which Tasks stopped, whether one of
	/// them was running, and — for a Task already completed or already cancelled
	/// — that there was nothing to stop.
	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		let task = match args.get("task_id").and_then(|v| v.as_str()) {
			None => return ToolError::Missing { field: "task_id" }.to_string(),
			Some(s) => match TaskId::from_str(s) {
				Ok(id) => id,
				Err(_) => {
					return ToolError::NoSuchTask(s.to_string()).to_string()
				},
			},
		};

		match ctx.harness.cancel_task(task).await {
			Ok(CancelOutcome::NotFound) => {
				ToolError::NoSuchTask(task.to_string()).to_string()
			},
			Ok(CancelOutcome::Completed) => {
				format!("{task} already completed. Nothing to stop.")
			},
			Ok(CancelOutcome::Already) => {
				format!("{task} was already cancelled.")
			},
			Ok(CancelOutcome::Cancelled { ids, running }) => {
				let ids = ids
					.iter()
					.map(TaskId::to_string)
					.collect::<Vec<_>>()
					.join(", ");
				if running {
					format!(
						"Cancelled {ids}. One of them was running; its Session \
						 will stop at its next decision point."
					)
				} else {
					format!("Cancelled {ids}.")
				}
			},
			Err(e) => format!("Error: {e}"),
		}
	}
}

/// One line per Task: id, state, Role and Title.
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
