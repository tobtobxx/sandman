//! Scenario: a human asks, in their own words, **"Could you greet me again in
//! about 3 minutes?"**
//!
//! The Comms Session has no scheduling tool of its own — its only Task-creating
//! tool is the plain `create_task`, which carries no timing. Turning "in about
//! 3 minutes" into an actual Schedule is the next Worker's job; what this case
//! measures is whether the Comms Session hands the request off at all, and
//! whether the Brief it writes still says what the human asked for. That is a
//! judgement, not a count, so a grader reads it: it must keep the greeting and
//! keep the delay, and add nothing that was not there.

use crate::bench::grader::Grader;
use crate::bench::intercept::{Answer, ToolsChoice};
use crate::bench::report::{self, RunReport};
use crate::bench::{CheckResult, Rig};
use crate::domain::ChannelKind;
use crate::harness::Drive;
use crate::roles::ToolName;

const MESSAGE: &str = "Hey! Could you greet me again in about 3 minutes? :)";

pub(super) async fn run() -> (Option<Rig>, RunReport) {
	let case = super::find("greet-again").expect("registered in CASES");

	let mut rig = match Rig::builder()
		.drive(Drive::CommsOnly)
		.channel(ChannelKind::Scripted)
		.tools(ToolsChoice::Intercept(Box::new(|call| match call.name {
			ToolName::CreateTask => {
				Answer::Say("Created t-99. It will be handled.".to_string())
			},
			_ => Answer::Deny("not available in this case".to_string()),
		})))
		.build()
		.await
	{
		Ok(rig) => rig,
		Err(trip) => return (None, super::build_failed(case, &trip)),
	};

	rig.tripwire(
		"create_task is reached for at most once",
		super::at_most_creations(1),
	);

	let mut graders = Vec::new();
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

		let creations = rig.interceptor.calls_to(ToolName::CreateTask);
		let checks = vec![if creations.len() == 1 {
			CheckResult::ok("handed off", "create_task was called once")
		} else {
			CheckResult::no(
				"handed off",
				format!("create_task was called {} time(s)", creations.len()),
			)
		}];

		if let Some(call) = creations.first() {
			let brief = call
				.args
				.get("brief")
				.and_then(|v| v.as_str())
				.unwrap_or("<no brief field>");
			graders.push(Grader {
				name: "brief keeps the ask".to_string(),
				input: format!(
					"The human asked: \"{MESSAGE}\"\n\n\
					 A Comms Session handed this off as a Brief for the next \
					 Worker to read, with no other context. Here is that \
					 Brief:\n\n\"{brief}\"\n\n\
					 Does the Brief still say the human wants to be greeted \
					 again, and still say the delay is about 3 minutes, \
					 without inventing anything the human did not ask for?"
				),
				judge: None,
			});
		}

		Ok(checks)
	}
	.await;

	let report = report::assemble(case, &mut rig, outcome, graders).await;
	(Some(rig), report)
}

#[cfg(test)]
mod tests {
	use super::run;

	#[tokio::test]
	#[ignore = "spends money on a real model; cargo test -- --ignored"]
	async fn greet_again() {
		let (_, report) = run().await;
		if !report.pass {
			panic!("greet-again did not pass: {report:#?}");
		}
	}
}
