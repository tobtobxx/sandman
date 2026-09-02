//! Suspending a Turn until another Task's answer exists.
//!
//! The only tool that holds a Turn. A Worker that needs a child's answer calls
//! this and blocks inside the tool call — no Session torn down, no field on the
//! Task, answer returns as this call's result in the same Turn so the Worker
//! remembers why it asked.
//!
//! Construct: `AwaitResult` implements [`crate::tools::Tool`], created in
//! `Registry::all` with no state; no constructor args.
//! Use: `call(ctx, {task_id}) -> String` parses the id, validates it exists,
//! then `waiters.wait(caller, task).await` until resolved.
//! Consumers: `session::turn` via `ToolRunner::run` (Worker only — `COMMS_SESSION_TOOLS`
//! never includes `AwaitResult`); `harness::complete_task`/`cancel_task` via
//! `waiters::resolve`/`resolve_held_by` wake it.
//!
//! Call trace: `turn → tools.run(await_result) → waiters.wait(caller, task)`
//! `→ harness.complete_task → waiters.resolve(task, answer) → return String → loop continues`.
//!
//! | Task outcome | What `call` returns |
//! | --- | --- |
//! | `Pending` | blocks, then resolved text |
//! | `Completed` (already finished) | resolved text at once |
//! | `Cancelled` | cancellation notice, so nobody hangs |
//!
//! Rules: **already finished resolves at once — outcome kept, not forgotten.**
//! **cancelled returns its notice in place of a Result.**
//! **any Task may be awaited by id, not only one this Session created.**
//! **suspension is the call itself — no link stored on the Task.**
//! **Tool only parses; `Waiters` owns the concurrency (`Pending`/`Resolved`, lock-then-await).**
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

	/// Parse `task_id`, validate it exists, then block until resolved.
	///
	/// Returns the Task's answer or its cancellation notice. Invalid or
	/// unknown id returns an error string.
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
		// Validate Task exists
		if ctx.harness.store.task(task).ok().flatten().is_none() {
			return ToolError::NoSuchTask(task.to_string()).to_string();
		}
		// Wait for answer
		ctx.harness.waiters.wait(ctx.id, task).await
	}
}
