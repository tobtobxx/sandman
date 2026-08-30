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

use crate::domain::{CallStatus, Cost, Spend};

use super::grader::Verdict;
use super::{CheckResult, Grader, GraderOutcome, Trip};

/// Everything one run of one case found.
#[derive(Debug, Clone, serde::Serialize)]
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
	case: &super::Case,
	rig: &mut super::Rig,
	found: Result<Vec<CheckResult>, Trip>,
	graders: Vec<Grader>,
) -> RunReport {
	let started_at = rig.started_at();
	let finished_at = rig.harness.now();

	let (checks, tripped) = match found {
		Ok(checks) => (checks, None),
		Err(trip) => (Vec::new(), Some(trip.to_string())),
	};
	let checks_passed = tripped.is_none() && checks.iter().all(|c| c.ok);

	let mut grader_outcomes = Vec::new();
	let mut grader_cost = Cost(0);
	if checks_passed {
		for grader in &graders {
			match super::grader::run(grader).await {
				Ok(outcome) => {
					grader_cost = grader_cost + outcome.cost;
					grader_outcomes.push(outcome);
				},
				Err(e) => grader_outcomes.push(GraderOutcome {
					name: grader.name.clone(),
					verdict: Verdict::Fail,
					detail: format!("the grader could not be reached: {e}"),
					raw: String::new(),
					cost: Cost(0),
				}),
			}
		}
	}

	rig.wind_down().await;

	let snapshot = rig.store.snapshot().ok();
	let model = snapshot
		.as_ref()
		.map(|s| s.run.model.clone())
		.unwrap_or_default();
	let failed_calls = snapshot
		.map(|s| s.calls)
		.unwrap_or_default()
		.into_iter()
		.filter_map(|call| match call.status {
			CallStatus::Failed { error, .. } => {
				Some(format!("{} ({}): {error}", call.id, call.session))
			},
			_ => None,
		})
		.collect();
	let spend = rig.spend().unwrap_or_default();

	let pass = checks_passed
		&& grader_outcomes.iter().all(|g| g.verdict == Verdict::Pass);

	RunReport {
		case: case.name.to_string(),
		description: case.description.to_string(),
		model,
		// Nothing between here and the Model seam records what reasoning
		// effort a real call asked for; `OpenRouter::from_env` always asks
		// for none. Honest until a case needs to vary it.
		reasoning_effort: "none".to_string(),
		started_at: started_at.0,
		finished_at: finished_at.0,
		wall_ms: (finished_at.0 - started_at.0).max(0),
		pass,
		tripped,
		checks,
		graders: grader_outcomes,
		failed_calls,
		spend,
		grader_cost,
	}
}

/// Write `result.json`, `store.sqlite` and `sandman.log` into a directory.
pub fn write_artifacts(
	report: &RunReport,
	rig: &super::Rig,
	dir: &std::path::Path,
) -> std::io::Result<()> {
	rig.save_to(dir)?;
	let json = serde_json::to_string_pretty(report).map_err(|e| {
		std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
	})?;
	std::fs::write(dir.join("result.json"), json)?;
	Ok(())
}

/// One line per run, then a line per failed check.
pub fn print_run(report: &RunReport) {
	let verdict = if report.pass { "PASS" } else { "FAIL" };
	println!(
		"[{verdict}] {} ({}ms, {}) — {}",
		report.case, report.wall_ms, report.spend.cost, report.description
	);
	if let Some(trip) = &report.tripped {
		println!("  tripped: {trip}");
	}
	for check in &report.checks {
		if !check.ok {
			println!("  check `{}` failed: {}", check.name, check.detail);
		}
	}
	for grader in &report.graders {
		if grader.verdict != Verdict::Pass {
			println!("  grader `{}` failed: {}", grader.name, grader.detail);
		}
	}
	for failed in &report.failed_calls {
		println!("  call failed: {failed}");
	}
}

/// Pass rate, mean wall time and total cost per case.
pub fn print_summary(summaries: &[CaseSummary]) {
	for summary in summaries {
		println!(
			"{}: {}/{} passed, mean {}ms, total {}",
			summary.case,
			summary.passed,
			summary.runs,
			summary.mean_wall_ms,
			summary.total_cost
		);
	}
}

/// Where a run's artifacts go: `bench/runs/<stamp>/<case>-run<k>/`.
pub fn run_dir(
	root: &std::path::Path,
	stamp: &str,
	case: &str,
	k: usize,
) -> std::path::PathBuf {
	root.join(stamp).join(format!("{case}-run{k}"))
}
