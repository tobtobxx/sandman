//! Tool registry — twelve capabilities dispatched through one seam.
//!
//! Construct: [`Registry::all`] builds every [`Tool`] (`Vec<Arc<dyn Tool>>`);
//! `SessionCtx.tools: Arc<dyn ToolRunner>` carries it.
//! Use: `session::turn` builds `schemas(names, SchemaCtx) -> Vec<ToolSchema>`
//! then `run(ctx, call) -> String` per `Reply::Calls` until `Text`/`Silent`.
//! Consumers: `session::turn` (Worker vs Comms tool set via `roles::tools_for`);
//! bench recorder/script wrapping [`Registry`] without touching prompts.
//!
//! | Seam | Real | Bench |
//! | --- | --- | --- |
//! | [`Tool`] — one capability | `create_task`, `await_result`, `message_human`, `web`, `recall`, `queue` in `tools/*` | — |
//! | [`ToolRunner`] — how calls get answered | [`Registry`] | recorder, script, refusal |
//!
//! Call trace: `turn → scheduler.request → Reply::Calls → tools.run(call) → Tool::call(ctx, args) → Store/Harness/Waiters → String → append Tool message → loop`.
//!
//! Rules: **tools independent of Roles — `roles.rs` assigns, multiple Roles may share a tool.** **a tool answers in words, always — failure is a sentence, not an `Err`.** **schemas built per Session — `message_human` enumerates only open Channels.** **`await_result` is the only tool that holds a Turn.** **metacognition has no tools.** **`Registry` emits `ToolCalled`/`ToolReturned` itself — tool calls are not Store state changes.**
//!
//! Defines: [`Tool`], [`ToolRunner`], [`Registry`], [`ToolError`].

pub mod await_result;
pub mod create_task;
pub mod message_human;
pub mod queue;
pub mod recall;
pub mod web;

use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::{ToolCall, ToolSchema};
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;

/// One capability a Session may hold.
#[async_trait]
pub trait Tool: Send + Sync {
	fn name(&self) -> ToolName;

	/// Describe this tool to the model.
	///
	/// Builds a [`ToolSchema`] for `ctx`; per-Session so Channel lists stay live.
	fn schema(&self, ctx: &SchemaCtx) -> ToolSchema;

	/// Execute the call and return the model's next read.
	///
	/// Takes parsed `args`; returns a sentence on success or on bad input.
	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String;
}

/// How a Session's tool calls get answered — the seam [`Registry`] implements.
#[async_trait]
pub trait ToolRunner: Send + Sync {
	/// Build schemas for the named tools.
	fn schemas(&self, names: &[ToolName], ctx: &SchemaCtx) -> Vec<ToolSchema>;

	/// Answer one call. Never fails; returns the string the model reads.
	async fn run(&self, ctx: &SessionCtx, call: &ToolCall) -> String;
}

/// The real runner: every tool, by name.
pub struct Registry {
	tools: Vec<Arc<dyn Tool>>,
	events: Arc<crate::event::Events>,
}

impl Registry {
	/// Build the full registry with every tool.
	pub fn all(events: Arc<crate::event::Events>) -> Self {
		let tools: Vec<Arc<dyn Tool>> = vec![
			Arc::new(create_task::CreateTask),
			Arc::new(create_task::CreateTaskFull),
			Arc::new(create_task::CreateResearchTask),
			Arc::new(await_result::AwaitResult),
			Arc::new(message_human::MessageHuman),
			Arc::new(web::WebSearch),
			Arc::new(web::WebFetch),
			Arc::new(recall::SearchLessons),
			Arc::new(recall::SearchTasks),
			Arc::new(recall::ViewSession),
			Arc::new(queue::ListTasks),
			Arc::new(queue::CancelTask),
		];
		Registry { tools, events }
	}
}

#[async_trait]
impl ToolRunner for Registry {
	/// Build schemas via the single [`crate::roles::schemas_for`] implementation.
	fn schemas(&self, names: &[ToolName], ctx: &SchemaCtx) -> Vec<ToolSchema> {
		crate::roles::schemas_for(names, ctx)
	}

	/// Dispatch one call and emit `ToolCalled`/`ToolReturned`.
	///
	/// Returns a sentence for unknown tools or bad JSON; never fails the turn.
	async fn run(&self, ctx: &SessionCtx, call: &ToolCall) -> String {
		// Parse tool name
		let Ok(name) = call.name.parse::<ToolName>() else {
			return ToolError::NoSuchTool(call.name.clone()).to_string();
		};

		// Parse arguments
		let parsed =
			serde_json::from_str::<serde_json::Value>(&call.arguments).ok();

		// Emit ToolCalled
		self.events.emit(crate::event::Event::ToolCalled {
			session: ctx.id,
			name,
			args: parsed.clone().unwrap_or(serde_json::Value::Null),
		});

		// Dispatch call
		let output = match parsed {
			// Bad JSON — return sentence
			None => ToolError::BadJson.to_string(),
			// Valid JSON — find tool and call
			Some(args) => match self.tools.iter().find(|t| t.name() == name) {
				Some(tool) => tool.call(ctx, args).await,
				None => ToolError::NoSuchTool(call.name.clone()).to_string(),
			},
		};

		// Emit ToolReturned
		self.events.emit(crate::event::Event::ToolReturned {
			session: ctx.id,
			name,
			output: output.clone(),
		});
		output
	}
}

/// Why a tool could not do what it asked — worded for the model.
///
/// Rendered as the string the model reads; never an `Err` past the runner.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
	#[error("Error: your arguments were not valid JSON. Try again.")]
	BadJson,
	#[error("Error: `{field}` is required.")]
	Missing { field: &'static str },
	#[error("Error: {0}")]
	Rejected(String),
	#[error("Error: there is no tool called {0}.")]
	NoSuchTool(String),
	#[error("Error: there is no Task {0}.")]
	NoSuchTask(String),
	#[error(
		"Error: `{given}` is not a Role. Not one of: {}",
		<crate::roles::RoleName as strum::VariantArray>::VARIANTS
			.iter()
			.map(|r| r.to_string())
			.collect::<Vec<_>>()
			.join(", ")
	)]
	NoSuchRole { given: String },
}
