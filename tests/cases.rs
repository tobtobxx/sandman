//! The bench cases.
//!
//! Each is an ordinary `#[tokio::test]` with its own [`Rig`] — its own in-memory
//! database, its own id counters, its own log in a temporary directory. Nothing
//! is shared, so nothing needs a process of its own and nothing moves the working
//! directory.
//!
//! They are `#[ignore]`d because they spend money on a real model:
//!
//! ```sh
//! cargo test -- --ignored               # all of them
//! cargo test -- --ignored hello         # one
//! cargo run --bin bench -- --times 5    # with a report and artifacts
//! ```
//!
//! Three kinds of verification, and they fail differently:
//!
//! - **Tripwires** — evaluated continuously, on every Event. A violation ends the
//!   run at once, so a looping swarm costs at most a call or two past it. For
//!   "this must never happen".
//! - **Goal checks** — evaluated once, at the end. A failure fails the run but
//!   does not stop it; the work is already done and its evidence is in the
//!   artifacts. For "this must have happened by the end".
//! - **Graders** — a model call, for what no read of the state can judge. They
//!   run only after the goal checks pass.
//!
//! Rule of thumb: if a bad outcome would make the swarm keep working, it is a
//! tripwire; if it can only be known at the end, a check; if a machine cannot
//! judge it at all, a grader.
//!
//! Not covered here: end-to-end delivery — whether a greeting actually reaches
//! the human, through `message_human`. That is the natural fourth case, and
//! TASKS.md says why it is the one most likely to fail.

use sandman::bench::{CheckResult, Rig, Trip};
use sandman::harness::Drive;

/// What the human says in the `hello` case.
const HELLO_MESSAGE: &str = "Hello :)";

/// What the human says in the `greet-again` case.
const GREET_MESSAGE: &str = "Hey! Could you greet me again in about 3 minutes? :)";

/// Tripwire: never more than `n` Tasks in the whole run.
fn at_most_tasks(_n: usize) -> impl Fn(&sandman::store::Store) -> CheckResult + Send + Sync {
    |_store| unimplemented!()
}

/// `"Hello :)"` gets a reply and creates no Tasks.
///
/// Comms-only: nothing drives the queue, so a Task created here would sit
/// unexecuted — and creating one at all is the failure.
#[tokio::test]
#[ignore = "spends money on a real model; cargo test -- --ignored"]
async fn hello() -> Result<(), Trip> {
    unimplemented!()
}

/// Asking to be greeted again in ~3 minutes spins off exactly one Task.
///
/// The Task is judged by a grader — whether it is a faithful hand-off of what
/// the human wanted — and never executed. The grader passes a Task that only
/// describes the delay in words: the Comms Session has no scheduling tool, so
/// turning words into a timed Task is the next Worker's job.
#[tokio::test]
#[ignore = "spends money on a real model; cargo test -- --ignored"]
async fn greet_again() -> Result<(), Trip> {
    unimplemented!()
}

/// A planning Task seeded from outside: greet the human in 3 minutes.
///
/// The planner should spin off exactly one Task scheduled ~3 minutes out, and
/// complete. The whole swarm runs. The scheduled Task is cancelled unexecuted
/// once the planner is done — a case that waits for work it no longer cares
/// about wastes money.
#[tokio::test]
#[ignore = "spends money on a real model; cargo test -- --ignored"]
async fn plan_greet() -> Result<(), Trip> {
    unimplemented!()
}

/// The unit bench: one Task, the real prompt and the real model, every tool call
/// intercepted.
///
/// This asks what the model *reaches for* rather than what the swarm eventually
/// produced. No child Worker runs, no web search happens, and the whole case is
/// one Session's decisions.
#[tokio::test]
#[ignore = "spends money on a real model; cargo test -- --ignored"]
async fn planner_schedules_the_greeting() -> Result<(), Trip> {
    unimplemented!()
}
