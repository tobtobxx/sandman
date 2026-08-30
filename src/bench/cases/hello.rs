//! Scenario: a human says a plain **"Hello :)"** on a fresh Channel.
//!
//! There is no work behind a greeting. The only thing being measured is
//! whether the Comms Session answers in kind and leaves the queue alone — a
//! Session that reaches for `create_task` here has mistaken small talk for a
//! Brief, which is the one thing this case must never see.

use crate::bench::report::{self, RunReport};
use crate::bench::{CheckResult, Rig};
use crate::domain::{ChannelKind, Who};
use crate::harness::Drive;

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
		let channel = rig.channel.expect("build() opened one");
		let session = rig
			.store
			.channel_session(channel)?
			.expect("a comms session stands on this channel");
		let baseline = rig
			.store
			.session(session)?
			.map(|s| s.calls.len())
			.unwrap_or(0);

		rig.send(MESSAGE).await;
		rig.until("the comms session finishes replying", move |store| {
			super::comms_finished(store, session, baseline)
		})
		.await?;

		let transcript = rig.transcript()?;
		let replied = transcript.iter().any(|u| u.who == Who::Sandman);

		Ok(vec![if replied {
			CheckResult::ok("replied", "the comms session said something back")
		} else {
			CheckResult::no("replied", "no reply appeared in the transcript")
		}])
	}
	.await;

	let report = report::assemble(case, &mut rig, outcome, Vec::new()).await;
	(Some(rig), report)
}

#[cfg(test)]
mod tests {
	use super::run;

	#[tokio::test]
	#[ignore = "spends money on a real model; cargo test -- --ignored"]
	async fn hello() {
		let (_, report) = run().await;
		if !report.pass {
			panic!("hello did not pass: {report:#?}");
		}
	}
}
