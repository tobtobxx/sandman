//! Holding for another Session's answer.
//!
//! The tool that makes "nothing waits" survivable. A Worker that needs a child's
//! answer calls this and its turn suspends inside the call — no context is torn
//! down, nothing is registered on the Task, and when the answer exists it comes
//! back as this call's result. The Worker carries on remembering why it asked.
//!
//! A Task already finished resolves at once. A Task that is cancelled resolves
//! with the notice that stands in for a Result, so nobody hangs on work that
//! will never produce one.
//!
//! Defines: [`AwaitResult`].

use std::str::FromStr;

use async_trait::async_trait;

use crate::domain::{TaskId, ToolSchema};
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;

use super::{Tool, ToolError};

/// Block this turn until a Task completes, then return its answer.
pub struct AwaitResult;

#[async_trait]
impl Tool for AwaitResult {
	fn name(&self) -> ToolName {
		ToolName::AwaitResult
	}

	fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
		ToolSchema {
			name: self.name().to_string(),
			description: "Block until a Task completes, then return its \
			              answer. Use the id you got back from creating it."
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

	/// Reads the Task id, then hands off to [`crate::waiters::Waiters::wait`].
	///
	/// Any Task may be awaited by id, not only one this Session created.
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
		if ctx.harness.store.task(task).ok().flatten().is_none() {
			return ToolError::NoSuchTask(task.to_string()).to_string();
		}
		ctx.harness.waiters.wait(ctx.id, task).await
	}
}
