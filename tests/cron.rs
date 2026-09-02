//! A cron Task through the Harness: coming due makes work instead of being
//! work.
//!
//! `Store::fire_cron` is covered next to the other Store smoke tests; what is
//! checked here is the routing in `Harness::step` — a due cron Task must reach
//! `fire_cron` and never `new_worker_session`, so no model is ever called for
//! it. The scripted model backing this Harness has no reply to give: if a
//! Session started, it would say so.

use std::sync::Arc;

use sandman::bench::script::ScriptedModel;
use sandman::config::Config;
use sandman::db::Backing;
use sandman::domain::{
	Brief, Clock, Creator, Duration, ManualClock, NewTask, Schedule, SessionId,
	TaskPriority, TaskState, Timestamp, Title,
};
use sandman::event::Events;
use sandman::harness::{CancelOutcome, Drive, Harness};
use sandman::memory::{Embedder, OpenRouterEmbedder};
use sandman::model::{Model, Models};
use sandman::roles::RoleName;
use sandman::scheduler::Scheduler;
use sandman::store::Store;
use sandman::tools::{Registry, Tool};

/// A Harness on a clock that only moves by hand, over an in-memory database.
fn harness(clock: Arc<ManualClock>) -> Arc<Harness> {
	let config = Arc::new(
		Config::parse_with(sandman::config::DEFAULT, &|_| {
			Some("/nonexistent".to_string())
		})
		.expect("the shipped default parses"),
	);
	let events = Arc::new(Events::new(1024));
	let store = Arc::new(
		Store::open(Backing::Memory, events.clone(), "scripted", clock.now())
			.unwrap(),
	);
	let model: Arc<dyn Model> = Arc::new(ScriptedModel::new(Vec::new()));
	let clock: Arc<dyn Clock> = clock;
	let scheduler = Arc::new(Scheduler::new(
		Models::uniform(model),
		store.clone(),
		clock.clone(),
	));
	let embedder: Arc<dyn Embedder> =
		Arc::new(OpenRouterEmbedder::from_spec(&config.embedding));
	Harness::new(
		store,
		events.clone(),
		scheduler,
		Arc::new(Registry::all(events)),
		clock,
		embedder,
		config,
	)
}

#[tokio::test]
async fn a_due_cron_task_makes_a_daughter_instead_of_running() {
	let clock = Arc::new(ManualClock::starting_at(Timestamp(0)));
	let harness = harness(clock.clone());
	let now = clock.now();
	let cron = harness
		.create_task(NewTask {
			title: Title::try_from("water the plants".to_string()).unwrap(),
			brief: Brief::try_from("every minute, on the minute".to_string())
				.unwrap(),
			role: RoleName::Planning,
			schedule: Schedule::parse(None, Some("* * * * *"), now).unwrap(),
			priority: TaskPriority::default(),
			created_by: Creator::Cli,
		})
		.unwrap();

	// Nothing to start before it comes round
	assert!(!harness.step(Drive::Full).await.unwrap());

	// Coming round starts something, and it is not a Session
	let due = harness.store.task(cron).unwrap().unwrap().schedule;
	clock.advance(now.until(due.not_before().unwrap()));
	assert!(harness.step(Drive::Full).await.unwrap());
	assert!(harness.store.snapshot().unwrap().sessions.is_empty());

	let tasks = harness.store.snapshot().unwrap().tasks;
	assert_eq!(tasks.len(), 2);
	let daughter = tasks.iter().find(|t| t.id != cron).unwrap();
	assert_eq!(daughter.schedule, Schedule::Now);
	assert_eq!(
		daughter.brief,
		tasks.iter().find(|t| t.id == cron).unwrap().brief
	);
	assert_eq!(
		daughter.created_by,
		sandman::domain::Creator::CronTask(cron)
	);

	// The cron Task is still pending, armed for the occurrence after
	let parent = harness.store.task(cron).unwrap().unwrap();
	assert_eq!(parent.state, TaskState::Pending);
	assert!(parent.schedule.not_before() > due.not_before());

	// Cancelling it stops the ones to come and leaves the one already out
	assert_eq!(
		harness.cancel_task(cron).await.unwrap(),
		CancelOutcome::Cancelled { running: false }
	);
	assert_eq!(
		harness.store.task(daughter.id).unwrap().unwrap().state,
		TaskState::Pending
	);

	// With the daughter out of the way too, an hour of occurrences passing
	// leaves the queue with nothing to start
	harness.cancel_task(daughter.id).await.unwrap();
	clock.advance(Duration::from_secs(3600));
	assert!(!harness.step(Drive::Full).await.unwrap());
	assert_eq!(harness.store.snapshot().unwrap().tasks.len(), 2);
}

/// A Worker awaiting a cron Task gets an immediate explanation, not a hang:
/// a cron Task never completes, and its schedule variant never changes, so
/// the check before parking is enough.
#[tokio::test]
async fn awaiting_a_cron_task_answers_at_once() {
	let clock = Arc::new(ManualClock::starting_at(Timestamp(0)));
	let harness = harness(clock.clone());
	let cron = harness
		.create_task(NewTask {
			title: Title::try_from("check the porch light".to_string())
				.unwrap(),
			brief: Brief::try_from("every minute".to_string()).unwrap(),
			role: RoleName::Planning,
			schedule: Schedule::parse(None, Some("* * * * *"), clock.now())
				.unwrap(),
			priority: TaskPriority::default(),
			created_by: Creator::Cli,
		})
		.unwrap();

	let tool = sandman::tools::await_result::AwaitResult;
	let ctx = harness.ctx(SessionId(0));
	let args = serde_json::json!({"task_id": cron.to_string()});
	let answered = tokio::time::timeout(
		std::time::Duration::from_secs(1),
		tool.call(&ctx, args),
	)
	.await;

	assert!(answered.is_ok(), "awaiting a cron Task must not hang");
	assert!(answered.unwrap().contains("cron schedule"));
}
