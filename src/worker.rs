//! The Worker Session: one Session per Task, ends with it — the only place a Task's Result is decided.
//!
//! Construct: `new_worker_session(ctx, task)` mints a `SessionId` from the Role's prompt and stamped Brief; every entry takes `SessionCtx` built by `Harness::ctx(id)`.
//! Use: `work_turn(ctx) → Worked` runs `session::turn(ctx, Tier from Task priority)` then `reflect` if `Text`/`Silent`; `Harness::drive_worker` loops until `Done` or `Aborted` then `complete_task` delivers and re-arms.
//! Consumers and seam — `Turn` reported by `session::turn`, decided here vs `comms.rs` (neither references the other):
//!
//! | `Turn` | `worker.rs` | `comms.rs` |
//! | --- | --- | --- |
//! | `Text` | `reflect` → `Done` or `Continue` | `say` to human |
//! | `Silent` | `reflect`; `Nothing` → `Continue` | legitimate end → `Idle` |
//! | `Unreachable` | `Failed` without review | `Idle`, nothing said |
//! | `Cancelled` | `Aborted`, no Result | unreachable (no Task) |
//!
//! Call trace: `Harness::drive_worker → work_turn → session::turn → reflect::reflect → tell` on `Feedback`; `Harness::complete_task → deliver → waiters::resolve`.
//! Rules: **a Turn decides nothing — ending policy lives in `worker.rs`/`comms.rs`, never `session.rs`.** **a Worker has no tool to submit — only `Review` completes a Task.** **Unreachable fails the Task without review — nothing can correct a gone model.** **`Cancelled` ends with no Result.** **`Review` fallback: `Nothing` on `Text` → Worker's text stands, on `Silent` → back to work.** **one Worker per Task, branchless dispatch (every Task becomes a Worker, never a Comms).** **uniform — Role varies, loop does not; Brief is the only parent context.** **no mechanical bound — human is the guard rail.**
//!
//! Defines: [`Worked`], [`new_worker_session`], [`work_turn`].

use crate::domain::{
	Message, NewSession, Outcome, SessionKind, SessionStatus, Task, TaskResult,
};
use crate::scheduler::Tier;
use crate::session::SessionCtx;

/// What one [`work_turn`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Worked {
	/// The Task has its Result, success or failure.
	Done(TaskResult),
	/// Feedback or a nudge went in; the Session takes another turn.
	Continue,
	/// The Task was cancelled. No Result exists and nothing was reviewed.
	Aborted,
}

/// Create a Worker Session from a Task.
///
/// Stamps the Brief with the current time and writes the Session via `Store::start_session`. Returns the new `SessionId`.
pub async fn new_worker_session(
	ctx: &SessionCtx,
	task: &Task,
) -> Result<crate::domain::SessionId, crate::store::StoreError> {
	// Build brief
	let now = ctx.clock.now();
	let brief =
		format!("{}\n\n{}", crate::domain::time::stamp(now), task.brief);
	let new = NewSession {
		kind: SessionKind::Worker { task: task.id, role: task.role },
		status: SessionStatus::Waiting,
		messages: vec![
			Message::System { content: crate::roles::system_prompt(task.role) },
			Message::User { content: brief },
		],
	};
	// Start session
	ctx.store.start_session(new, now)
}

/// Run one Worker turn and apply review policy.
///
/// Runs `session::turn` at the Task's Tier, then `reflect` on `Text`/`Silent`. Returns `Done`, `Continue`, or `Aborted`.
pub async fn work_turn(ctx: &SessionCtx) -> Worked {
	// Load session
	let Ok(Some(session)) = ctx.store.session(ctx.id) else {
		return Worked::Aborted;
	};
	let Some(task_id) = session.kind.task() else {
		return Worked::Aborted;
	};
	// Resolve tier
	let tier = match ctx.store.task(task_id) {
		Ok(Some(task)) => Tier::from(task.priority),
		_ => Tier::TaskNormal,
	};

	// Run turn
	match crate::session::turn(ctx, tier).await {
		// Cancelled - end with no Result
		crate::session::Turn::Cancelled => {
			let now = ctx.clock.now();
			let _ = ctx.store.end_session(ctx.id, SessionStatus::Finished, now);
			Worked::Aborted
		},

		// Unreachable - fail without review
		crate::session::Turn::Unreachable(error) => {
			let now = ctx.clock.now();
			let _ = ctx.store.end_session(
				ctx.id,
				SessionStatus::Failed { reason: error.clone() },
				now,
			);
			Worked::Done(TaskResult::Failed(error))
		},

		// Text - review with Worker's text as fallback
		crate::session::Turn::Text(text) => {
			review(ctx, Worked::Done(TaskResult::Succeeded(text))).await
		},

		// Silent - review with Continue as fallback
		crate::session::Turn::Silent => {
			review(ctx, Worked::Continue).await
		},
	}
}

/// Apply review after a turn and handle its outcome.
///
/// On `Complete` ends with success, on `Feedback` enqueues and continues, on `Nothing` falls back to `on_nothing`.
async fn review(ctx: &SessionCtx, on_nothing: Worked) -> Worked {
	// Mark reflecting
	let _ = ctx.store.set_status(ctx.id, SessionStatus::Reflecting);
	// Run review
	match crate::reflect::reflect(ctx).await {
		// Complete - end with success
		Outcome::Complete(answer) => {
			let now = ctx.clock.now();
			let _ = ctx.store.end_session(ctx.id, SessionStatus::Finished, now);
			Worked::Done(TaskResult::Succeeded(answer))
		},
		// Feedback - enqueue and continue
		Outcome::Feedback(feedback) => {
			crate::session::tell(ctx, &feedback).await;
			let _ = ctx.store.set_status(ctx.id, SessionStatus::Waiting);
			Worked::Continue
		},
		// Nothing - fallback to caller's intent
		Outcome::Nothing => {
			if matches!(on_nothing, Worked::Done(_)) {
				let now = ctx.clock.now();
				let _ =
					ctx.store.end_session(ctx.id, SessionStatus::Finished, now);
			} else {
				let _ = ctx.store.set_status(ctx.id, SessionStatus::Waiting);
			}
			on_nothing
		},
	}
}
