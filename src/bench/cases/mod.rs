//! The bench cases themselves.
//!
//! One file per case, each opening with what it tests in plain language.
//! `cargo test -- --ignored` runs the `#[cfg(test)]` wrapper next to each one,
//! and `bin/bench` runs the same [`CASES`] table several times over and keeps
//! the artifacts. Everything lives in the library, because `bin/bench` cannot
//! reach into an integration test crate.
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

mod greet_again;
mod hello;
mod plan_greet;

use std::future::Future;
use std::pin::Pin;

use super::report::{self, RunReport};
use super::{CheckResult, Grader, Rig, Trip, Watch};

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

/// Every case. Adding one is a file below and a line here.
pub const CASES: &[Case] = &[
	Case {
		name: "hello",
		description: "`\"Hello :)\"` gets a reply and reaches for no Task.",
		run: || Box::pin(hello::run()),
	},
	Case {
		name: "greet-again",
		description:
			"Asking to be greeted again in ~3 minutes creates one Task.",
		run: || Box::pin(greet_again::run()),
	},
	Case {
		name: "plan-greet",
		description: "A planning Worker schedules a greeting ~3 minutes out.",
		run: || Box::pin(plan_greet::run()),
	},
];

/// The case of that name, for `--case` and for a test wrapper.
pub fn find(name: &str) -> Option<&'static Case> {
	CASES.iter().find(|c| c.name == name)
}

/// A `RunReport` for a case whose Rig never came up. Not a `RunReport::assemble`
/// call, because that needs a Rig to wind down and there is none.
fn build_failed(case: &Case, trip: &Trip) -> RunReport {
	RunReport {
		case: case.name.to_string(),
		description: case.description.to_string(),
		model: String::new(),
		reasoning_effort: String::new(),
		started_at: 0,
		finished_at: 0,
		wall_ms: 0,
		pass: false,
		tripped: Some(trip.to_string()),
		checks: Vec::new(),
		graders: Vec::new(),
		failed_calls: Vec::new(),
		spend: crate::domain::Spend::default(),
		grader_cost: crate::domain::Cost(0),
	}
}

/// Wind a Rig down, assemble its report, and hand back the `(Option<Rig>,
/// RunReport)` shape every case's `run` returns.
///
/// The Rig always comes back `Some` here — a build failure never reaches this
/// far, since there is no Rig yet for `assemble` to wind down; that path is
/// [`build_failed`] instead.
async fn finish(
	case: &'static Case,
	mut rig: Rig,
	outcome: Result<Vec<CheckResult>, Trip>,
	graders: Vec<Grader>,
) -> (Option<Rig>, RunReport) {
	let report = report::assemble(case, &mut rig, outcome, graders).await;
	(Some(rig), report)
}

/// Tripwire: a Task-creating tool is never reached for more than `n` times.
///
/// Counts calls, not rows: a unit bench answers the creation itself, so a
/// Session that keeps asking for Tasks leaves nothing in the Store to count.
fn at_most_creations(n: usize) -> impl Fn(&Watch) -> CheckResult + Send + Sync {
	use crate::roles::ToolName;
	move |watch: &Watch| {
		let count = watch
			.calls
			.iter()
			.filter(|c| {
				matches!(
					c.name,
					ToolName::CreateTask
						| ToolName::CreateTaskFull
						| ToolName::CreateResearchTask
				)
			})
			.count();
		if count <= n {
			CheckResult::ok(
				"at_most_creations",
				format!("{count} creation call(s) so far"),
			)
		} else {
			CheckResult::no(
				"at_most_creations",
				format!("{count} creation call(s), more than the {n} allowed"),
			)
		}
	}
}

/// The `#[ignore]`d test wrapper every case needs: run it for real, fail the
/// test with the report if it did not pass. One line per case instead of
/// eight.
macro_rules! bench_test {
	($name:ident) => {
		#[cfg(test)]
		mod tests {
			#[tokio::test]
			#[ignore = "spends money on a real model; cargo test -- --ignored"]
			async fn $name() {
				let (_, report) = super::run().await;
				if !report.pass {
					panic!(
						concat!(stringify!($name), " did not pass: {:#?}"),
						report
					);
				}
			}
		}
	};
}
pub(crate) use bench_test;
