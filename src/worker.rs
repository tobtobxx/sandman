//! The Worker Session: created from a Task, ends when that Task completes.
//!
//! A Worker is a Session plus one policy — what the text a turn produces means.
//! Here it means: stop and be reviewed. **A Worker has no tool to submit
//! anything.** After every plain-text turn the metacognitive review reads the
//! whole conversation and either writes the Task's answer as its summary,
//! corrects the Worker with feedback it takes another turn on, or has nothing to
//! say. So what reaches a subscriber is chosen from the whole conversation, not
//! taken from whatever the Worker happened to say last.
//!
//! A Worker is uniform. Every one runs the same way; only the Role of its Task
//! differs. It sees the Brief and nothing of the work that led to it.
//!
//! Neither loop here has a mechanical bound. Silence is not an ending but
//! something the review corrects, and nothing caps how many turns a Task takes.
//! The human watching is the guard rail.
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

/// A fresh Worker Session for a Task: the Role's system prompt, the Brief, and
/// nothing else.
///
/// The Brief arrives stamped with the time it reached its Worker, so "just now"
/// and "earlier" mean something to a Session that runs for a while. Separate
/// from running it, so a caller can put something in the context first.
pub async fn new_worker_session(
	ctx: &SessionCtx,
	task: &Task,
) -> Result<crate::domain::SessionId, crate::store::StoreError> {
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
	ctx.store.start_session(new, now)
}

/// One turn for a Worker Session, and what follows it.
///
/// The whole policy of a Worker lives here; the Harness only decides when to
/// call this again.
///
/// An unreachable model is the one failure written without a review — nothing
/// can review or correct a Worker whose model is gone. A cancellation ends the
/// Session with no Result, because the Task was already marked cancelled and
/// must not complete.
///
/// A review that wrote neither a summary nor feedback falls back to what the
/// Worker itself wrote last; a review of silence sends the Worker back to work.
pub async fn work_turn(ctx: &SessionCtx) -> Worked {
	let Ok(Some(session)) = ctx.store.session(ctx.id) else {
		return Worked::Aborted;
	};
	let Some(task_id) = session.kind.task() else {
		return Worked::Aborted;
	};
	let tier = match ctx.store.task(task_id) {
		Ok(Some(task)) => Tier::from(task.priority),
		_ => Tier::TaskNormal,
	};

	match crate::session::turn(ctx, tier).await {
		crate::session::Turn::Cancelled => {
			let now = ctx.clock.now();
			let _ = ctx.store.end_session(ctx.id, SessionStatus::Finished, now);
			Worked::Aborted
		},

		crate::session::Turn::Unreachable(error) => {
			let now = ctx.clock.now();
			let _ = ctx.store.end_session(
				ctx.id,
				SessionStatus::Failed { reason: error.clone() },
				now,
			);
			Worked::Done(TaskResult::Failed(error))
		},

		crate::session::Turn::Text(text) => {
			// A review that has nothing to say falls back to the Worker's
			// own text as the Task's answer.
			review(ctx, Worked::Done(TaskResult::Succeeded(text))).await
		},

		crate::session::Turn::Silent => {
			// A review of silence that has nothing to say sends the Worker
			// back to work.
			review(ctx, Worked::Continue).await
		},
	}
}

/// Review the turn just taken and act on it. `on_nothing` is what a review
/// with neither a summary nor feedback falls back to.
async fn review(ctx: &SessionCtx, on_nothing: Worked) -> Worked {
	let _ = ctx.store.set_status(ctx.id, SessionStatus::Reflecting);
	match crate::reflect::reflect(ctx).await {
		Outcome::Complete(answer) => {
			let now = ctx.clock.now();
			let _ = ctx.store.end_session(ctx.id, SessionStatus::Finished, now);
			Worked::Done(TaskResult::Succeeded(answer))
		},
		Outcome::Feedback(feedback) => {
			crate::session::tell(ctx, &feedback).await;
			let _ = ctx.store.set_status(ctx.id, SessionStatus::Waiting);
			Worked::Continue
		},
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
