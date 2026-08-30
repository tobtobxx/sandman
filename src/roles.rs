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
//! [`COMMS_SESSION_TOOLS`].

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
pub fn system_prompt(role: RoleName) -> String {
	crate::prompts::system_prompt(role)
}

/// Which tools a Role holds.
///
/// The three create-task tools are split so a Role that should not choose
/// Roles only ever sees the narrow one: research can hand a detail to
/// planning or to another researcher, but not pick an arbitrary Role, so it
/// gets `create_task` and `create_research_task` only. Planning may target
/// any Role, so it alone also gets `create_task_full`. The manager fills the
/// queue as well as changing it, so it gets `create_task_full` too, to
/// schedule and repeat work by Role. Memory only ever hands a follow-up to
/// planning, so it gets the plain `create_task`.
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

impl RoleName {
	/// The name this Role goes by on the wire, in a Brief and in the database.
	pub fn as_str(&self) -> &'static str {
		match self {
			RoleName::Research => "research",
			RoleName::Planning => "planning",
			RoleName::Memory => "memory",
			RoleName::TaskManager => "task_manager",
		}
	}

	pub fn parse(name: &str) -> Option<RoleName> {
		match name {
			"research" => Some(RoleName::Research),
			"planning" => Some(RoleName::Planning),
			"memory" => Some(RoleName::Memory),
			"task_manager" => Some(RoleName::TaskManager),
			_ => None,
		}
	}
}

impl ToolName {
	pub fn as_str(&self) -> &'static str {
		match self {
			ToolName::CreateTask => "create_task",
			ToolName::CreateTaskFull => "create_task_full",
			ToolName::CreateResearchTask => "create_research_task",
			ToolName::AwaitResult => "await_result",
			ToolName::MessageHuman => "message_human",
			ToolName::WebSearch => "web_search",
			ToolName::WebFetch => "web_fetch",
			ToolName::SearchLessons => "search_lessons",
			ToolName::SearchTasks => "search_tasks",
			ToolName::ViewSession => "view_session",
			ToolName::ListTasks => "list_tasks",
			ToolName::CancelTask => "cancel_task",
		}
	}

	pub fn parse(name: &str) -> Option<ToolName> {
		match name {
			"create_task" => Some(ToolName::CreateTask),
			"create_task_full" => Some(ToolName::CreateTaskFull),
			"create_research_task" => Some(ToolName::CreateResearchTask),
			"await_result" => Some(ToolName::AwaitResult),
			"message_human" => Some(ToolName::MessageHuman),
			"web_search" => Some(ToolName::WebSearch),
			"web_fetch" => Some(ToolName::WebFetch),
			"search_lessons" => Some(ToolName::SearchLessons),
			"search_tasks" => Some(ToolName::SearchTasks),
			"view_session" => Some(ToolName::ViewSession),
			"list_tasks" => Some(ToolName::ListTasks),
			"cancel_task" => Some(ToolName::CancelTask),
			_ => None,
		}
	}
}

impl std::fmt::Display for RoleName {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(self.as_str())
	}
}

impl std::fmt::Display for ToolName {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(self.as_str())
	}
}

// Both names are written by hand rather than derived, and both go through
// `as_str` and `parse`. These two names are read by the model, stored in the
// database and put on the wire at once; a derive would name them a fourth way of
// its own, and nothing would catch the day that name and `as_str` disagreed.

impl serde::Serialize for RoleName {
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.collect_str(self.as_str())
	}
}

impl<'de> serde::Deserialize<'de> for RoleName {
	fn deserialize<D: serde::Deserializer<'de>>(
		d: D,
	) -> Result<Self, D::Error> {
		let s = <String as serde::Deserialize>::deserialize(d)?;
		RoleName::parse(&s).ok_or_else(|| {
			serde::de::Error::custom(format!("`{s}` is not a Role"))
		})
	}
}

impl serde::Serialize for ToolName {
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.collect_str(self.as_str())
	}
}

impl<'de> serde::Deserialize<'de> for ToolName {
	fn deserialize<D: serde::Deserializer<'de>>(
		d: D,
	) -> Result<Self, D::Error> {
		let s = <String as serde::Deserialize>::deserialize(d)?;
		ToolName::parse(&s).ok_or_else(|| {
			serde::de::Error::custom(format!("`{s}` is not a tool"))
		})
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
///
/// The single implementation: [`crate::tools::Registry::schemas`] calls this
/// rather than building its own, so the model is never offered two competing
/// descriptions of the same tool.
pub fn schemas_for(names: &[ToolName], ctx: &SchemaCtx) -> Vec<ToolSchema> {
	use crate::tools::{
		await_result, create_task, message_human, queue, recall, web, Tool,
	};

	names
		.iter()
		.map(|name| match name {
			ToolName::CreateTask => create_task::CreateTask.schema(ctx),
			ToolName::CreateTaskFull => create_task::CreateTaskFull.schema(ctx),
			ToolName::CreateResearchTask => {
				create_task::CreateResearchTask.schema(ctx)
			},
			ToolName::AwaitResult => await_result::AwaitResult.schema(ctx),
			ToolName::MessageHuman => message_human::MessageHuman.schema(ctx),
			ToolName::WebSearch => web::WebSearch.schema(ctx),
			ToolName::WebFetch => web::WebFetch.schema(ctx),
			ToolName::SearchLessons => recall::SearchLessons.schema(ctx),
			ToolName::SearchTasks => recall::SearchTasks.schema(ctx),
			ToolName::ViewSession => recall::ViewSession.schema(ctx),
			ToolName::ListTasks => queue::ListTasks.schema(ctx),
			ToolName::CancelTask => queue::CancelTask.schema(ctx),
		})
		.collect()
}
