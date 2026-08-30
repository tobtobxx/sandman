//! The bench: measuring what the model does inside Sandman.
//!
//! It tests the harness-and-model combination against the real system prompts,
//! the real tools and the real scheduler. It exists to inform prompt and
//! mechanics changes.
//!
//! **A case is an ordinary test.** No process of its own, no working directory
//! to move, no globals to keep apart. A [`Rig`] owns a private in-memory
//! database, its own id counters, its own log file and its own Harness, so two
//! cases in one process share nothing. That is the whole isolation story.
//!
//! **A case waits on the Event stream, not on a clock.** [`Rig::until`] wakes
//! when something actually changed and evaluates every tripwire on the way past.
//! Nothing polls, and nothing has to throw across a polling loop to stop a run.
//!
//! **Everything a case does not want to be real can be replaced**, at one of
//! four seams: the model, the tool runner, the clock, the embedder. The
//! interesting one is the tool runner. Intercepting it is how a unit bench asks
//! the real model, with the real prompt for one real Task, what it would *do* —
//! and answers every tool call itself instead of paying for the work behind it.
//!
//! Files: [`rig`] the harness under test; [`intercept`] watching and answering
//! tool calls; [`script`] a model whose replies are written by the test;
//! [`grader`] verification a model has to do; [`report`] what a run leaves
//! behind.
//!
//! Defines: [`Trip`], [`CheckResult`].

pub mod grader;
pub mod intercept;
pub mod report;
pub mod rig;
pub mod script;

pub use grader::{Grader, GraderOutcome, Verdict};
pub use intercept::{Interceptor, RecordedToolCall, ToolsChoice};
pub use rig::{Rig, RigBuilder};
pub use script::ScriptedModel;

/// Why a run stopped early.
///
/// A tripwire is "this must never happen", and it is evaluated continuously
/// while a case drives. Tripping one ends the run at once, so a looping swarm
/// costs at most a call or two past the violation.
///
/// This is a value a test propagates with `?`, not a panic and not a process
/// exit. The [`Rig`] winds itself down on the way out either way.
#[derive(Debug, thiserror::Error)]
pub enum Trip {
    #[error("tripwire `{name}`: {detail}")]
    Tripwire { name: String, detail: String },
    #[error("timed out waiting for {what}")]
    Timeout { what: String },
    #[error("the whole case ran past its {seconds}s bound")]
    CaseTimeout { seconds: u64 },
    #[error(transparent)]
    Store(#[from] crate::store::StoreError),
}

/// What one check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub name: String,
    pub ok: bool,
    /// What was seen, so a failure reads without opening the artifacts.
    pub detail: String,
}

impl CheckResult {
    pub fn ok(_name: &str, _detail: impl Into<String>) -> Self {
        unimplemented!()
    }

    pub fn no(_name: &str, _detail: impl Into<String>) -> Self {
        unimplemented!()
    }
}
