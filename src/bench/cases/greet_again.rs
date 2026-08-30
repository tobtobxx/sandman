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
use crate::domain::ChannelKind;
use crate::harness::Drive;
use crate::roles::ToolName;

use super::{CheckResult, Rig};

const MESSAGE: &str = "Hey! Could you greet me again in about 3 minutes? :)";

super::bench_case! {
	name: "greet-again",
	builder: Rig::builder()
		.drive(Drive::CommsOnly)
		.channel(ChannelKind::Scripted)
		.tools(ToolsChoice::Intercept(Box::new(|call| match call.name {
			ToolName::CreateTask => {
				Answer::Say("Created t-99. It will be handled.".to_string())
			},
			_ => Answer::Deny("not available in this case".to_string()),
		}))),
	tripwires: [
		("create_task is reached for at most once", super::at_most_creations(1)),
	],
	body: |rig, graders| {
		rig.converse(MESSAGE).await?;

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
}
