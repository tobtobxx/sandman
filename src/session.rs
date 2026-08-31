//! The Turn: model calls and tool calls, until the model replies with plain text.
//!
//! Both shapes of Session run this one loop. In the prototype a Session was one
//! class holding both its data and its loop; here the data is in the Store —
//! because its whole life has to be watchable while it happens, and a loop that
//! awaits cannot hold it — and the loop is a function over [`SessionCtx`].
//!
//! **A turn decides nothing.** It reports how it ended — text, silence, an
//! unreachable model, or a Task that was cancelled underneath it — and the caller
//! says what that means. This is the seam worth protecting: the two shapes of
//! Session differ by almost nothing else, and they once ran as two copies of one
//! loop until they quietly drifted apart. Ending policy belongs in `worker.rs`
//! or `comms.rs`, never here.
//!
//! The single exception is the metacognitive interrupt, which fires between two
//! model calls in this loop. It has to: a caller only ever sees turns that
//! finished, and a Worker grinding on tool calls never finishes one — which is
//! exactly the failure the interrupt exists to catch. The top of the loop is
//! where it goes, because there every tool call already has its result and a
//! pushed message cannot split the two.
//!
//! Defines: [`SessionCtx`], [`Turn`], [`turn`], [`tell`].

use std::sync::Arc;

use crate::domain::{
	AssistantBody, CallRequest, Clock, Message, Reply, SessionId,
	SessionStatus, TaskState,
};
use crate::event::Events;
use crate::harness::Harness;
use crate::model::Purpose;
use crate::roles::{SchemaCtx, COMMS_SESSION_TOOLS};
use crate::scheduler::{Scheduler, SchedulerError, Tier};
use crate::store::Store;
use crate::tools::ToolRunner;

/// What a running Session and its tools need to reach.
///
/// Everything here is an [`Arc`], and none of it is the Session's own state: a
/// Session owns nothing. The Harness is here because tools reach it — creating a
/// Task, waiting on one, messaging a human — and the reference is safe because
/// the Harness holds Session *ids*, never Sessions, so nothing is cyclic.
#[derive(Clone)]
pub struct SessionCtx {
	pub id: SessionId,
	pub store: Arc<Store>,
	pub events: Arc<Events>,
	pub scheduler: Arc<Scheduler>,
	pub tools: Arc<dyn ToolRunner>,
	pub clock: Arc<dyn Clock>,
	pub harness: Arc<Harness>,
}

/// How a turn ended.
///
/// None of these is a success or a failure on its own; reading that is the
/// caller's job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Turn {
	/// The model replied with plain text and called no tool.
	Text(String),
	/// The model replied with nothing at all.
	Silent,
	/// The model could not be reached.
	Unreachable(String),
	/// The Task this Session was working on was cancelled. The turn ends with no
	/// Result and nothing is reviewed.
	Cancelled,
}

// How many messages may pass without any metacognition before an interrupt
// fires is `[metacognition].interrupt_interval`.
//
// Counted from the last one of either kind. A review has just read the whole
// conversation and had its own chance at feedback, so an interrupt one message
// later would only spend money to repeat it. A Worker taking short turns is
// reviewed and never interrupted; a Comms Session — never reviewed — is
// interrupted on a plain message count.

/// One turn.
///
/// The tier is the caller's, because this one loop drives both shapes: a Worker
/// passes its Task's tier, a Comms Session passes [`Tier::Comms`]. Metacognition
/// runs its own calls through the scheduler directly and never has to ask.
pub async fn turn(ctx: &SessionCtx, tier: Tier) -> Turn {
	loop {
		// Check if cancelled
		if let Ok(Some(session)) = ctx.store.session(ctx.id) {
			if let Some(task) = session.kind.task() {
				if matches!(
					ctx.store.task_state(task),
					Ok(Some(TaskState::Cancelled { .. }))
				) {
					return Turn::Cancelled;
				}
			}
		}

		// Check if due for interrupt
		if let Ok(count) = ctx.store.message_count(ctx.id) {
			let after = ctx
				.store
				.last_reflection(ctx.id)
				.ok()
				.flatten()
				.map(|r| r.after_message)
				.unwrap_or(0);
			let every = ctx.harness.config.metacognition.interrupt_interval;
			if count.saturating_sub(after) >= every {
				check_in(ctx).await;
			}
		};

		// Get message history
		let Ok(Some(session)) = ctx.store.session(ctx.id) else {
			return Turn::Unreachable("the Session vanished".to_string());
		};
		let Ok(messages) = ctx.store.messages(ctx.id) else {
			return Turn::Unreachable(
				"could not read the Session's messages".to_string(),
			);
		};

		// Collect tool schemas, and say what this Session's calls are for.
		// One match, so a Session cannot hold one shape's tools and be
		// answered by the other shape's model.
		let (tool_names, purpose) = match &session.kind {
			crate::domain::SessionKind::Worker { role, .. } => {
				(crate::roles::tools_for(*role), Purpose::Work(*role))
			},
			crate::domain::SessionKind::Comms { .. } => {
				(&COMMS_SESSION_TOOLS[..], Purpose::Comms)
			},
		};
		let schema_ctx =
			SchemaCtx { open_channels: ctx.harness.open_channels() };
		let tools = ctx.tools.schemas(tool_names, &schema_ctx);

		// Schedule request
		let _ = ctx.store.set_status(ctx.id, SessionStatus::Thinking);
		let request = CallRequest { messages, tools };
		let outcome =
			ctx.scheduler.request(ctx.id, request, tier, purpose).await;

		// Process outcome
		let completion = match outcome {
			Ok((_call, completion)) => completion,
			Err(SchedulerError::Call { source, .. }) => {
				return Turn::Unreachable(source.to_string());
			},
			Err(SchedulerError::Store(e)) => {
				return Turn::Unreachable(e.to_string());
			},
		};

		match completion.reply {
			Reply::Text(text) => {
				let _ = ctx.store.append_message(
					ctx.id,
					Message::Assistant {
						body: AssistantBody::Text(text.clone()),
						reasoning: completion.reasoning,
					},
				);
				if text.trim().is_empty() {
					return Turn::Silent;
				}
				return Turn::Text(text);
			},
			Reply::Calls { preamble, calls } => {
				let _ = ctx.store.append_message(
					ctx.id,
					Message::Assistant {
						body: AssistantBody::Calls {
							preamble,
							calls: calls.clone(),
						},
						reasoning: completion.reasoning,
					},
				);

				let _ = ctx.store.set_status(ctx.id, SessionStatus::Tools);
				for call in calls.iter() {
					let output = ctx.tools.run(ctx, call).await;
					let _ = ctx.store.append_message(
						ctx.id,
						Message::Tool {
							tool_call_id: call.id.clone(),
							content: output,
						},
					);
				}
			},
		}
	}
}

/// Put something in the context for the next turn to see.
///
/// The only way anything reaches a Session from outside: mail, a child's answer,
/// and the feedback metacognition wrote all arrive as one of these.
pub async fn tell(ctx: &SessionCtx, content: &str) {
	let _ = ctx
		.store
		.append_message(ctx.id, Message::User { content: content.to_string() });
}

/// The interrupt, fired from the top of the loop.
///
/// Records what it found either way. An interrupt that found nothing wrong is
/// the normal outcome — and a run where none ever fired and a run where they all
/// passed would otherwise look identical from outside.
async fn check_in(ctx: &SessionCtx) {
	let _ = ctx.store.set_status(ctx.id, SessionStatus::Reflecting);
	if let crate::domain::Nudge::Feedback(text) =
		crate::reflect::interrupt(ctx).await
	{
		tell(ctx, &text).await;
	}
}
