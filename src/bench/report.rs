//! What a bench run leaves behind and prints.
//!
//! Construct: `assemble(case, rig, found, graders) -> RunReport` — winds the
//! rig down, turns a `Trip` into `tripped` rather than an early return, and
//! grades only if every check passed.
//! Use: `write_artifacts(report, rig, dir)` writes `result.json`/`store.sqlite`/`sandman.log`;
//! `print_run`/`print_summary` render verdicts; `run_dir(root, stamp, case, k)` names the dir.
//! Consumers: `cases::finish` (every case) and `bin/bench` (driver loop, summary, artifact root).
//!
//! Artifacts per run — caller-named dir, never cwd, so parallel runs cannot collide:
//!
//! | file | contains | read when |
//! |---|---|---|
//! | `result.json` | pass, checks, tripped, wall time, Spend + grader cost apart | scanning results |
//! | `store.sqlite` | Tasks/Results/Sessions/transcripts/calls with request/reply | run failed, need why |
//! | `sandman.log` | ordered Events | order the DB cannot show |
//!
//! Rules:
//! - **Trip is data, not control** — a tripped run still reports checks seen on the way there.
//! - **Graders run only if checks passed** — nothing to judge on a countable failure.
//! - **Grader cost never in Spend** — bench machinery, not swarm work; `Spend` is re-summed from the store.
//! - **Wind down before snapshot** — no spend after grading starts; cost is honest.
//!
//! Defines: [`RunReport`], [`CaseSummary`], [`assemble`], [`write_artifacts`], [`print_run`], [`print_summary`], [`run_dir`].

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

/// Assemble one run's report.
///
/// Winds the rig down, grades only if every check passed, and records spend.
/// A `Trip` becomes `tripped` rather than an early return.
pub async fn assemble(
	case: &super::Case,
	rig: &mut super::Rig,
	found: Result<Vec<CheckResult>, Trip>,
	graders: Vec<Grader>,
) -> RunReport {
	// Capture timestamps
	let started_at = rig.started_at();
	let finished_at = rig.harness.now();

	// Classify outcome
	let (checks, tripped) = match found {
		Ok(checks) => (checks, None),
		Err(trip) => (Vec::new(), Some(trip.to_string())),
	};
	let checks_passed = tripped.is_none() && checks.iter().all(|c| c.ok);

	// Grade if clean
	let mut grader_outcomes = Vec::new();
	let mut grader_cost = Cost(0);
	if checks_passed {
		for grader in &graders {
			match super::grader::run(grader, rig.config.for_grader()).await {
				// Grader answered - collect cost
				Ok(outcome) => {
					grader_cost = grader_cost + outcome.cost;
					grader_outcomes.push(outcome);
				},
				// Grader unreachable - record failure
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

	// Wind down rig
	rig.wind_down().await;

	// Collect snapshot data
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

	// Build report
	let pass = checks_passed
		&& grader_outcomes.iter().all(|g| g.verdict == Verdict::Pass);

	RunReport {
		case: case.name.to_string(),
		description: case.description.to_string(),
		model,
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

/// Write `result.json`, `store.sqlite` and `sandman.log` into `dir`.
pub fn write_artifacts(
	report: &RunReport,
	rig: &super::Rig,
	dir: &std::path::Path,
) -> std::io::Result<()> {
	rig.save_to(dir)?;
	let json = serde_json::to_string_pretty(report)
		.map_err(|e| std::io::Error::other(e.to_string()))?;
	std::fs::write(dir.join("result.json"), json)?;
	Ok(())
}

/// Print one run's verdict and failures, colored for scan.
pub fn print_run(report: &RunReport) {
	// Resolve verdict
	let on = super::color::enabled();
	let verdict = if report.pass {
		super::color::green(on, "PASS")
	} else {
		super::color::red(on, "FAIL")
	};
	println!(
		"[{verdict}] {} {} {} — {}",
		super::color::bold(on, &report.case),
		super::color::dim(on, &format!("{}ms", report.wall_ms)),
		super::color::dim(on, &report.spend.cost.to_string()),
		report.description
	);

	// Print failures
	let mark = super::color::red(on, "✗");
	if let Some(trip) = &report.tripped {
		println!("  {mark} tripped: {trip}");
	}
	for check in &report.checks {
		if !check.ok {
			println!(
				"  {mark} check `{}` failed: {}",
				check.name, check.detail
			);
		}
	}
	for grader in &report.graders {
		if grader.verdict != Verdict::Pass {
			println!(
				"  {mark} grader `{}` failed: {}",
				grader.name, grader.detail
			);
		}
	}
	for failed in &report.failed_calls {
		println!("  {mark} call failed: {failed}");
	}
}

/// Print pass rate, mean wall time and total cost per case as a table.
pub fn print_summary(summaries: &[CaseSummary]) {
	// Return if empty
	if summaries.is_empty() {
		return;
	}
	let on = super::color::enabled();
	println!("\n{}", super::color::bold(on, "Summary"));

	// Collect totals
	let name_width = summaries.iter().map(|s| s.case.len()).max().unwrap_or(0);
	let mut total_passed = 0;
	let mut total_runs = 0;
	let mut total_cost = Cost(0);

	// Print per case
	for summary in summaries {
		println!(
			"  {:<name_width$}  {} passed  mean {:>6}ms   total {}",
			summary.case,
			ratio(on, summary.passed, summary.runs),
			summary.mean_wall_ms,
			summary.total_cost,
		);
		total_passed += summary.passed;
		total_runs += summary.runs;
		total_cost = total_cost + summary.total_cost;
	}

	// Print totals
	println!(
		"  {} run(s) passed, total spent {}",
		ratio(on, total_passed, total_runs),
		total_cost
	);
}

/// Format `passed/runs` colored by outcome.
fn ratio(on: bool, passed: usize, runs: usize) -> String {
	let text = format!("{passed}/{runs}");
	if runs > 0 && passed == runs {
		super::color::green(on, &text)
	} else if passed == 0 {
		super::color::red(on, &text)
	} else {
		super::color::yellow(on, &text)
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
