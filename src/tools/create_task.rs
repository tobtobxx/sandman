//! Putting work on the queue. The only route between agents.
//!
//! Three tools, one enqueue path. They differ only in what they let the caller
//! choose, and the split is the point: the common case — hand a piece of work to
//! planning — is free of the Role and schedule arguments a Worker can get wrong,
//! and a Role that should not be choosing Roles is given only the narrow tool.
//!
//! None of them waits. A Worker that wants the answer calls `await_result` with
//! the id it got back, when it is ready for it. None of them subscribes anyone
//! either: the Store reads the subscriber off the calling Session, so a Comms
//! Session — which cannot block on a tool call, and must be handed the answer as
//! mail instead — is subscribed whether or not a tool remembered to ask.
//!
//! Defines: [`CreateTask`], [`CreateResearchTask`], [`CreateTaskFull`].

use async_trait::async_trait;
use strum::VariantArray;

use crate::domain::{
	Brief, Creator, NewTask, Schedule, TaskId, TaskPriority, Title, ToolSchema,
};
use crate::roles::{RoleName, SchemaCtx, ToolName};
use crate::session::SessionCtx;

use super::{Tool, ToolError};

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

/// Every field a create-task tool call might carry, read off the arguments
/// but not yet checked. Not every tool accepts every field — `create_task`
/// and `create_research_task` only ever look at `title` and `brief` — so
/// nothing here fails on its own; each tool below validates and fills in
/// what it needs.
struct ParsedArgs {
	title: Option<String>,
	brief: Option<String>,
	role: Option<String>,
	run_at_seconds: Option<i64>,
	repeat_seconds: Option<i64>,
	priority: Option<String>,
}

/// Read every field a create-task tool might carry off the raw arguments.
/// Nothing is checked here — that is each tool's own job.
fn parse_args(args: &serde_json::Value) -> ParsedArgs {
	let str_field = |name: &str| {
		args.get(name).and_then(|v| v.as_str()).map(str::to_string)
	};
	ParsedArgs {
		title: str_field("title"),
		brief: str_field("brief"),
		role: str_field("role"),
		run_at_seconds: args.get("run_at_seconds").and_then(|v| v.as_i64()),
		repeat_seconds: args.get("repeat_seconds").and_then(|v| v.as_i64()),
		priority: str_field("priority"),
	}
}

/// The Title, or why it cannot be one.
fn require_title(title: Option<String>) -> Result<Title, ToolError> {
	let title = title.ok_or(ToolError::Missing { field: "title" })?;
	Title::try_from(title).map_err(|e| ToolError::Rejected(e.to_string()))
}

/// The Brief, or why it cannot be one.
fn require_brief(brief: Option<String>) -> Result<Brief, ToolError> {
	let brief = brief.ok_or(ToolError::Missing { field: "brief" })?;
	Brief::try_from(brief).map_err(|e| ToolError::Rejected(e.to_string()))
}

/// `create_task_full`'s Role, or why the one given is not one.
fn require_role(role: Option<String>) -> Result<RoleName, ToolError> {
	let given = role.ok_or(ToolError::Missing { field: "role" })?;
	given.parse().map_err(|_| ToolError::NoSuchRole { given })
}

/// `create_task_full`'s Priority, or why the one given is not one. Defaults
/// to normal.
fn priority_from(priority: Option<String>) -> Result<TaskPriority, ToolError> {
	match priority.as_deref() {
		None => Ok(TaskPriority::default()),
		Some(other) => other.parse().map_err(|_| {
			ToolError::Rejected(format!(
				"`{other}` is not a priority. Use high, normal or low."
			))
		}),
	}
}

/// What a caller is told once its Task exists.
///
/// A Worker is reminded that it can call `await_result`; a Comms Session is told
/// the answer will reach it when it is ready, because it has no such tool. The
/// branch that picks the wording and the subscription the Store derives read the
/// same Session — say the two differently and this promise goes unkept.
fn created_reply(ctx: &SessionCtx, id: TaskId) -> String {
	let is_worker = ctx
		.store
		.session(ctx.id)
		.ok()
		.flatten()
		.map(|s| matches!(s.kind, crate::domain::SessionKind::Worker { .. }))
		.unwrap_or(true);
	if is_worker {
		format!(
			"Created {id}. Call await_result with this id when you are ready \
			 for its answer."
		)
	} else {
		format!(
			"Created {id}. Its answer will reach you here when it is ready."
		)
	}
}

#[async_trait]
impl Tool for CreateTask {
	fn name(&self) -> ToolName {
		ToolName::CreateTask
	}

	fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
		ToolSchema {
			name: self.name().to_string(),
			description: "Enqueue a planning Task: the common case, with no \
			              Role or timing to choose."
				.to_string(),
			parameters: serde_json::json!({
				"type": "object",
				"properties": {
					"title": { "type": "string", "description": TITLE_DESC },
					"brief": { "type": "string", "description": BRIEF_DESC },
				},
				"required": ["title", "brief"],
			}),
		}
	}

	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		let parsed = parse_args(&args);
		let title = match require_title(parsed.title) {
			Ok(t) => t,
			Err(e) => return e.to_string(),
		};
		let brief = match require_brief(parsed.brief) {
			Ok(b) => b,
			Err(e) => return e.to_string(),
		};

		let new = NewTask {
			title,
			brief,
			role: RoleName::Planning,
			schedule: Schedule::Now,
			priority: TaskPriority::default(),
			created_by: Creator::Session(ctx.id),
		};
		match ctx.harness.create_task(new) {
			Ok(id) => created_reply(ctx, id),
			Err(e) => format!("Error: {e}"),
		}
	}
}

#[async_trait]
impl Tool for CreateResearchTask {
	fn name(&self) -> ToolName {
		ToolName::CreateResearchTask
	}

	fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
		ToolSchema {
			name: self.name().to_string(),
			description: "Enqueue a research Task, so a fact can be checked \
			              without leaving your own line of work."
				.to_string(),
			parameters: serde_json::json!({
				"type": "object",
				"properties": {
					"title": { "type": "string", "description": TITLE_DESC },
					"brief": { "type": "string", "description": BRIEF_DESC },
				},
				"required": ["title", "brief"],
			}),
		}
	}

	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		let parsed = parse_args(&args);
		let title = match require_title(parsed.title) {
			Ok(t) => t,
			Err(e) => return e.to_string(),
		};
		let brief = match require_brief(parsed.brief) {
			Ok(b) => b,
			Err(e) => return e.to_string(),
		};

		let new = NewTask {
			title,
			brief,
			role: RoleName::Research,
			schedule: Schedule::Now,
			priority: TaskPriority::default(),
			created_by: Creator::Session(ctx.id),
		};
		match ctx.harness.create_task(new) {
			Ok(id) => created_reply(ctx, id),
			Err(e) => format!("Error: {e}"),
		}
	}
}

#[async_trait]
impl Tool for CreateTaskFull {
	fn name(&self) -> ToolName {
		ToolName::CreateTaskFull
	}

	/// Carries `role`, `run_at_seconds`, `repeat_seconds` and `priority` beyond
	/// the common two. The `role` enum is built from every [`RoleName`], so it
	/// cannot name a Role that does not exist.
	fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
		let roles: Vec<&'static str> =
			RoleName::VARIANTS.iter().map(Into::into).collect();
		ToolSchema {
			name: self.name().to_string(),
			description: "Enqueue a Task, choosing its Role, its timing and \
			              its priority."
				.to_string(),
			parameters: serde_json::json!({
				"type": "object",
				"properties": {
					"title": { "type": "string", "description": TITLE_DESC },
					"brief": { "type": "string", "description": BRIEF_DESC },
					"role": {
						"type": "string",
						"enum": roles,
						"description": "Which Role should do this work.",
					},
					"run_at_seconds": {
						"type": "integer",
						"description": "Delay in seconds from now before this \
										Task may run. Omit to run as soon as \
										the queue reaches it.",
					},
					"repeat_seconds": {
						"type": "integer",
						"description": "If set, this Task repeats every this \
										many seconds, anchored to \
										`run_at_seconds`.",
					},
					"priority": {
						"type": "string",
						"enum": ["high", "normal", "low"],
						"description": "How urgently the swarm should spend a \
										model call on this Task. Defaults to \
										normal.",
					},
				},
				"required": ["title", "brief", "role"],
			}),
		}
	}

	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		let parsed = parse_args(&args);
		let schedule = Schedule::from_offsets(
			parsed.run_at_seconds,
			parsed.repeat_seconds,
			ctx.clock.now(),
		);
		let priority = match priority_from(parsed.priority) {
			Ok(p) => p,
			Err(e) => return e.to_string(),
		};
		let title = match require_title(parsed.title) {
			Ok(t) => t,
			Err(e) => return e.to_string(),
		};
		let brief = match require_brief(parsed.brief) {
			Ok(b) => b,
			Err(e) => return e.to_string(),
		};
		let role = match require_role(parsed.role) {
			Ok(r) => r,
			Err(e) => return e.to_string(),
		};

		let new = NewTask {
			title,
			brief,
			role,
			schedule,
			priority,
			created_by: Creator::Session(ctx.id),
		};
		match ctx.harness.create_task(new) {
			Ok(id) => created_reply(ctx, id),
			Err(e) => format!("Error: {e}"),
		}
	}
}
