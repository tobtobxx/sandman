//! The bench driver: the reporting way to run the cases.
//!
//! ```text
//! cargo run --bin bench                     all cases, once, in parallel
//! cargo run --bin bench -- --case hello     only the named case(s)
//! cargo run --bin bench -- --times 5        each case N times, for variance
//! cargo run --bin bench -- --serial         one at a time
//! ```
//!
//! The same cases run under `cargo test -- --ignored`, where a failure reads as
//! an ordinary test failure. This binary exists for what a test runner does not
//! give: several runs of one case, a pass rate and a mean, and the artifacts
//! kept under `bench/runs/<stamp>/`.
//!
//! Parallel runs hit the model concurrently, and rate limiting under load shows
//! up as inflated wall time rather than as failures. Keep that in mind when
//! reading variance across `--times`.
//!
//! Every case builds its own [`sandman::bench::Rig`] — its own database, its own
//! log, its own id counters — so running them together in one process is honest.
//!
//! Cases live in `sandman::bench::cases`, and the same table backs the
//! `#[cfg(test)]` wrapper next to each one.

use sandman::bench::report::{self, CaseSummary, RunReport};
use sandman::bench::{Case, CASES};

/// What the driver was asked to do.
struct Args {
	/// Resolved against `CASES` while parsing, so an unknown `--case` is an
	/// error before anything is built or spent.
	cases: Vec<&'static Case>,
	times: usize,
	serial: bool,
	/// Where the artifacts go. `bench/runs` by default.
	out: std::path::PathBuf,
}

#[derive(clap::Parser)]
#[command(
	name = "bench",
	about = "Run Sandman's bench cases against a real model"
)]
struct Cli {
	/// Only these cases. Repeatable; omit for every case.
	#[arg(long = "case")]
	case: Vec<String>,
	/// Run each case this many times, for variance.
	#[arg(long, default_value_t = 1)]
	times: usize,
	/// One case at a time instead of all at once.
	#[arg(long)]
	serial: bool,
	/// Where the artifacts go.
	#[arg(long, default_value = "bench/runs")]
	out: std::path::PathBuf,
}

fn parse(argv: &[String]) -> Result<Args, String> {
	use clap::Parser;

	let cli = Cli::try_parse_from(argv).unwrap_or_else(|e| e.exit());

	let cases: Vec<&'static Case> = if cli.case.is_empty() {
		CASES.iter().collect()
	} else {
		cli.case
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

	Ok(Args {
		cases,
		times: cli.times.max(1),
		serial: cli.serial,
		out: cli.out,
	})
}

/// Run one case once, write its artifacts, and report.
///
/// A case that could not build its Rig still reports; only writing the artifacts
/// fails here.
async fn run_once(
	case: &Case,
	dir: &std::path::Path,
) -> Result<RunReport, String> {
	let (rig, report) = (case.run)().await;
	if let Some(rig) = rig {
		report::write_artifacts(&report, &rig, dir).map_err(|e| {
			format!("could not write artifacts to {}: {e}", dir.display())
		})?;
	}
	Ok(report)
}

/// Pass rate, mean wall time and total cost — swarm Spend plus grader cost —
/// per case, in the order cases were first seen.
fn summarize(reports: &[(String, RunReport)]) -> Vec<CaseSummary> {
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

#[tokio::main]
async fn main() {
	let argv: Vec<String> = std::env::args().collect();
	let args = match parse(&argv) {
		Ok(args) => args,
		Err(message) => {
			eprintln!("{message}");
			std::process::exit(1);
		},
	};

	let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
	let mut reports: Vec<(String, RunReport)> = Vec::new();

	if args.serial {
		for &case in &args.cases {
			for k in 1..=args.times {
				let dir = report::run_dir(&args.out, &stamp, case.name, k);
				match run_once(case, &dir).await {
					Ok(report) => {
						report::print_run(&report);
						reports.push((case.name.to_string(), report));
					},
					Err(e) => eprintln!("{}: {e}", case.name),
				}
			}
		}
	} else {
		let mut handles = Vec::new();
		for &case in &args.cases {
			for k in 1..=args.times {
				let dir = report::run_dir(&args.out, &stamp, case.name, k);
				handles.push(tokio::spawn(async move {
					(case.name, run_once(case, &dir).await)
				}));
			}
		}
		for handle in handles {
			match handle.await {
				Ok((name, Ok(report))) => {
					report::print_run(&report);
					reports.push((name.to_string(), report));
				},
				Ok((name, Err(e))) => eprintln!("{name}: {e}"),
				Err(e) => eprintln!("a case task panicked: {e}"),
			}
		}
	}

	report::print_summary(&summarize(&reports));
}
