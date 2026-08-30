//! The bench cases themselves.
//!
//! They live in the library rather than in `tests/` because both consumers have
//! to reach them: `cargo test` runs each as an ordinary `#[tokio::test]` through
//! the thin wrappers in `tests/cases.rs`, and `bin/bench` runs the same table
//! several times over and keeps the artifacts. An integration test is its own
//! crate, so a binary cannot call into one.
//!
//! Every case is a unit bench: one Session, one real Brief, every tool call
//! intercepted. It asks what the model reached for — which tool, with what
//! arguments, in what order — and says nothing about what the swarm would have
//! done with that choice. Integration is a series of these, not a case of its
//! own.
//!
//! Three kinds of verification, and they fail differently:
//!
//! - **Tripwires** — evaluated continuously, on every Event. A violation ends the
//!   run at once, so a looping Session costs at most a call or two past it. For
//!   "this must never happen".
//! - **Goal checks** — evaluated once, at the end. A failure fails the run but
//!   does not stop it; the work is already done and its evidence is in the
//!   artifacts. For "this must have happened by the end".
//! - **Graders** — a model call, for what no read of the state can judge. Rare,
//!   and warranted only when nothing countable answers the question. They run
//!   only after the goal checks pass.
//!
//! Rule of thumb: if a bad outcome would make the Session keep working, it is a
//! tripwire; if it can only be known at the end, a check; if a machine cannot
//! judge it at all, a grader.
//!
//! Not covered here: delivery — whether a Session reaches for `message_human`
//! and names the right Channel. That is the natural fourth case, and TASKS.md
//! says why it is the one most likely to fail.
//!
//! Defines: [`Case`], [`CASES`], [`find`].

use std::future::Future;
use std::pin::Pin;

use super::report::RunReport;
use super::{CheckResult, Rig, Watch};

/// What the human says in the `hello` case.
const HELLO_MESSAGE: &str = "Hello :)";

/// What the human says in the `greet-again` case.
const GREET_MESSAGE: &str =
	"Hey! Could you greet me again in about 3 minutes? :)";

/// One case, run to the end.
///
/// The Rig comes back so the driver can write `store.sqlite` and `sandman.log`
/// out of it; it is absent only when the Rig could not be built, which still
/// reports. A tripwire is not an error here — it is [`RunReport::tripped`], so a
/// run that ended early still says what it saw on the way.
type CaseFn =
	fn() -> Pin<Box<dyn Future<Output = (Option<Rig>, RunReport)> + Send>>;

/// One question put to the harness-and-model combination.
pub struct Case {
	pub name: &'static str,
	/// One line, for `result.json` and the driver's output.
	pub description: &'static str,
	pub run: CaseFn,
}

/// Every case. Adding one is a function above and a line here.
pub const CASES: &[Case] = &[
	Case {
		name: "hello",
		description: "`\"Hello :)\"` gets a reply and reaches for no Task.",
		run: || Box::pin(hello()),
	},
	Case {
		name: "greet-again",
		description:
			"Asking to be greeted again in ~3 minutes creates one Task.",
		run: || Box::pin(greet_again()),
	},
	Case {
		name: "plan-greet",
		description: "A planning Worker schedules a greeting ~3 minutes out.",
		run: || Box::pin(plan_greet()),
	},
];

/// The case of that name, for `--case` and for a test wrapper.
pub fn find(_name: &str) -> Option<&'static Case> {
	unimplemented!()
}

/// Tripwire: `create_task` is never reached for more than `n` times.
///
/// Counts calls, not rows: a unit bench answers the creation itself, so a
/// Session that keeps asking for Tasks leaves nothing in the Store to count.
fn at_most_creations(
	_n: usize,
) -> impl Fn(&Watch) -> CheckResult + Send + Sync {
	|_watch| unimplemented!()
}

/// One Comms Session, nothing driving the queue. Reaching for `create_task` is
/// the failure, so the case denies it and asserts it was never called.
async fn hello() -> (Option<Rig>, RunReport) {
	unimplemented!()
}

/// The Brief handed over is judged by a grader — whether it is a faithful
/// hand-off of what the human wanted — because no count answers that. The grader
/// passes a Brief that only describes the delay in words: the Comms Session has
/// no scheduling tool, so turning words into a timed Task is the next Worker's
/// job.
async fn greet_again() -> (Option<Rig>, RunReport) {
	unimplemented!()
}

/// A planning Task seeded from outside. The planner should reach for
/// `create_task` exactly once, with a Schedule ~3 minutes out, and complete. The
/// creation is answered by the case, so no child Worker runs and no scheduled
/// Task is left to cancel — the whole case is one Session's decisions.
async fn plan_greet() -> (Option<Rig>, RunReport) {
	unimplemented!()
}
