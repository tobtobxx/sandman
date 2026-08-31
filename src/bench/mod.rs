//! Bench — what the model reaches for inside Sandman.
//!
//! One Session against one real Brief, real prompts and real scheduler.
//! Every tool call goes through [`intercept::Interceptor`] so the case decides
//! what happens without paying for work behind it. One Rig is one isolated
//! Sandman (private DB, counters, log, Harness); integration is a series of
//! unit benches, not a swarm case.
//!
//! Construct: [`rig::RigBuilder`] → [`rig::Rig`] (model/clock/tools/drive/channel/timeout).
//! Drive: [`Rig::until`] follows the Event stream; predicate and every
//! tripwire re-checked on each Event, no polling.
//! Verify: tripwire (continuous → [`Trip`]) vs check (once → [`CheckResult`])
//! vs grader (model judgement, after checks pass).
//! Report: [`report::assemble`] winds down, waits for in-flight calls, emits
//! `RunReport` + `store.sqlite`/`sandman.log`/`result.json`.
//!
//! Consumers: [`cases::CASES`] via `tests/cases.rs` (`cargo test -- --ignored`)
//! and `bin/bench`; both call `Case::run` → `(Option<Rig>, RunReport)`.
//!
//! Seams — real unless the case replaces them:
//! | Seam | Real | Bench |
//! | --- | --- | --- |
//! | Model | OpenRouter | [`script::ScriptedModel`] / custom |
//! | ToolRunner | `tools::Registry` | [`intercept::Interceptor`] (always) |
//! | Clock | `SystemClock` | `FixedClock` / `ManualClock` |
//! | Embedder | `OpenRouterEmbedder` | test-supplied |
//!
//! Rules: one Session per case; per-Rig isolation, not per-process.
//! Schemas never change, only answers do. A case that waits for a Schedule
//! to fire is about the Harness, not the model.

pub mod cases;
pub mod color;
pub mod grader;
pub mod intercept;
pub mod report;
pub mod rig;
pub mod script;

pub use cases::{Case, CASES};
pub use grader::{Grader, GraderOutcome, Verdict};
pub use intercept::{Interceptor, RecordedToolCall, ToolsChoice};
pub use rig::{Rig, RigBuilder, Watch};
pub use script::ScriptedModel;

/// Early stop reason for a case.
///
/// Tripwire fires continuously on the Event stream; timeout hits the case bound.
/// Propagated with `?`, never a panic; [`Rig`] winds down either way.
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

/// Result of one goal check, evaluated once at the end.
///
/// `ok` with `detail` always stored so `result.json` reads without artifacts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CheckResult {
	pub name: String,
	pub ok: bool,
	/// Human-readable evidence kept in `result.json`.
	pub detail: String,
}

impl CheckResult {
	/// Build a passing result with evidence.
	pub fn ok(name: &str, detail: impl Into<String>) -> Self {
		CheckResult {
			name: name.to_string(),
			ok: true,
			detail: detail.into(),
		}
	}

	/// Build a failing result with evidence.
	pub fn no(name: &str, detail: impl Into<String>) -> Self {
		CheckResult {
			name: name.to_string(),
			ok: false,
			detail: detail.into(),
		}
	}
}
