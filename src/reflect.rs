//! Metacognition — two bare model calls that judge a Session.
//!
//! Review judges a finished turn; interrupt judges a running one. Neither is an
//! agent, has a Role, or holds tools — both share `metacognise`, which builds a
//! sandwiched `CallRequest` (`META_SYSTEM` + transcript with `System→User` +
//! question) and sends it via `Scheduler::request` at `Tier::Metacognition` /
//! `Purpose::Metacognition`.
//!
//! Construct: nothing to build — `SessionCtx` in (`Harness::ctx(id)` builds it).
//! Use: `reflect(ctx) → Outcome` after a Worker `Text`/`Silent`; `interrupt(ctx) → Nudge`
//! mid-turn from `session::turn` when `msgs - last_reflection >= interrupt_interval`;
//! `section(content, name)` extracts `<name>…</name>` with next-section truncation.
//! Consumers and how they handle the same output differently:
//!
//! | Kind | When | Caller | Returns | May complete `Task`? |
//! | --- | --- | --- | --- | --- |
//! | `Review` | after Worker `Text`/`Silent` | `worker::review` | `Outcome::{Complete,Feedback,Nothing}` | yes — `Complete` writes `TaskResult` |
//! | `Interrupt` | mid-turn, counted from last metacognition | `session::check_in` inside `turn` | `Nudge::{Feedback,Nothing}` | never — `Nudge` has no `Complete` |
//!
//! Rules: **both are bare calls, not agents — no Role, no tools, synchronous.**
//! **both fail open — `FailedOpen` or `Store` error is `Nothing` and never wedges a run.**
//! **interrupt cannot complete a Task — enforced by `Nudge` having no `Complete`.**
//! **only `Feedback` re-enters context via `tell`; `Reflection` and `Lesson` never do.**
//! **lessons outlive the Session, anchored by `LessonSubject`, found later by meaning.**
//! **review is Worker-only; interrupt is the only metacognition Comms ever sees and the guard for a Worker that never stops calling tools.**
//! **sections are `<summary>`, `<feedback>`, `<lessons>`; next-section truncation handles missing `</>`.**
//!
//! Defines: [`reflect`], [`interrupt`], [`section`], [`SECTIONS`].

use crate::domain::{
	CallRequest, LessonSubject, Message, NewLesson, Nudge, Outcome, Reflection,
	ReflectionKind, ReflectionResult, Reply, SessionId, SessionKind,
};
use crate::scheduler::{SchedulerError, Tier};
use crate::session::SessionCtx;

/// Sections metacognition may write. Next section truncates an unclosed one.
pub const SECTIONS: [&str; 3] = ["summary", "feedback", "lessons"];

/// Review a Worker's finished turn.
///
/// Reads the full transcript and parses tagged sections. Returns `Complete`,
/// `Feedback`, or `Nothing`; `Feedback` takes precedence over `summary`.
pub async fn reflect(ctx: &SessionCtx) -> Outcome {
	metacognise(
		ctx,
		ReflectionKind::Review,
		crate::prompts::META_SYSTEM,
		crate::prompts::REVIEW,
	)
	.await
}

/// Check a running Session mid-turn.
///
/// Reads the full transcript and parses tagged sections. Returns `Feedback` or
/// `Nothing`; any `<summary>` is discarded and `Nothing` is the expected outcome.
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

/// Run one metacognitive call.
///
/// Builds a sandwiched request, sends at `Metacognition` tier, and records the
/// `Reflection`. Fails open on model or store error.
async fn metacognise(
	ctx: &SessionCtx,
	kind: ReflectionKind,
	system: &str,
	question: &str,
) -> Outcome {
	// Get conversation context
	let Ok(after_message) = ctx.store.message_count(ctx.id) else {
		return Outcome::Nothing;
	};
	let Ok(transcript) = ctx.store.messages(ctx.id) else {
		return Outcome::Nothing;
	};

	// Build sandwiched request
	let mut messages = Vec::with_capacity(transcript.len() + 2);
	messages.push(Message::System { content: system.to_string() });
	for message in transcript {
		messages.push(match message {
			Message::System { content } => Message::User { content },
			other => other,
		});
	}
	messages.push(Message::System { content: question.to_string() });

	let request = CallRequest { messages, tools: Vec::new() };
	let now = ctx.clock.now();

	// Send request
	match ctx
		.scheduler
		.request(
			ctx.id,
			request,
			Tier::Metacognition,
			crate::model::Purpose::Metacognition,
		)
		.await
	{
		// Call succeeded - parse and record
		Ok((call, completion)) => {
			let content = match completion.reply {
				Reply::Text(text) => text,
				Reply::Calls { preamble, .. } => preamble.unwrap_or_default(),
			};

			// Parse sections - feedback takes precedence
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

			// Record reflection
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

			// Keep lessons
			keep_lessons(ctx, ctx.id, &content).await;

			outcome
		},
		// Model failed - record FailedOpen and fail open
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
		// Store failed - fail open
		Err(SchedulerError::Store(_)) => Outcome::Nothing,
	}
}

/// Persist `<lessons>` if present and non-empty.
///
/// Anchors the lesson by `SessionKind` for later meaning search. Never
/// re-enters the judged Session's context.
async fn keep_lessons(ctx: &SessionCtx, session: SessionId, content: &str) {
	// Extract lessons
	let Some(lessons) = section(content, "lessons") else {
		return;
	};
	let text = lessons.trim();
	if text.is_empty() {
		return;
	}

	// Load session
	let Ok(Some(loaded)) = ctx.store.session(session) else {
		return;
	};

	// Resolve subject
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

	// Persist lesson
	let _ = ctx.store.keep_lesson(
		NewLesson { text: text.to_string(), session, about },
		ctx.clock.now(),
	);
}

/// Extract one tagged section from model output.
///
/// Handles self-closing `<name/>` and missing `</name>` via next-section
/// truncation. Returns trimmed text or `None` if absent.
pub fn section(content: &str, name: &str) -> Option<String> {
	// Find opening tag
	let tag_start = format!("<{name}");
	let idx = content.find(&tag_start)?;
	let rest = content[idx + tag_start.len()..].trim_start();

	// Handle self-closing
	if let Some(after_self) = rest.strip_prefix("/>") {
		let _ = after_self;
		return Some(String::new());
	}

	// Require open tag close
	let after_open = rest.strip_prefix('>')?;

	// Find closing bound
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
