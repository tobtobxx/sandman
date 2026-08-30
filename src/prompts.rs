//! Every prompt, as a literal string compiled into the binary.
//!
//! Each is one plain Markdown file under `prompts/`, and the content is the
//! text — nothing is templated and nothing is interpolated. `include_str!` means
//! a missing file is a build failure rather than a run that starts and then
//! cannot answer, and there is no startup read to go wrong.
//!
//! [`system_prompt`] joins exactly two of these: the shared mechanics, then the
//! Role's own file. That join is the whole assembly, done once here so every
//! consumer reads a finished prompt rather than reassembling it. The cost is
//! repetition — the Role catalogue is written out in each prompt that needs it
//! — and it is paid deliberately: this prompt set has twice shipped a
//! self-contradiction, and both times it hid in text that no single place held
//! whole.
//!
//! Defines: the prompt constants, [`system_prompt`].

/// What every Worker Session is told about how Sandman works, before its Role's
/// own text.
pub const MECHANICS: &str = include_str!("prompts/mechanics.md");

/// The Comms Session's whole system prompt. It is not a Worker, gets none of
/// [`MECHANICS`], and is never told to produce a Result.
pub const COMMS_SESSION: &str = include_str!("prompts/comms-session.md");

pub const RESEARCH: &str = include_str!("prompts/research.md");
pub const PLANNING: &str = include_str!("prompts/planning.md");
pub const MEMORY: &str = include_str!("prompts/memory.md");
pub const TASK_MANAGER: &str = include_str!("prompts/task_manager.md");

/// What metacognition is, told to the model that performs it. Shared by the
/// review and the interrupt; only the question after it differs.
pub const META_SYSTEM: &str = include_str!("prompts/meta.md");

/// The question a review is asked: what did this Worker's turn mean for its Task?
pub const REVIEW: &str = include_str!("prompts/review-prompt.md");

/// The question an interrupt is asked: is this run still going somewhere?
pub const INTERRUPT: &str = include_str!("prompts/interrupt-prompt.md");

/// The whole system prompt a Worker Session of this Role starts with:
/// [`MECHANICS`], then the Role's own file, joined and nothing else.
pub fn system_prompt(role: crate::roles::RoleName) -> String {
	use crate::roles::RoleName;
	let role_text = match role {
		RoleName::Research => RESEARCH,
		RoleName::Planning => PLANNING,
		RoleName::Memory => MEMORY,
		RoleName::TaskManager => TASK_MANAGER,
	};
	format!("{MECHANICS}\n\n{role_text}")
}
