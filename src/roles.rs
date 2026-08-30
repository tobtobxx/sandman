//! The Role set, closed and living in code.
//!
//! A Role is a system prompt plus a set of tools, carried by a Task, which
//! together decide how its Session approaches the problem. A Role is a property
//! of work, never a kind of agent: every Worker is mechanically the same, and
//! only the Role of its Task differs.
//!
//! Tools are independent of Roles, so more than one Role may hold the same tool.
//!
//! [`RoleName`] is the single source of truth. `system_prompt` and `tools_for`
//! match on it exhaustively, so a Role added without a prompt or without a tool
//! set does not compile. In the prototype these were a hand-written map and the
//! two could drift: a Role could vanish from `create_task`'s enum while the
//! catalogue in the prompts still named it, and a Worker could argue with that
//! contradiction for a whole run.
//!
//! Defines: [`RoleName`], [`ToolName`], [`system_prompt`], [`tools_for`],
//! [`role_catalogue`], [`COMMS_SESSION_TOOLS`].

use crate::domain::ToolSchema;

/// Which kind of Worker does a Task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RoleName {
	/// Finds things out in the world. Searches and reads the web, and answers
	/// with the URLs it relied on.
	Research,
	/// Breaks work into pieces and creates a Task for each. The default: work
	/// goes here when nothing else is plainly right. The only Role that can
	/// reach a human, through `message_human`.
	Planning,
	/// Finds out what this swarm already did. Searches the Lessons and what past
	/// Tasks asked and answered.
	Memory,
	/// Runs the swarm's queue. Lists and searches Tasks, and cancels the ones
	/// that must not run.
	TaskManager,
}

/// One tool, by name. The closed set; `tools/` holds what each one does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ToolName {
	/// Enqueue a planning Task. The common case, with no Role or timing to get
	/// wrong.
	CreateTask,
	/// Enqueue a Task, choosing the Role, the timing and the priority.
	CreateTaskFull,
	/// Enqueue a research Task.
	CreateResearchTask,
	/// Block this turn until a Task completes, and return its answer.
	AwaitResult,
	/// Inject a message into the Comms Session on a named Channel.
	MessageHuman,
	WebSearch,
	WebFetch,
	SearchLessons,
	SearchTasks,
	ViewSession,
	CurrentTime,
	ListTasks,
	CancelTask,
}

/// Every Role, in catalogue order.
pub const ROLE_NAMES: [RoleName; 4] = [
	RoleName::Research,
	RoleName::Planning,
	RoleName::Memory,
	RoleName::TaskManager,
];

/// The Comms Session is not a Worker and has no Role, but it still needs tools.
/// It never ends, so it is never told to produce a Result.
pub const COMMS_SESSION_TOOLS: [ToolName; 1] = [ToolName::CreateTask];

/// The system message a Worker Session starts with: the shared mechanics, then
/// the Role's own text, joined and nothing else.
///
/// Nothing is assembled conditionally and nothing is interpolated. The cost is
/// repetition — the Role catalogue is written out in several prompt files — and
/// it is paid on purpose: a prompt that has to be assembled in the reader's head
/// is a prompt whose contradictions are invisible.
pub fn system_prompt(_role: RoleName) -> String {
	unimplemented!()
}

/// Which tools a Role holds.
///
/// The three create-task tools are split so a Role that should not choose Roles
/// only ever sees the narrow one.
pub fn tools_for(_role: RoleName) -> &'static [ToolName] {
	unimplemented!()
}

/// The catalogue on its own, for the error a bad `role` argument gets back.
/// This is an error message, not a prompt: no system prompt is built from it.
pub fn role_catalogue() -> &'static str {
	unimplemented!()
}

impl RoleName {
	/// The name this Role goes by on the wire, in a Brief and in the database.
	pub fn as_str(&self) -> &'static str {
		unimplemented!()
	}

	pub fn parse(_name: &str) -> Option<RoleName> {
		unimplemented!()
	}
}

impl ToolName {
	pub fn as_str(&self) -> &'static str {
		unimplemented!()
	}

	pub fn parse(_name: &str) -> Option<ToolName> {
		unimplemented!()
	}
}

impl std::fmt::Display for RoleName {
	fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		unimplemented!()
	}
}

impl std::fmt::Display for ToolName {
	fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		unimplemented!()
	}
}

/// What a tool needs to know about the world to describe itself.
///
/// Schemas are built for each Session rather than declared once, because
/// `message_human` must offer the Channels that are actually open — its
/// `channel` enum should only ever name a Channel that exists.
#[derive(Debug, Clone)]
pub struct SchemaCtx {
	pub open_channels:
		Vec<(crate::domain::ChannelId, crate::domain::ChannelKind)>,
}

/// The schemas for a set of tools, as they go to the model.
pub fn schemas_for(_names: &[ToolName], _ctx: &SchemaCtx) -> Vec<ToolSchema> {
	unimplemented!()
}
