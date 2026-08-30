//! Putting work on the queue. The only route between agents.
//!
//! Three tools, one enqueue path. They differ only in what they let the caller
//! choose, and the split is the point: the common case — hand a piece of work to
//! planning — is free of the Role and schedule arguments a Worker can get wrong,
//! and a Role that should not be choosing Roles is given only the narrow tool.
//!
//! None of them waits. A Worker that wants the answer calls `await_result` with
//! the id it got back, when it is ready for it. None of them subscribes a
//! Worker either: only a Comms Session subscribes, because it cannot block on a
//! tool call and so must be handed the answer as mail instead.
//!
//! Defines: [`CreateTask`], [`CreateResearchTask`], [`CreateTaskFull`].

use async_trait::async_trait;

use crate::domain::{TaskId, ToolSchema};
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;

use super::Tool;

/// Shared wording, so three schemas cannot describe the same argument three
/// ways.
pub const TITLE_DESC: &str =
	"One line describing the Task, so a human can scan it.";
pub const BRIEF_DESC: &str =
	"The full instructions. The Worker sees nothing else, so include \
     every fact it needs. Write it for someone with no context.";

/// Enqueue a planning Task. No Role and no timing to choose.
pub struct CreateTask;

/// Enqueue a research Task, so a Worker can have something looked up without
/// leaving its own line of work.
pub struct CreateResearchTask;

/// Enqueue a Task, choosing its Role, its timing and its priority.
pub struct CreateTaskFull;

/// The one path all three take.
///
/// Validates the Title and the Brief, resolves the Role, works out the
/// [`crate::domain::Schedule`], records who asked, and creates the Task. Comes
/// back with the id and a sentence telling the caller what to do next — or with
/// what was wrong, in words the model can act on.
async fn enqueue(
	_ctx: &SessionCtx,
	_role: crate::roles::RoleName,
	_args: serde_json::Value,
	_allow_schedule: bool,
) -> Result<TaskId, super::ToolError> {
	unimplemented!()
}

/// What a caller is told once its Task exists.
///
/// A Worker is reminded that it can call `await_result`; a Comms Session is told
/// the answer will reach it when it is ready, because it has no such tool.
fn created_reply(_ctx: &SessionCtx, _id: TaskId) -> String {
	unimplemented!()
}

#[async_trait]
impl Tool for CreateTask {
	fn name(&self) -> ToolName {
		ToolName::CreateTask
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
impl Tool for CreateResearchTask {
	fn name(&self) -> ToolName {
		ToolName::CreateResearchTask
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
impl Tool for CreateTaskFull {
	fn name(&self) -> ToolName {
		ToolName::CreateTaskFull
	}

	/// Carries `role`, `run_at_seconds`, `repeat_seconds` and `priority` beyond
	/// the common two. The `role` enum is built from [`crate::roles::ROLE_NAMES`],
	/// so it cannot name a Role that does not exist.
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
