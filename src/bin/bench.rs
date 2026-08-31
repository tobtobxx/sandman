//! bench — reporting driver for `sandman::bench` cases.
//!
//! What it is: the `cargo test -- --ignored` counterpart that runs each
//! `Case` through a real model, keeps `N` runs per case, and persists
//! artifacts under `bench/runs/<stamp>/` with pass rate and mean.
//!
//! Construct: `Cli` (clap) → `Args` via `parse()` (names resolved against `CASES`).
//! Use: `main()` → `run_once(case, dir)` → `RunReport` → `report::write_artifacts` / `report::print_run`.
//! Consumers: `report::print_run` per completed run, `report::print_summary` after all.
//!
//! ```text
//! cargo run --bin bench                     all cases, once, in parallel
//! cargo run --bin bench -- --list           list cases, run none
//! cargo run --bin bench -- --case hello     only named case(s)
//! cargo run --bin bench -- --times 5        each case N times, for variance
//! cargo run --bin bench -- --serial         one at a time
//! ```
//!
//! Call trace:
//! ```text
//! main → parse(argv) → CASES / find(name)
//!      → run_once → (case.run)() → report::write_artifacts
//!      → summarize → report::print_summary
//! ```
//!
//! Seams: none of its own — `Rig` owns the four (`Model`, `ToolRunner`,
//! `Clock`, `Embedder`); ordering (`--serial` vs `JoinSet`) and artifact
//! root (`--out`) are the only choices here.
//!
//! Rules: one `Rig` per run (private DB/log/counters) so parallel runs are honest.
//! Rate limiting surfaces as wall time, not failure. `--list` exits before
//! any spend. Announce before await so silence does not read as hang.

use sandman::bench::report::{self, CaseSummary, RunReport};
use sandman::bench::{Case, CASES};

/// Resolved driver arguments.
///
/// Cases validated against `CASES`; unknown `--case` is an error before
/// any Rig is built or spend incurred.
struct Args {
	cases: Vec<&'static Case>,
	times: usize,
	serial: bool,
	out: std::path::PathBuf,
}

#[derive(clap::Parser)]
#[command(
	name = "bench",
	about = "Run Sandman's bench cases against a real model"
)]
struct Cli {
	/// Say which cases there are, and run none of them.
	#[arg(long)]
	list: bool,
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

/// Parse CLI argv into `Args`.
///
/// Exits on `--list`; validates `--case` names against `CASES`.
fn parse(argv: &[String]) -> Result<Args, String> {
	// Parse CLI
	use clap::Parser;
	let cli = Cli::try_parse_from(argv).unwrap_or_else(|e| e.exit());

	// Handle list flag
	if cli.list {
		list();
		std::process::exit(0);
	}

	// Resolve cases
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

	// Build args
	Ok(Args {
		cases,
		times: cli.times.max(1),
		serial: cli.serial,
		out: cli.out,
	})
}

/// List every case.
///
/// Prints padded name and dim description, one per line.
fn list() {
	// Measure width
	let on = sandman::bench::color::enabled();
	let width = CASES.iter().map(|c| c.name.len()).max().unwrap_or(0);

	// Print cases
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
/// Executes `case.run()` and writes `result.json`/`store.sqlite`/`sandman.log`.
/// Returns `RunReport`; only artifact write failure becomes `Err`.
async fn run_once(
	case: &Case,
	dir: &std::path::Path,
) -> Result<RunReport, String> {
	// Run case
	let (rig, report) = (case.run)().await;

	// Write artifacts
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
fn announce(on: bool, case: &str, k: usize, times: usize) {
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
fn summarize(reports: &[(String, RunReport)]) -> Vec<CaseSummary> {
	// Group by case
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

	// Summarize each case
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
	// Parse arguments
	let argv: Vec<String> = std::env::args().collect();
	let args = match parse(&argv) {
		Ok(args) => args,
		Err(message) => {
			eprintln!("{message}");
			std::process::exit(1);
		},
	};

	// Build run context
	let on = sandman::bench::color::enabled();
	let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
	let total = args.cases.len() * args.times;
	println!(
		"{}",
		sandman::bench::color::bold(
			on,
			&format!(
				"Running {} case(s), {} time(s) each — {total} run(s) total",
				args.cases.len(),
				args.times
			)
		)
	);

	// Prepare collection
	let mut reports: Vec<(String, RunReport)> = Vec::new();
	let mut done = 0usize;

	// Run cases
	if args.serial {
		// Run serially
		for &case in &args.cases {
			for k in 1..=args.times {
				announce(on, case.name, k, args.times);
				let dir = report::run_dir(&args.out, &stamp, case.name, k);
				done += 1;
				match run_once(case, &dir).await {
					// Run succeeded - print and collect
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
					// Artifact write failed - report error
					Err(e) => eprintln!("{}: {e}", case.name),
				}
			}
		}
	} else {
		// Run in parallel
		let mut set = tokio::task::JoinSet::new();
		for &case in &args.cases {
			for k in 1..=args.times {
				announce(on, case.name, k, args.times);
				let dir = report::run_dir(&args.out, &stamp, case.name, k);
				set.spawn(
					async move { (case.name, run_once(case, &dir).await) },
				);
			}
		}
		// Drain completions
		while let Some(outcome) = set.join_next().await {
			done += 1;
			match outcome {
				// Run succeeded - print and collect
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
				// Artifact write failed - report error
				Ok((name, Err(e))) => eprintln!("{name}: {e}"),
				// Task panicked - report panic
				Err(e) => eprintln!("a case task panicked: {e}"),
			}
		}
	}

	// Print summary
	report::print_summary(&summarize(&reports));
}
