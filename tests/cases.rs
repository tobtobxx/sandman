//! The bench cases, as tests.
//!
//! One wrapper per entry in `sandman::bench::cases::CASES`, which is where the
//! cases themselves live — `bin/bench` runs the same table, and it cannot reach
//! into an integration test crate. What a case is, and how a tripwire, a check
//! and a grader differ, is documented there.
//!
//! They are `#[ignore]`d because they spend money on a real model:
//!
//! ```sh
//! cargo test -- --ignored               # all of them
//! cargo test -- --ignored hello         # one
//! cargo run --bin bench -- --times 5    # with a report and artifacts
//! ```

/// Run one case, and fail the test with what it found if it did not pass.
///
/// An unknown name is a failure of this file, not of the case, and says so.
async fn case(_name: &str) {
	unimplemented!()
}

#[tokio::test]
#[ignore = "spends money on a real model; cargo test -- --ignored"]
async fn hello() {
	case("hello").await
}

#[tokio::test]
#[ignore = "spends money on a real model; cargo test -- --ignored"]
async fn greet_again() {
	case("greet-again").await
}

#[tokio::test]
#[ignore = "spends money on a real model; cargo test -- --ignored"]
async fn plan_greet() {
	case("plan-greet").await
}
