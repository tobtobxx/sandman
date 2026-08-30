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

use sandman::bench::{CheckResult, Rig, Trip, Watch};
use sandman::harness::Drive;

/// What the human says in the `hello` case.
const HELLO_MESSAGE: &str = "Hello :)";

/// What the human says in the `greet-again` case.
const GREET_MESSAGE: &str = "Hey! Could you greet me again in about 3 minutes? :)";

/// Tripwire: `create_task` is never reached for more than `n` times.
///
/// Counts calls, not rows: a unit bench answers the creation itself, so a
/// Session that keeps asking for Tasks leaves nothing in the Store to count.
fn at_most_creations(_n: usize) -> impl Fn(&Watch) -> CheckResult + Send + Sync {
    |_watch| unimplemented!()
}

/// `"Hello :)"` gets a reply and reaches for no Task at all.
///
/// One Comms Session, nothing driving the queue: reaching for `create_task` is
/// the failure, so the case denies it and asserts it was never called.
#[tokio::test]
#[ignore = "spends money on a real model; cargo test -- --ignored"]
async fn hello() -> Result<(), Trip> {
    unimplemented!()
}

/// Asking to be greeted again in ~3 minutes reaches for `create_task` once.
///
/// The Brief it hands over is judged by a grader — whether it is a faithful
/// hand-off of what the human wanted — because no count answers that. The grader
/// passes a Brief that only describes the delay in words: the Comms Session has
/// no scheduling tool, so turning words into a timed Task is the next Worker's
/// job.
#[tokio::test]
#[ignore = "spends money on a real model; cargo test -- --ignored"]
async fn greet_again() -> Result<(), Trip> {
    unimplemented!()
}

/// A planning Task seeded from outside: greet the human in 3 minutes.
///
/// The planner should reach for `create_task` exactly once, with a Schedule
/// ~3 minutes out, and complete. The creation is answered by the case, so no
/// child Worker runs and no scheduled Task is left to cancel — the whole case is
/// one Session's decisions.
#[tokio::test]
#[ignore = "spends money on a real model; cargo test -- --ignored"]
async fn plan_greet() -> Result<(), Trip> {
    unimplemented!()
}
