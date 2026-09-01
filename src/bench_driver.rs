//! Bench subcommand driver — previously `src/bin/bench.rs`.
//!
//! What it is: the CLI loop that runs each `Case` through a real model,
//! keeps `N` runs per case, and persists artifacts under
//! `bench/runs/<stamp>/` with pass rate and mean. No Harness wiring of its
//! own — `Rig` owns the four seams (`Model`, `ToolRunner`, `Clock`,
//! `Embedder`).
//!
//! Construct: `BenchFlags` from `cli` → `bench(flags, config_flag)` validates
//! `--case` names against `CASES`, loads `Config` from `--config`, and threads
//! it to each `Rig`. Use: `main` on `Cmd::Bench` → `bench` →
//! `bench_run_once(case, dir, config)` → `report::write_artifacts` /
//! `report::print_run`. Consumers: `main` only.
//!
//! Call trace:
//! ```text
//! bench → CASES / find(name) → bench_run_once → (case.run)(config) → write_artifacts
//!       → summarize → report::print_summary
//! ```
//!
//! Rules: one `Rig` per run (private DB/log/counters) so parallel runs are
//! honest. `--list` exits before any spend. Announce before await so silence
//! does not read as hang. `--config` is honored for every run.

use std::path::PathBuf;
use std::sync::Arc;

use sandman::bench::report::{self, CaseSummary, RunReport};
use sandman::bench::{Case, CASES};
use sandman::config::Config;

use crate::cli::BenchFlags;

/// List every case.
///
/// Prints padded name and dim description, one per line.
fn bench_list() {
	let on = sandman::bench::color::enabled();
	let width = CASES.iter().map(|c| c.name.len()).max().unwrap_or(0);
	for case in CASES {
		let name = format!("{:width$}", case.name);
		println!(
			"{}  {}",
			sandman::bench::color::bold(on, &name),
			sandman::bench::color::dim(on, case.description),
		);
	}
}

/// Run one case and persist its artifacts.
///
/// Executes `(case.run)(config)` and writes `result.json`/`store.sqlite`/`sandman.log`.
/// Returns `RunReport`; only artifact write failure becomes `Err`.
async fn bench_run_once(
	case: &Case,
	dir: &std::path::Path,
	config: Arc<Config>,
) -> Result<RunReport, String> {
	let (rig, report) = (case.run)(config).await;
	if let Some(rig) = rig {
		report::write_artifacts(&report, &rig, dir).map_err(|e| {
			format!("could not write artifacts to {}: {e}", dir.display())
		})?;
	}
	Ok(report)
}

/// Announce a run before awaiting it.
///
/// Prevents silence that reads as hang while a real model call is in flight.
fn bench_announce(on: bool, case: &str, k: usize, times: usize) {
	let what = if times > 1 {
		format!("{case} (run {k}/{times})")
	} else {
		case.to_string()
	};
	println!("{} {what}", sandman::bench::color::cyan(on, "→"));
}

/// Summarize runs per case.
///
/// Computes pass count, mean wall time, and combined cost in first-seen order.
fn bench_summarize(reports: &[(String, RunReport)]) -> Vec<CaseSummary> {
	let mut order: Vec<String> = Vec::new();
	let mut by_case: std::collections::HashMap<String, Vec<&RunReport>> =
		std::collections::HashMap::new();
	for (name, report) in reports {
		by_case
			.entry(name.clone())
			.or_insert_with(|| {
				order.push(name.clone());
				Vec::new()
			})
			.push(report);
	}
	order
		.into_iter()
		.map(|name| {
			let runs = &by_case[&name];
			let passed = runs.iter().filter(|r| r.pass).count();
			let mean_wall_ms =
				runs.iter().map(|r| r.wall_ms).sum::<i64>() / runs.len() as i64;
			let total_cost =
				runs.iter().fold(sandman::domain::Cost(0), |acc, r| {
					acc + r.spend.cost + r.grader_cost
				});
			CaseSummary {
				case: name,
				runs: runs.len(),
				passed,
				mean_wall_ms,
				total_cost,
			}
		})
		.collect()
}

/// Execute the bench subcommand.
pub async fn bench(
	flags: BenchFlags,
	config_flag: Option<PathBuf>,
) -> Result<(), String> {
	if flags.list {
		bench_list();
		return Ok(());
	}

	let times = flags.times.max(1);

	let cases: Vec<&'static Case> = if flags.case.is_empty() {
		CASES.iter().collect()
	} else {
		flags
			.case
			.iter()
			.map(|name| {
				sandman::bench::cases::find(name).ok_or_else(|| {
					let known: Vec<&str> =
						CASES.iter().map(|c| c.name).collect();
					format!(
						"`{name}` is not a case. Known cases: {}",
						known.join(", ")
					)
				})
			})
			.collect::<Result<Vec<_>, String>>()?
	};

	let path =
		Config::path(config_flag).map_err(|e| e.to_string())?;
	let config = Config::load(&path).map_err(|e| e.to_string())?;
	let config = Arc::new(config);

	let on = sandman::bench::color::enabled();
	let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
	let total = cases.len() * times;
	println!(
		"{}",
		sandman::bench::color::bold(
			on,
			&format!(
				"Running {} case(s), {} time(s) each — {total} run(s) total",
				cases.len(),
				times
			)
		)
	);

	let mut reports: Vec<(String, RunReport)> = Vec::new();
	let mut done = 0usize;

	if flags.serial {
		for &case in &cases {
			for k in 1..=times {
				bench_announce(on, case.name, k, times);
				let dir = report::run_dir(&flags.out, &stamp, case.name, k);
				done += 1;
				match bench_run_once(case, &dir, config.clone()).await {
					Ok(report) => {
						print!(
							"{} ",
							sandman::bench::color::dim(
								on,
								&format!("[{done}/{total}]")
							)
						);
						report::print_run(&report);
						reports.push((case.name.to_string(), report));
					},
					Err(e) => eprintln!("{}: {e}", case.name),
				}
			}
		}
	} else {
		let mut set = tokio::task::JoinSet::new();
		for &case in &cases {
			for k in 1..=times {
				bench_announce(on, case.name, k, times);
				let dir = report::run_dir(&flags.out, &stamp, case.name, k);
				let cfg = config.clone();
				set.spawn(
					async move { (case.name, bench_run_once(case, &dir, cfg).await) },
				);
			}
		}
		while let Some(outcome) = set.join_next().await {
			done += 1;
			match outcome {
				Ok((name, Ok(report))) => {
					print!(
						"{} ",
						sandman::bench::color::dim(
							on,
							&format!("[{done}/{total}]")
						)
					);
					report::print_run(&report);
					reports.push((name.to_string(), report));
				},
				Ok((name, Err(e))) => eprintln!("{name}: {e}"),
				Err(e) => eprintln!("a case task panicked: {e}"),
			}
		}
	}

	report::print_summary(&bench_summarize(&reports));
	Ok(())
}
