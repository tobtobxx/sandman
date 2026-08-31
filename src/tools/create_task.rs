//! Enqueueing work. The only route between agents.
//!
//! Construct: `NewTask { title, brief, role, schedule, priority, created_by }` at `Harness::create_task` → `Store::create_task` mints `TaskId`, derives `subscriber` from `Creator::Session` and emits `TaskCreated`; each tool builds `NewTask` from `SessionCtx`.
//! Use: `Tool::call(ctx, args) -> String` parses JSON via `parse_args`, validates via `require_*`, builds `NewTask` and enqueues; `await_result` later holds for the answer, `created_reply` tells Workers to await and Comms Sessions that mail will arrive.
//! Consumers: `roles.rs` assigns `CreateTask` to every Worker and Comms (`COMMS_SESSION_TOOLS`), `CreateResearchTask` to `Research`, `CreateTaskFull` to `Planning`/`TaskManager`; `Registry::all` constructs all three.
//! Seam: `Tool` (one capability: `schema` + `call`) vs `ToolRunner::Registry` (real dispatch, emits `ToolCalled`/`ToolReturned`); bench replaces `ToolRunner` to watch or script answers without changing prompts.
//!
//! | Tool | role | schedule | priority | holder |
//! | --- | --- | --- | --- | --- |
//! | `CreateTask` | `Planning` fixed | `Now` fixed | `Normal` fixed | every Worker + Comms |
//! | `CreateResearchTask` | `Research` fixed | `Now` fixed | `Normal` fixed | `Research` |
//! | `CreateTaskFull` | caller chooses | caller chooses (`run_at`/`repeat`) | caller chooses | `Planning`, `TaskManager` |
//!
//! Call trace: `Tool::call → parse_args → require_* → NewTask → Harness::create_task → Store::create_task → created_reply`
//!
//! Rules: **one enqueue path — `Harness::create_task` → `Store::create_task`.** **no tool waits; `await_result` holds.** **no tool subscribes; Store derives `subscriber` from `Creator`.** **narrow tool per Role so common case carries no `Role`/`Schedule` to get wrong.** **Worker told to `await_result`, Comms told mail will arrive — same `Session` read both ways.**
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

/// Shared wording, so three schemas cannot describe the same argument three ways.
pub const TITLE_DESC: &str =
	"One line describing the Task, so a human can scan it.";
pub const BRIEF_DESC: &str =
	"The full instructions. The Worker sees nothing else, so include \
     every fact it needs. Write it for someone with no context.";

/// Enqueue a planning Task. No Role and no timing to choose.
pub struct CreateTask;

/// Enqueue a research Task without leaving the current line of work.
pub struct CreateResearchTask;

/// Enqueue a Task, choosing its Role, timing and priority.
pub struct CreateTaskFull;

/// Raw fields from a create-task call, unchecked.
/// Each tool validates only what it accepts.
struct ParsedArgs {
	title: Option<String>,
	brief: Option<String>,
	role: Option<String>,
	run_at_seconds: Option<i64>,
	repeat_seconds: Option<i64>,
	priority: Option<String>,
}

/// Extract all create-task fields from JSON without validation.
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

/// Parse `priority` or default to `Normal`.
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

/// Reply after enqueue, worded by `SessionKind`.
/// Workers are told to `await_result`; Comms Sessions are told mail will arrive.
fn created_reply(ctx: &SessionCtx, id: TaskId) -> String {
	// Resolve Session kind
	let is_worker = ctx
		.store
		.session(ctx.id)
		.ok()
		.flatten()
		.map(|s| matches!(s.kind, crate::domain::SessionKind::Worker { .. }))
		.unwrap_or(true);
	// Reply by kind
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
		// Parse arguments
		let parsed = parse_args(&args);
		// Validate fields
		let title = match require_title(parsed.title) {
			Ok(t) => t,
			Err(e) => return e.to_string(),
		};
		let brief = match require_brief(parsed.brief) {
			Ok(b) => b,
			Err(e) => return e.to_string(),
		};

		// Enqueue task
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
		// Parse arguments
		let parsed = parse_args(&args);
		// Validate fields
		let title = match require_title(parsed.title) {
			Ok(t) => t,
			Err(e) => return e.to_string(),
		};
		let brief = match require_brief(parsed.brief) {
			Ok(b) => b,
			Err(e) => return e.to_string(),
		};

		// Enqueue task
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

	/// Schema with `role`, `run_at_seconds`, `repeat_seconds` and `priority`.
	/// `role` enum is built from `RoleName`, so no unknown Role.
	fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
		// Collect roles
		let roles: Vec<&'static str> =
			RoleName::VARIANTS.iter().map(Into::into).collect();
		// Build schema
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
		// Parse arguments
		let parsed = parse_args(&args);
		// Build schedule
		let schedule = Schedule::from_offsets(
			parsed.run_at_seconds,
			parsed.repeat_seconds,
			ctx.clock.now(),
		);
		// Validate priority
		let priority = match priority_from(parsed.priority) {
			Ok(p) => p,
			Err(e) => return e.to_string(),
		};
		// Validate fields
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

		// Enqueue task
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
