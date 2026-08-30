//! The Run: one lifetime of Sandman, as the database records it.
//!
//! Before anything persisted, "a run" and "the process" were the same thing and
//! neither needed a name. With the state kept, they come apart: several Runs
//! share one database, and the question of what a number covers has to be
//! answerable.
//!
//! The split is deliberate and not symmetric. **Spend is scoped to a Run** —
//! what this run has cost is the number a human wants, and summing every run
//! Sandman has ever done would be summing nothing in particular. **The Lessons
//! and past Tasks are not** — they are searched across every Run, which is what
//! finally makes the `memory` Role useful rather than empty for the first ten
//! minutes of every start.
//!
//! Defines: [`Run`].

use super::ids::RunId;
use super::time::Timestamp;

/// One lifetime of Sandman.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
	pub id: RunId,
	pub started_at: Timestamp,
	/// Set on a clean shutdown. Absent on a Run still going, and on one whose
	/// process was killed — the two are indistinguishable afterwards, which is
	/// accepted.
	pub ended_at: Option<Timestamp>,
	/// The model this Run talked to, so a comparison across Runs knows what it
	/// is comparing.
	pub model: String,
}
