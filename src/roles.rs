//! Closed Role catalogue — prompt plus tool set per Task.
//!
//! A Role is a property of work, not a kind of agent. Every Worker runs the
//! same loop; only its Task's Role changes the system prompt and the tools
//! offered. Tools live in `tools/` independent of Roles; this file decides the
//! assignment.
//!
//! Construct: `RoleName` and `ToolName` variants (`strum` snake_case is the
//! single string form; serde delegates to it so no second name exists).
//! Use: `system_prompt(role) -> String` joins mechanics and Role text,
//! `tools_for(role) -> &[ToolName]` returns the assignment, `schemas_for` builds
//! per-Session schemas (`message_human` enumerates only open Channels).
//! Consumers: `worker::new_worker_session` via `system_prompt`, `tools::Registry`
//! via `schemas_for`, bench `intercept` via `ToolName`.
//!
//! | Role | system_prompt | tools_for |
//! | --- | --- | --- |
//! | Research | mechanics + research.md | create_task, create_research_task, await_result, web_search, web_fetch |
//! | Planning | mechanics + planning.md | create_task, create_task_full, await_result, message_human |
//! | Memory | mechanics + memory.md | create_task, await_result, search_lessons, search_tasks, view_session |
//! | TaskManager | mechanics + task_manager.md | create_task_full, await_result, list_tasks, search_tasks, cancel_task |
//! | Comms (`COMMS_SESSION_TOOLS`) | `prompts::COMMS_SESSION` (no mechanics) | create_task only |
//!
//! Rules: **new Role without a prompt or tool set does not compile — exhaustive match.**
//! **no second string form — serde uses strum.** **Comms has no Role and never produces a Result.**
//! **tools independent of Roles — multiple Roles may share a tool.**
//!
//! Defines: [`RoleName`], [`ToolName`], [`COMMS_SESSION_TOOLS`], [`system_prompt`],
//! [`tools_for`], [`SchemaCtx`], [`schemas_for`].

use crate::domain::ToolSchema;

/// Closed set of Worker kinds. A Task carries one; its Session inherits the prompt and tools.
#[derive(
	Debug,
	Clone,
	Copy,
	PartialEq,
	Eq,
	PartialOrd,
	Ord,
	Hash,
	strum::Display,
	strum::EnumString,
	strum::IntoStaticStr,
	strum::VariantArray,
)]
#[strum(serialize_all = "snake_case")]
pub enum RoleName {
	/// Searches the web and answers with URLs.
	Research,
	/// Splits work into Tasks; the default and only Role that can reach a human.
	Planning,
	/// Searches lessons and past Tasks.
	Memory,
	/// Operates the queue — lists, searches and cancels Tasks.
	TaskManager,
}

/// One tool, by name. The closed set; `tools/` holds what each one does.
#[derive(
	Debug,
	Clone,
	Copy,
	PartialEq,
	Eq,
	PartialOrd,
	Ord,
	Hash,
	strum::Display,
	strum::EnumString,
	strum::IntoStaticStr,
)]
#[strum(serialize_all = "snake_case")]
pub enum ToolName {
	/// Enqueue a planning Task without Role or timing.
	CreateTask,
	/// Enqueue a Task, choosing Role, timing and priority.
	CreateTaskFull,
	/// Enqueue a research Task.
	CreateResearchTask,
	/// Block this turn until a Task completes and return its answer.
	AwaitResult,
	/// Inject a message into the Comms Session on a Channel.
	MessageHuman,
	WebSearch,
	WebFetch,
	SearchLessons,
	SearchTasks,
	ViewSession,
	ListTasks,
	CancelTask,
}

/// Tools for the standing Comms Session.
/// Never ends and never produces a Result; only creates planning Tasks.
pub const COMMS_SESSION_TOOLS: [ToolName; 1] = [ToolName::CreateTask];

/// Build the Worker's system prompt for a Role.
/// Joins shared mechanics and Role text with no templating.
/// Delegates to `prompts::system_prompt`.
pub fn system_prompt(role: RoleName) -> String {
	crate::prompts::system_prompt(role)
}

/// Return the tool set for a Role.
/// Exhaustive match — adding a Role without tools does not compile.
pub fn tools_for(role: RoleName) -> &'static [ToolName] {
	match role {
		RoleName::Research => &[
			ToolName::CreateTask,
			ToolName::CreateResearchTask,
			ToolName::AwaitResult,
			ToolName::WebSearch,
			ToolName::WebFetch,
		],
		RoleName::Planning => &[
			ToolName::CreateTask,
			ToolName::CreateTaskFull,
			ToolName::AwaitResult,
			ToolName::MessageHuman,
		],
		RoleName::Memory => &[
			ToolName::CreateTask,
			ToolName::AwaitResult,
			ToolName::SearchLessons,
			ToolName::SearchTasks,
			ToolName::ViewSession,
		],
		RoleName::TaskManager => &[
			ToolName::CreateTaskFull,
			ToolName::AwaitResult,
			ToolName::ListTasks,
			ToolName::SearchTasks,
			ToolName::CancelTask,
		],
	}
}

// Single string form — serde delegates to strum snake_case so names never diverge

impl serde::Serialize for RoleName {
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.collect_str(self)
	}
}

impl<'de> serde::Deserialize<'de> for RoleName {
	fn deserialize<D: serde::Deserializer<'de>>(
		d: D,
	) -> Result<Self, D::Error> {
		let s = <String as serde::Deserialize>::deserialize(d)?;
		s.parse().map_err(|_| {
			serde::de::Error::custom(format!("`{s}` is not a Role"))
		})
	}
}

impl serde::Serialize for ToolName {
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.collect_str(self)
	}
}

impl<'de> serde::Deserialize<'de> for ToolName {
	fn deserialize<D: serde::Deserializer<'de>>(
		d: D,
	) -> Result<Self, D::Error> {
		let s = <String as serde::Deserialize>::deserialize(d)?;
		s.parse().map_err(|_| {
			serde::de::Error::custom(format!("`{s}` is not a tool"))
		})
	}
}

/// Context for building tool schemas for a Session.
/// Carries open Channels so `message_human` enumerates only what exists.
#[derive(Debug, Clone)]
pub struct SchemaCtx {
	pub open_channels:
		Vec<(crate::domain::ChannelId, crate::domain::ChannelKind)>,
}

/// Build tool schemas for the model from a name set.
/// Sole implementation — `Registry` delegates here so descriptions never diverge.
pub fn schemas_for(names: &[ToolName], ctx: &SchemaCtx) -> Vec<ToolSchema> {
	use crate::tools::{
		await_result, create_task, message_human, queue, recall, web, Tool,
	};

	// Map names to schemas
	names
		.iter()
		.map(|name| match name {
			// create-task family
			ToolName::CreateTask => create_task::CreateTask.schema(ctx),
			ToolName::CreateTaskFull => create_task::CreateTaskFull.schema(ctx),
			ToolName::CreateResearchTask => {
				create_task::CreateResearchTask.schema(ctx)
			},
			// await and human
			ToolName::AwaitResult => await_result::AwaitResult.schema(ctx),
			ToolName::MessageHuman => message_human::MessageHuman.schema(ctx),
			// web
			ToolName::WebSearch => web::WebSearch.schema(ctx),
			ToolName::WebFetch => web::WebFetch.schema(ctx),
			// recall
			ToolName::SearchLessons => recall::SearchLessons.schema(ctx),
			ToolName::SearchTasks => recall::SearchTasks.schema(ctx),
			ToolName::ViewSession => recall::ViewSession.schema(ctx),
			// queue
			ToolName::ListTasks => queue::ListTasks.schema(ctx),
			ToolName::CancelTask => queue::CancelTask.schema(ctx),
		})
		.collect()
}
