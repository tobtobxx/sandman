//! Compiled-in prompts — the only wording for Worker, Comms and metacognition.
//!
//! Construct: `include_str!("prompts/*.md")` at build time; a missing file fails
//! the build, no runtime read. [`system_prompt`] joins [`MECHANICS`] with the
//! Role's file — the whole assembly, done once so every consumer reads a finished string.
//! Use: Worker → [`system_prompt`] (`MECHANICS` + Role); Comms → [`COMMS_SESSION`];
//! metacognition → [`META_SYSTEM`] + [`REVIEW`]/[`INTERRUPT`] as system/question pairs.
//! Consumers: `roles::system_prompt` delegates here; `harness::attach` and
//! `comms::respond` ([`COMMS_SESSION`]); `reflect::{reflect,interrupt}`
//! ([`META_SYSTEM`] + question); bench rig (real prompts, scripted model).
//! Seam: [`system_prompt`] matches `RoleName` exhaustively — a Role without a prompt
//! does not compile. No templating seam; prompts are plain Markdown.
//!
//! | Prompt | Consumer | Composition |
//! | --- | --- | --- |
//! | `MECHANICS` + Role | Worker Session | `MECHANICS` joined with one of `RESEARCH`/`PLANNING`/`MEMORY`/`TASK_MANAGER` |
//! | `COMMS_SESSION` | Comms Session | alone, no `MECHANICS`, no Role |
//! | `META_SYSTEM` + `REVIEW` | `reflect::reflect` | system + question, no tools |
//! | `META_SYSTEM` + `INTERRUPT` | `reflect::interrupt` | system + question, no tools |
//!
//! Rules: **nothing templated, nothing interpolated, nothing conditional.**
//! **repetition is paid deliberately — a prompt assembled in the reader's head hides contradictions.**
//! **Worker gets `MECHANICS`+Role, Comms gets `COMMS_SESSION` alone, metacognition gets `META_SYSTEM`+question.**
//!
//! Defines: [`MECHANICS`], [`COMMS_SESSION`], [`RESEARCH`]/[`PLANNING`]/[`MEMORY`]/[`TASK_MANAGER`],
//! [`META_SYSTEM`], [`REVIEW`], [`INTERRUPT`], [`system_prompt`].

/// Shared mechanics prepended to every Worker system prompt.
pub const MECHANICS: &str = include_str!("prompts/mechanics.md");

/// Whole system prompt for the standing Comms Session.
///
/// No `MECHANICS`, no Role, never produces a Result.
pub const COMMS_SESSION: &str = include_str!("prompts/comms-session.md");

/// Role prompt for Research.
pub const RESEARCH: &str = include_str!("prompts/research.md");

/// Role prompt for Planning.
pub const PLANNING: &str = include_str!("prompts/planning.md");

/// Role prompt for Memory.
pub const MEMORY: &str = include_str!("prompts/memory.md");

/// Role prompt for TaskManager.
pub const TASK_MANAGER: &str = include_str!("prompts/task_manager.md");

/// Shared system prompt for metacognition.
///
/// Used by both review and interrupt.
pub const META_SYSTEM: &str = include_str!("prompts/meta.md");

/// Question a review asks about a Worker's turn.
pub const REVIEW: &str = include_str!("prompts/review-prompt.md");

/// Question an interrupt asks mid-turn.
pub const INTERRUPT: &str = include_str!("prompts/interrupt-prompt.md");

/// Build the Worker system prompt for a Role.
///
/// Joins `MECHANICS` with the Role's file and returns it.
pub fn system_prompt(role: crate::roles::RoleName) -> String {
	use crate::roles::RoleName;
	// Resolve role text
	let role_text = match role {
		RoleName::Research => RESEARCH,
		RoleName::Planning => PLANNING,
		RoleName::Memory => MEMORY,
		RoleName::TaskManager => TASK_MANAGER,
	};
	// Join mechanics and role
	format!("{MECHANICS}\n\n{role_text}")
}
