//! Verification a model has to do.
//!
//! Some outcomes no read of the state can judge. That exactly one Task was
//! created is a count; that it is *the Task that was wanted* — that it kept the
//! greeting, kept the delay, and added nothing — is a judgement.
//!
//! A grader is one model call against the same model the swarm uses, because a
//! grader must be as good as the combination being measured. It is bench
//! machinery and not part of the swarm: the call goes straight to the model, not
//! through the scheduler, and what it costs is reported separately and never
//! counts as Spend.
//!
//! Graders run only after every goal check has passed. There is nothing to judge
//! about a run that already failed on something countable.
//!
//! **A reply with no verdict in it is a FAIL.** An unparseable judgement must
//! never quietly pass.
//!
//! Defines: [`Grader`], [`GraderOutcome`], [`Verdict`], [`run`], [`default_judge`].

use crate::domain::Cost;

/// What a grader is told it is doing.
pub const GRADER_SYSTEM: &str = "\
You grade the outcome of an agent swarm against what was wanted.
Be strict and literal: grade what is written, not what was probably meant.
End your reply with a verdict on its own line: <verdict>pass</verdict> or <verdict>fail</verdict>.";

/// A model's judgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
}

/// One question put to a model about a finished run.
pub struct Grader {
    pub name: String,
    /// The whole user message the grader judges. Built from the run's state by
    /// the case that owns it.
    pub input: String,
    /// How to read the reply. [`default_judge`] looks for the verdict tag.
    pub judge: Option<Box<dyn Fn(&str) -> (Verdict, String) + Send + Sync>>,
}

/// What one grader found.
#[derive(Debug, Clone)]
pub struct GraderOutcome {
    pub name: String,
    pub verdict: Verdict,
    pub detail: String,
    /// The grader's whole reply, kept for when a marginal verdict needs reading.
    pub raw: String,
    pub cost: Cost,
}

/// Run one grader.
///
/// Fails only on transport trouble; a `fail` verdict is a normal outcome, not an
/// error.
pub async fn run(_grader: &Grader) -> Result<GraderOutcome, crate::model::ModelError> {
    unimplemented!()
}

/// Look for `<verdict>pass</verdict>` or `<verdict>fail</verdict>`.
///
/// No tag is a FAIL.
pub fn default_judge(_reply: &str) -> (Verdict, String) {
    unimplemented!()
}
