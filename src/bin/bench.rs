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
//! Cases live in `tests/cases/`, which this binary and the test harness share.

/// What the driver was asked to do.
struct Args {
	cases: Option<Vec<String>>,
	times: usize,
	serial: bool,
	/// Where the artifacts go. `bench/runs` by default.
	out: std::path::PathBuf,
}

fn parse(_argv: &[String]) -> Result<Args, String> {
	unimplemented!()
}

/// Run one case once, write its artifacts, and report.
async fn run_once(
	_name: &str,
	_dir: &std::path::Path,
) -> Result<sandman::bench::report::RunReport, String> {
	unimplemented!()
}

#[tokio::main]
async fn main() {
	unimplemented!()
}
