//! Scenario: a human says a plain **"Hello :)"** on a fresh Channel.
//!
//! There is no work behind a greeting. The only thing being measured is
//! whether the Comms Session answers in kind and leaves the queue alone — a
//! Session that reaches for `create_task` here has mistaken small talk for a
//! Brief, which is the one thing this case must never see.

use crate::domain::{ChannelKind, Who};
use crate::harness::Drive;

use super::report::RunReport;
use super::{CheckResult, Rig};

const MESSAGE: &str = "Hello :)";

pub(super) async fn run() -> (Option<Rig>, RunReport) {
	let case = super::find("hello").expect("registered in CASES");
	let mut rig = match Rig::builder()
		.drive(Drive::CommsOnly)
		.channel(ChannelKind::Scripted)
		.build()
		.await
	{
		Ok(rig) => rig,
		Err(trip) => return (None, super::build_failed(case, &trip)),
	};
	rig.tripwire(
		"create_task is never reached for",
		super::at_most_creations(0),
	);

	let outcome = async {
		rig.converse(MESSAGE).await?;
		let replied = rig.transcript()?.iter().any(|u| u.who == Who::Sandman);
		Ok(vec![if replied {
			CheckResult::ok("replied", "the comms session said something back")
		} else {
			CheckResult::no("replied", "no reply appeared in the transcript")
		}])
	}
	.await;

	super::finish(case, rig, outcome, Vec::new()).await
}

super::bench_test!(hello);
