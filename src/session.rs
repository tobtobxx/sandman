//! The Turn loop — both Session shapes run it; policy lives elsewhere.
//!
//! Construct: `Harness::ctx(id)` builds `SessionCtx` (Store, Events, Scheduler, ToolRunner, Clock, Harness as `Arc`s; Session owns nothing, state in Store so loop stays watchable while awaiting).
//! Use: `turn(ctx, tier) → Turn` loops until plain text; `tell(ctx, text)` enqueues for next iteration (mail, child answers, interrupt feedback all arrive as `tell`).
//! Consumers and seam — `Turn` reported here, decided in `worker.rs` vs `comms.rs` (neither references the other):
//!
//! | `Turn` | `worker.rs` | `comms.rs` |
//! | --- | --- | --- |
//! | `Text` | `reflect` → `Done` or `Continue` | `say` to human |
//! | `Silent` | `reflect`; `Nothing` → `Continue` | legitimate end → `Idle` |
//! | `Unreachable` | `Failed` without review | `Idle`, nothing said |
//! | `Cancelled` | `Aborted`, no Result | unreachable (no Task) |
//!
//! Call trace per iteration inside `turn`:
//! `cancelled? → Cancelled` · `msgs - last_reflection ≥ interrupt_interval → reflect::interrupt → tell` · `scheduler.request(Tier from caller, Purpose by SessionKind) → Reply::Calls → tools.run → loop | Reply::Text → Text/Silent` · `reflect::reflect` only after `turn` returns (Worker only).
//!
//! Rules: **a Turn decides nothing — ending policy lives in `worker.rs`/`comms.rs`, never here.** **the interrupt fires inside `turn` between model calls — a Worker grinding on tools never returns a `Turn`.** **one `scheduler.request` in flight, ordered Tier then arrival; Tier from caller (`Task` priority vs `Tier::Comms`), `Purpose` from `SessionKind`.** **Session owns nothing.** **worker and comms never reference each other.**
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
use crate::roles::{COMMS_SESSION_TOOLS, SchemaCtx};
use crate::scheduler::{Scheduler, SchedulerError, Tier};
use crate::store::Store;
use crate::tools::ToolRunner;

/// What a running Session and its tools need.
///
/// All `Arc`s; Session owns nothing, state lives in `Store`. Built by `Harness::ctx(id)`.
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
/// Reading success or failure is the caller's job.
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

/// Run one turn to completion.
///
/// Sends requests, runs tools and fires interrupt. Loops on `Calls` until `Text`, `Silent`, `Cancelled` or `Unreachable`.
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

		// Check interrupt due
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

		// Collect tool schemas
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

		// Handle LLM errors
		let completion = match outcome {
			Ok((_call, completion)) => completion,
			Err(SchedulerError::Call { source, .. }) => {
				return Turn::Unreachable(source.to_string());
			},
			Err(SchedulerError::Store(e)) => {
				return Turn::Unreachable(e.to_string());
			},
		};

		// Process replies
		match completion.reply {
			// Only text - append to history and return
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
			// Tool calls - append to history, run calls and continue loop
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

/// Enqueue text for the next turn to see.
///
/// The only path into a Session — mail, child answers and interrupt feedback all arrive here.
pub async fn tell(ctx: &SessionCtx, content: &str) {
	let _ = ctx
		.store
		.append_message(ctx.id, Message::User { content: content.to_string() });
}

/// Run interrupt and enqueue feedback if any.
///
/// Records outcome on the judged Session; `Nothing` is the expected case.
async fn check_in(ctx: &SessionCtx) {
	// Mark reflecting
	let _ = ctx.store.set_status(ctx.id, SessionStatus::Reflecting);
	// Run interrupt
	if let crate::domain::Nudge::Feedback(text) =
		crate::reflect::interrupt(ctx).await
	{
		// Enqueue feedback
		tell(ctx, &text).await;
	}
}
