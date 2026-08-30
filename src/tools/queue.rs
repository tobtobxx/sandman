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

use async_trait::async_trait;

use crate::domain::ToolSchema;
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;

use super::Tool;

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
		unimplemented!()
	}

	async fn call(
		&self,
		_ctx: &SessionCtx,
		_args: serde_json::Value,
	) -> String {
		unimplemented!()
	}
}

#[async_trait]
impl Tool for CancelTask {
	fn name(&self) -> ToolName {
		ToolName::CancelTask
	}

	fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
		unimplemented!()
	}

	/// Says what actually happened in words: which Tasks stopped, whether one of
	/// them was running, and — for a Task already completed or already cancelled
	/// — that there was nothing to stop.
	async fn call(
		&self,
		_ctx: &SessionCtx,
		_args: serde_json::Value,
	) -> String {
		unimplemented!()
	}
}
