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

use crate::domain::{
	CallRequest, LessonSubject, Message, NewLesson, Nudge, Outcome, Reflection,
	ReflectionKind, ReflectionResult, Reply, SessionId, SessionKind,
};
use crate::scheduler::{SchedulerError, Tier};
use crate::session::SessionCtx;

/// Every section metacognition may write. A section ends where the next begins.
pub const SECTIONS: [&str; 3] = ["summary", "feedback", "lessons"];

/// Review one Worker Session's conversation.
///
/// Feedback is read first: it is the review saying the run is not over, whatever
/// else it wrote beside it. A summary written alongside would be an answer to a
/// Task the review has just said is unfinished.
///
/// Neither a summary nor feedback means the review had nothing to say, and the
/// caller falls back to what the Worker itself wrote last.
pub async fn reflect(ctx: &SessionCtx) -> Outcome {
	metacognise(
		ctx,
		ReflectionKind::Review,
		crate::prompts::META_SYSTEM,
		crate::prompts::REVIEW,
	)
	.await
}

/// Interrupt a Session mid-turn and ask whether the run is still going somewhere:
/// is it looping, already finished, chasing something that cannot be reached, or
/// no longer on its goal?
///
/// A summary is dropped rather than obeyed. The interrupt was told not to write
/// one, and a model that writes it anyway is answering a Task on behalf of a
/// Session that never offered an answer. An empty answer is the expected one.
pub async fn interrupt(ctx: &SessionCtx) -> Nudge {
	match metacognise(
		ctx,
		ReflectionKind::Interrupt,
		crate::prompts::META_SYSTEM,
		crate::prompts::INTERRUPT,
	)
	.await
	{
		Outcome::Feedback(text) => Nudge::Feedback(text),
		Outcome::Complete(_) | Outcome::Nothing => Nudge::Nothing,
	}
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
///
/// Three ways it ends, and each writes a different record:
///
/// - the call answered — a [`crate::domain::ReflectionResult::Ran`] on that call;
/// - the call failed — a [`crate::domain::ReflectionResult::FailedOpen`] on the
///   same call, which exists because the scheduler recorded it before sending;
/// - the Store refused — nothing was queued, so there is no call to anchor a
///   Reflection on and none is written. Failing open still holds: the Session
///   carries on either way.
async fn metacognise(
	ctx: &SessionCtx,
	kind: ReflectionKind,
	system: &str,
	question: &str,
) -> Outcome {
	let Ok(after_message) = ctx.store.message_count(ctx.id) else {
		return Outcome::Nothing;
	};
	let Ok(transcript) = ctx.store.messages(ctx.id) else {
		return Outcome::Nothing;
	};

	let mut messages = Vec::with_capacity(transcript.len() + 2);
	messages.push(Message::System { content: system.to_string() });
	for message in transcript {
		messages.push(match message {
			Message::System { content } => Message::User { content },
			other => other,
		});
	}
	messages.push(Message::User { content: question.to_string() });

	let request = CallRequest { messages, tools: Vec::new() };
	let now = ctx.clock.now();

	match ctx
		.scheduler
		.request(ctx.id, request, Tier::Metacognition, now)
		.await
	{
		Ok((call, completion)) => {
			let content = match completion.reply {
				Reply::Text(text) => text,
				Reply::Calls { preamble, .. } => preamble.unwrap_or_default(),
			};

			let feedback = section(&content, "feedback")
				.map(|s| s.trim().to_string())
				.filter(|s| !s.is_empty());
			let summary = if kind == ReflectionKind::Review {
				section(&content, "summary")
					.map(|s| s.trim().to_string())
					.filter(|s| !s.is_empty())
			} else {
				None
			};

			let outcome = match feedback {
				Some(text) => Outcome::Feedback(text),
				None => match summary {
					Some(text) => Outcome::Complete(text),
					None => Outcome::Nothing,
				},
			};

			let _ = ctx.store.record_reflection(
				ctx.id,
				Reflection {
					kind,
					call,
					after_message,
					at: now,
					result: ReflectionResult::Ran {
						reasoning: completion.reasoning,
						content: content.clone(),
						outcome: outcome.clone(),
					},
				},
			);

			keep_lessons(ctx, ctx.id, &content).await;

			outcome
		},
		Err(SchedulerError::Call { call, source }) => {
			let _ = ctx.store.record_reflection(
				ctx.id,
				Reflection {
					kind,
					call,
					after_message,
					at: now,
					result: ReflectionResult::FailedOpen {
						error: source.to_string(),
					},
				},
			);
			Outcome::Nothing
		},
		Err(SchedulerError::Store(_)) => Outcome::Nothing,
	}
}

/// Keep what a metacognition thought was worth keeping.
///
/// An empty `<lessons>` section is normal, and for an interrupt it is the
/// expected answer; either way it writes nothing. A lesson never re-enters the
/// conversation it came from — the Session still cannot see what was written
/// about it.
async fn keep_lessons(ctx: &SessionCtx, session: SessionId, content: &str) {
	let Some(lessons) = section(content, "lessons") else {
		return;
	};
	let text = lessons.trim();
	if text.is_empty() {
		return;
	}
	let Ok(Some(loaded)) = ctx.store.session(session) else {
		return;
	};
	let about = match loaded.kind {
		SessionKind::Worker { task, role } => {
			let Ok(Some(t)) = ctx.store.task(task) else {
				return;
			};
			LessonSubject::Task { task, role, title: t.title }
		},
		SessionKind::Comms { channel, .. } => {
			LessonSubject::Conversation { channel }
		},
	};
	let _ = ctx.store.keep_lesson(
		NewLesson { text: text.to_string(), session, about },
		ctx.clock.now(),
	);
}

/// Read one `<name>…</name>` section out of what the metacognition wrote.
///
/// Small models drop the closing tag. Everything up to the next section is still
/// meant as this one's text, so a section stops there rather than at the end of
/// the reply — otherwise an unclosed `<lessons>` swallows the summary written
/// after it.
pub fn section(content: &str, name: &str) -> Option<String> {
	let tag_start = format!("<{name}");
	let idx = content.find(&tag_start)?;
	let rest = content[idx + tag_start.len()..].trim_start();

	if let Some(after_self) = rest.strip_prefix("/>") {
		let _ = after_self;
		return Some(String::new());
	}

	let after_open = rest.strip_prefix('>')?;

	let close_tag = format!("</{name}>");
	let mut end = after_open.find(&close_tag).unwrap_or(after_open.len());

	for other in SECTIONS.iter().filter(|s| **s != name) {
		let other_open = format!("<{other}");
		if let Some(pos) = after_open.find(&other_open) {
			end = end.min(pos);
		}
	}

	Some(after_open[..end].trim().to_string())
}
