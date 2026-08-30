//! Scenario: a `planning` Task is seeded directly onto the queue — no human,
//! no Channel — with a Brief asking for a greeting **about 3 minutes from
//! now**.
//!
//! A planning Worker already holds `create_task_full`, the one tool that can
//! choose a Schedule, so unlike the Comms Session it can act on this itself.
//! What this case measures is whether it does: reach for `create_task_full`
//! exactly once, with a delay in the right neighbourhood, and then finish. The
//! creation is answered by the case rather than let run for real, so no child
//! Worker starts and there is nothing left on the queue to clean up — the
//! whole case is one Session's decision.

use crate::bench::intercept::{Answer, ToolsChoice};
use crate::bench::report::{self, RunReport};
use crate::bench::{CheckResult, Rig};
use crate::domain::{
	Brief, Creator, NewTask, Schedule, TaskPriority, TaskState, Title,
};
use crate::harness::Drive;
use crate::roles::{RoleName, ToolName};

/// Loose enough to allow for a model that says "three minutes" and means
/// somewhere close to it, tight enough that "tomorrow" or "right now" both
/// fail it.
const EXPECTED_DELAY_SECONDS: std::ops::RangeInclusive<i64> = 60..=600;

fn brief() -> NewTask {
	NewTask {
		title: Title::try_from("Greet the human again".to_string())
			.expect("a short literal title is never empty"),
		brief: Brief::try_from(
			"Greet the human again in about 3 minutes from now. You have no \
			 Channel of your own to speak on, so schedule a Task to say the \
			 greeting at the right time rather than saying anything yourself."
				.to_string(),
		)
		.expect("a short literal brief is never empty"),
		role: RoleName::Planning,
		schedule: Schedule::Now,
		subscriber: None,
		priority: TaskPriority::default(),
		created_by: Creator::Cli,
	}
}

pub(super) async fn run() -> (Option<Rig>, RunReport) {
	let case = super::find("plan-greet").expect("registered in CASES");

	let mut rig = match Rig::builder()
		.drive(Drive::Full)
		.tools(ToolsChoice::Intercept(Box::new(|call| match call.name {
			ToolName::CreateTaskFull => Answer::Say(
				"Created t-99. It will run on schedule.".to_string(),
			),
			_ => Answer::Deny("not available in this case".to_string()),
		})))
		.build()
		.await
	{
		Ok(rig) => rig,
		Err(trip) => return (None, super::build_failed(case, &trip)),
	};

	rig.tripwire(
		"create_task_full is reached for at most once",
		super::at_most_creations(1),
	);

	let outcome = async {
		let seed = rig.seed_task(brief())?;

		rig.until("the planner completes", move |store| {
			matches!(
				store.task(seed),
				Ok(Some(t)) if matches!(t.state, TaskState::Completed { .. })
			)
		})
		.await?;

		let mut checks = Vec::new();

		let creations = rig.interceptor.calls_to(ToolName::CreateTaskFull);
		checks.push(if creations.len() == 1 {
			CheckResult::ok(
				"scheduled once",
				"create_task_full was called once",
			)
		} else {
			CheckResult::no(
				"scheduled once",
				format!(
					"create_task_full was called {} time(s)",
					creations.len()
				),
			)
		});

		if let Some(call) = creations.first() {
			let delay =
				call.args.get("run_at_seconds").and_then(|v| v.as_i64());
			checks.push(match delay {
				Some(seconds) if EXPECTED_DELAY_SECONDS.contains(&seconds) => {
					CheckResult::ok(
						"delay is about 3 minutes",
						format!("run_at_seconds was {seconds}"),
					)
				},
				Some(seconds) => CheckResult::no(
					"delay is about 3 minutes",
					format!(
						"run_at_seconds was {seconds}, outside \
						 {EXPECTED_DELAY_SECONDS:?}"
					),
				),
				None => CheckResult::no(
					"delay is about 3 minutes",
					"run_at_seconds was not set",
				),
			});
		}

		let task = rig.tasks()?.into_iter().find(|t| t.id == seed);
		let succeeded = matches!(
			task.map(|t| t.state),
			Some(TaskState::Completed { result, .. })
				if matches!(result, crate::domain::TaskResult::Succeeded(_))
		);
		checks.push(if succeeded {
			CheckResult::ok("finished", "the planner submitted a Result")
		} else {
			CheckResult::no(
				"finished",
				"the Task did not complete as a success",
			)
		});

		Ok(checks)
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
	async fn plan_greet() {
		let (_, report) = run().await;
		if !report.pass {
			panic!("plan-greet did not pass: {report:#?}");
		}
	}
}
