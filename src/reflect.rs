//! Metacognition. Two of them, sharing everything but the question they ask.
//!
//! A **review** runs when a Worker Session ends its turn without calling a tool
//! — prose or silence. It reads the whole conversation and decides what that turn
//! meant for the Task. Its `<summary>` is the Task's answer.
//!
//! An **interrupt** runs mid-turn, on a message count, and decides nothing about
//! the Task. It exists for the failure a review structurally cannot see: a Worker
//! that never stops calling tools never produces a plain-text turn, so it is
//! never reviewed and can grind on one dead end until something else stops it. It
//! reaches Comms Sessions too, which are never reviewed at all — it is the only
//! metacognition they ever see.
//!
//! Neither is an agent. Both are bare model calls the Harness makes: no Role, no
//! identity, no tools of any kind. Each writes tagged sections and the Harness
//! reads them. Modelling either as a swarm member would make its outcome
//! asynchronous and hold a Task's answer hostage on a pending review.
//!
//! The sections either may write, and nothing else:
//!
//! - `<summary>` — the Task's answer. A review only.
//! - `<feedback>` — correction, injected into the Session's context as a message
//!   of its own; it takes another turn on it.
//! - `<lessons>` — what is worth keeping for whoever does this kind of work next.
//!   It outlives the Session it judged, and nothing reads it automatically.
//!
//! **Both fail open, always.** A call that cannot be made is recorded as having
//! found nothing and the Session carries on. This matters most for the interrupt,
//! which runs mid-turn on a Session that is otherwise fine: broken metacognition
//! must never be what wedges a run.
//!
//! That an interrupt cannot complete a Task is a fact about its signature, not a
//! check: [`interrupt`] returns [`Nudge`], which has no completing variant.
//!
//! Defines: [`reflect`], [`interrupt`], [`due_for_interrupt`], [`INTERRUPT_EVERY`],
//! [`section`].

use crate::domain::{Nudge, Outcome, SessionId};
use crate::session::SessionCtx;

/// How many messages may pass without any metacognition before an interrupt
/// fires.
///
/// Counted from the last one of either kind. A review has just read the whole
/// conversation and had its own chance at feedback, so an interrupt one message
/// later would only spend money to repeat it. A Worker taking short turns is
/// reviewed and never interrupted; a Comms Session — never reviewed — is
/// interrupted on a plain message count.
pub const INTERRUPT_EVERY: usize = 15;

/// Every section metacognition may write. A section ends where the next begins.
pub const SECTIONS: [&str; 3] = ["summary", "feedback", "lessons"];

/// Is this Session due for an interrupt on its next model call?
pub async fn due_for_interrupt(_ctx: &SessionCtx) -> bool {
	unimplemented!()
}

/// Review one Worker Session's conversation.
///
/// Feedback is read first: it is the review saying the run is not over, whatever
/// else it wrote beside it. A summary written alongside would be an answer to a
/// Task the review has just said is unfinished.
///
/// Neither a summary nor feedback means the review had nothing to say, and the
/// caller falls back to what the Worker itself wrote last.
pub async fn reflect(_ctx: &SessionCtx) -> Outcome {
	unimplemented!()
}

/// Interrupt a Session mid-turn and ask whether the run is still going somewhere:
/// is it looping, already finished, chasing something that cannot be reached, or
/// no longer on its goal?
///
/// A summary is dropped rather than obeyed. The interrupt was told not to write
/// one, and a model that writes it anyway is answering a Task on behalf of a
/// Session that never offered an answer. An empty answer is the expected one.
pub async fn interrupt(_ctx: &SessionCtx) -> Nudge {
	unimplemented!()
}

/// One metacognitive call, whichever it is.
///
/// The sandwich: the metacognition system prompt on top, the Session's whole
/// conversation as the subject with system messages recast as user so the
/// reviewer never adopts them, and the framing of the question last.
///
/// The call goes through the scheduler at [`crate::scheduler::Tier::Metacognition`]
/// and is recorded against the Session it judges, so its cost lands where the
/// work is.
async fn metacognise(
	_ctx: &SessionCtx,
	_kind: crate::domain::ReflectionKind,
	_system: &str,
	_question: &str,
) -> Outcome {
	unimplemented!()
}

/// Keep what a metacognition thought was worth keeping.
///
/// An empty `<lessons>` section is normal, and for an interrupt it is the
/// expected answer; either way it writes nothing. A lesson never re-enters the
/// conversation it came from — the Session still cannot see what was written
/// about it.
async fn keep_lessons(_ctx: &SessionCtx, _session: SessionId, _content: &str) {
	unimplemented!()
}

/// Read one `<name>…</name>` section out of what the metacognition wrote.
///
/// Small models drop the closing tag. Everything up to the next section is still
/// meant as this one's text, so a section stops there rather than at the end of
/// the reply — otherwise an unclosed `<lessons>` swallows the summary written
/// after it.
pub fn section(_content: &str, _name: &str) -> Option<String> {
	unimplemented!()
}
