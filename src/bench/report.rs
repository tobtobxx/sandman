//! What a bench run leaves behind, and what it prints.
//!
//! Three artifacts per run, in a directory the driver names — never the working
//! directory, so several runs from one process cannot write over each other:
//!
//! - `result.json` — pass or fail, every check with what it saw, the trip that
//!   ended it if one did, wall time, Spend, and the graders with their cost kept
//!   apart from the swarm's.
//! - `store.sqlite` — the run's whole database: every Task with its Result, every
//!   Session with its full transcript and its metacognition, every model call
//!   with its request and reply. This is what you open when a run fails and you
//!   want to know why. `sqlite3` reads it.
//! - `sandman.log` — the order, which the database cannot show.
//!
//! Defines: [`RunReport`], [`CaseSummary`], [`write_artifacts`], [`print_summary`].

use crate::domain::{Cost, Spend};

use super::{CheckResult, Grader, GraderOutcome, Trip};

/// Everything one run of one case found.
#[derive(Debug, Clone)]
pub struct RunReport {
	pub case: String,
	pub description: String,
	pub model: String,
	pub reasoning_effort: String,
	pub started_at: i64,
	pub finished_at: i64,
	pub wall_ms: i64,
	pub pass: bool,
	/// Why the run ended early, if it did.
	pub tripped: Option<String>,
	pub checks: Vec<CheckResult>,
	pub graders: Vec<GraderOutcome>,
	/// Calls that never came back, with the wire error — how an expired key
	/// becomes visible without opening the log.
	pub failed_calls: Vec<String>,
	pub spend: Spend,
	/// Kept apart from Spend: a grader is bench machinery, not the swarm.
	pub grader_cost: Cost,
}

/// Several runs of one case, for reading variance.
#[derive(Debug, Clone)]
pub struct CaseSummary {
	pub case: String,
	pub runs: usize,
	pub passed: usize,
	pub mean_wall_ms: i64,
	pub total_cost: Cost,
}

/// Everything between what a case found and its report, done once rather than
/// in each case.
///
/// Winds the Rig down first, so nothing can still spend while the graders run.
/// A [`Trip`] becomes [`RunReport::tripped`] rather than an early return: a run
/// that ended on a tripwire still reports what it saw on the way there. Graders
/// run only if every check passed — there is nothing to judge about a run that
/// already failed on something countable — and their cost is kept apart from
/// Spend.
pub async fn assemble(
	_case: &super::Case,
	_rig: &mut super::Rig,
	_found: Result<Vec<CheckResult>, Trip>,
	_graders: Vec<Grader>,
) -> RunReport {
	unimplemented!()
}

/// Write `result.json`, `store.sqlite` and `sandman.log` into a directory.
pub fn write_artifacts(
	_report: &RunReport,
	_rig: &super::Rig,
	_dir: &std::path::Path,
) -> std::io::Result<()> {
	unimplemented!()
}

/// One line per run, then a line per failed check.
pub fn print_run(_report: &RunReport) {
	unimplemented!()
}

/// Pass rate, mean wall time and total cost per case.
pub fn print_summary(_summaries: &[CaseSummary]) {
	unimplemented!()
}

/// Where a run's artifacts go: `bench/runs/<stamp>/<case>-run<k>/`.
pub fn run_dir(
	_root: &std::path::Path,
	_stamp: &str,
	_case: &str,
	_k: usize,
) -> std::path::PathBuf {
	unimplemented!()
}
