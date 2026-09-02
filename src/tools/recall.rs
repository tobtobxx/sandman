//! Swarm memory — what it already did, read by meaning.
//!
//! The `memory` Role does no new work. It reads what the swarm kept: Lessons
//! metacognition wrote, Tasks asked and answered, and the Sessions behind them.
//! State persists across Runs, so searches reach back to every Lesson and Task
//! the Store holds. Ranking is by meaning, not keyword.
//!
//! Construct: `SearchLessons`, `SearchTasks`, `ViewSession` (`Tool` impls, no
//! state) in `Registry::all`; no config beyond `Store` + `Embedder` on `SessionCtx`.
//! Use: `Tool::call(ctx, args) -> String` via `Registry::run` after
//! `session::turn` builds schemas; searches parse `query`/`count`, load corpus,
//! delegate to `memory::rank`, render hits; `ViewSession` resolves an id then
//! renders the capped transcript.
//! Consumers: `RoleName::Memory` (all three) and `RoleName::TaskManager`
//! (`SearchTasks`); bench recorder wraps `ToolRunner` to answer without embedding.
//! Seam: `Tool` (one capability) vs `ToolRunner::Registry` (real dispatch).
//!
//! | Tool | corpus | ranks by | shows on hit |
//! | --- | --- | --- | --- |
//! | `SearchLessons` | `lesson_corpus` (`lesson/{id}` → `text`) | `Lesson.text` | day, about, text, Session |
//! | `SearchTasks` | `task/{id}` → `title\nbrief` | Title + Brief, never Result | id, title, Result/outcome |
//! | `ViewSession` | one `Session` | — | capped transcript + metacognition |
//!
//! Call trace: `turn → schemas_for → model → Reply::Calls → Registry::run → recall::Tool::call → Store + memory::rank → String → append Tool message → loop`.
//!
//! Rules: **search is by meaning — `memory::rank` does cosine, brute force.**
//! **SearchTasks ranks by Title+Brief, never Result — hit found by question, answer shown after.**
//! **ViewSession returns at most `VIEW_SESSION_CAP` chars — a Session can be larger than its reader.**
//! **Task id in ViewSession resolves to Session that ran it — search hit translates without guess.**
//! **tool answers in words, always — embed or Store failure is a sentence, not an `Err`.**
//! **searches reach across every Run, not just the current one.**
//!
//! Defines: [`SearchLessons`], [`SearchTasks`], [`ViewSession`], [`VIEW_SESSION_CAP`].

use std::str::FromStr;

use async_trait::async_trait;

use crate::domain::{
	Hit, Lesson, ReflectionResult, Session, SessionId, Task, TaskId, TaskState,
	ToolSchema,
};
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;

use super::{Tool, ToolError};

/// Default hit count when `count` is omitted.
const DEFAULT_HITS: usize = 5;

/// Cap on `ViewSession` output in chars.
pub const VIEW_SESSION_CAP: usize = 40_000;

/// Search Lessons by meaning; renders day, about, text and Session.
pub struct SearchLessons;

/// Search Tasks by Title and Brief by meaning; shows Result on hits.
pub struct SearchTasks;

/// Read one Session's transcript, capped at `VIEW_SESSION_CAP`.
pub struct ViewSession;

#[async_trait]
impl Tool for SearchLessons {
	fn name(&self) -> ToolName {
		ToolName::SearchLessons
	}

	fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
		search_schema(
			self.name(),
			"Search the Lessons metacognition kept — tips, tricks and gotchas \
			 from past runs — semantically.",
		)
	}

	/// Search Lessons by `query`/`count` and render hits.
	///
	/// Loads every Lesson, ranks via `memory::rank`, returns sentences on failure.
	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		// Parse arguments
		let (query, count) = match parse_search_args(&args) {
			Ok(v) => v,
			Err(e) => return e.to_string(),
		};

		// Load lessons
		let lessons = match ctx.harness.store.all_lessons() {
			Ok(l) => l,
			Err(e) => return format!("Error: {e}"),
		};

		// Build corpus
		let corpus = crate::memory::lesson_corpus(lessons);

		// Rank and render
		match crate::memory::rank(
			&ctx.harness.store,
			ctx.harness.embedder.as_ref(),
			&query,
			&corpus,
			count,
		)
		.await
		{
			Ok(hits) => render_lesson_hits(&hits),
			Err(e) => crate::memory::search_failed("the Lessons", &e),
		}
	}
}

#[async_trait]
impl Tool for SearchTasks {
	fn name(&self) -> ToolName {
		ToolName::SearchTasks
	}

	fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
		search_schema(
			self.name(),
			"Search past Tasks by what they asked for — their Title and \
			 Brief — by meaning. Shows the Result on a hit.",
		)
	}

	/// Search Tasks by `query`/`count` and render hits.
	///
	/// Loads every Task, ranks by Title+Brief via `memory::rank`, returns sentences on failure.
	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		// Parse arguments
		let (query, count) = match parse_search_args(&args) {
			Ok(v) => v,
			Err(e) => return e.to_string(),
		};

		// Load tasks
		let tasks = match ctx.harness.store.all_tasks() {
			Ok(t) => t,
			Err(e) => return format!("Error: {e}"),
		};

		// Build corpus
		let corpus: Vec<(String, String, Task)> = tasks
			.into_iter()
			.map(|t| {
				let text = format!("{}\n{}", t.title, t.brief.as_str());
				(format!("task/{}", t.id), text, t)
			})
			.collect();

		// Rank and render
		match crate::memory::rank(
			&ctx.harness.store,
			ctx.harness.embedder.as_ref(),
			&query,
			&corpus,
			count,
		)
		.await
		{
			Ok(hits) => render_task_hits(&hits),
			Err(e) => crate::memory::search_failed("Tasks", &e),
		}
	}
}

#[async_trait]
impl Tool for ViewSession {
	fn name(&self) -> ToolName {
		ToolName::ViewSession
	}

	fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
		ToolSchema {
			name: self.name().to_string(),
			description: format!(
				"Read one Session's whole conversation, capped at {} \
				 characters. Give either a Session id or a Task id — a Task \
				 id is resolved to the Session that ran it.",
				VIEW_SESSION_CAP
			),
			parameters: serde_json::json!({
				"type": "object",
				"properties": {
					"session_id": {
						"type": "string",
						"description": "A Session id, e.g. \"s-04\".",
					},
					"task_id": {
						"type": "string",
						"description": "A Task id, e.g. \"t-03\", from a \
										search_tasks hit.",
					},
				},
				"required": [],
			}),
		}
	}

	/// Resolve a Session or Task id, load the Session and render it capped.
	///
	/// Accepts `session_id` or `task_id`; Task id maps to its running Session. Returns a sentence on bad ids or Store failure.
	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		// Read raw ids
		let given_session = args.get("session_id").and_then(|v| v.as_str());
		let given_task = args.get("task_id").and_then(|v| v.as_str());

		// Resolve Session id
		let session_id = if let Some(s) = given_session {
			// Session id given - parse
			match SessionId::from_str(s) {
				Ok(id) => id,
				Err(_) => {
					return ToolError::Rejected(format!(
						"`{s}` is not a Session id."
					))
					.to_string();
				},
			}
		} else if let Some(s) = given_task {
			// Task id given - resolve to Session
			let task = match TaskId::from_str(s) {
				Ok(id) => id,
				Err(_) => {
					return ToolError::NoSuchTask(s.to_string()).to_string()
				},
			};
			match ctx.harness.store.session_for_task(task) {
				Ok(Some(id)) => id,
				Ok(None) => return format!("No Session ever ran {task}."),
				Err(e) => return format!("Error: {e}"),
			}
		} else {
			// No id given - reject
			return ToolError::Rejected(
				"Give either session_id or task_id.".to_string(),
			)
			.to_string();
		};

		// Load and render Session
		match ctx.harness.store.session(session_id) {
			// Found - render transcript
			Ok(Some(session)) => render_session(&session),
			// Not found - reject
			Ok(None) => ToolError::Rejected(format!(
				"There is no Session {session_id}."
			))
			.to_string(),
			// Store failed - report error
			Err(e) => format!("Error: {e}"),
		}
	}
}

/// Build the shared search schema (`query` + `count`).
fn search_schema(name: ToolName, description: &str) -> ToolSchema {
	ToolSchema {
		name: name.to_string(),
		description: description.to_string(),
		parameters: serde_json::json!({
			"type": "object",
			"properties": {
				"query": {
					"type": "string",
					"description": "What to search for, in plain language.",
				},
				"count": {
					"type": "integer",
					"description": format!(
						"How many hits to return. Defaults to {DEFAULT_HITS}."
					),
				},
			},
			"required": ["query"],
		}),
	}
}

/// Parse `query` and `count` from search args.
fn parse_search_args(
	args: &serde_json::Value,
) -> Result<(String, usize), ToolError> {
	// Parse query
	let query = args
		.get("query")
		.and_then(|v| v.as_str())
		.ok_or(ToolError::Missing { field: "query" })?;
	// Parse count
	let count = args
		.get("count")
		.and_then(|v| v.as_u64())
		.map(|n| n as usize)
		.unwrap_or(DEFAULT_HITS);
	Ok((query.to_string(), count))
}

/// Render Lesson hits as text.
fn render_lesson_hits(hits: &[Hit<Lesson>]) -> String {
	// Handle empty
	if hits.is_empty() {
		return "No lessons match.".to_string();
	}
	// Render hits
	hits.iter()
		.map(|h| {
			format!(
				"{} — {}\n{}\n(see {} for the full conversation)",
				h.item.day,
				h.item.about.describe(),
				h.item.text,
				h.item.session
			)
		})
		.collect::<Vec<_>>()
		.join("\n\n")
}

/// Render Task hits with outcome on each.
fn render_task_hits(hits: &[Hit<Task>]) -> String {
	// Handle empty
	if hits.is_empty() {
		return "No Tasks match.".to_string();
	}
	// Render hits
	hits.iter()
		.map(|h| {
			let task = &h.item;
			let outcome = match &task.state {
				// Completed - show Result
				TaskState::Completed { result, .. } => {
					result.content().to_string()
				},
				// Cancelled - no answer
				TaskState::Cancelled { .. } => {
					"Cancelled; no answer.".to_string()
				},
				// Pending - still waiting
				TaskState::Pending => "Still pending.".to_string(),
				// Running - still working
				TaskState::Running { .. } => "Still running.".to_string(),
			};
			format!("{} — {}\n{}", task.id, task.title, outcome)
		})
		.collect::<Vec<_>>()
		.join("\n\n")
}

/// Render one Session's transcript, capped at `VIEW_SESSION_CAP`.
fn render_session(session: &Session) -> String {
	// Collect transcript
	let mut out = String::new();
	for message in &session.messages {
		out.push_str(&message.render());
		out.push_str("\n\n");
	}

	// Append metacognition
	if !session.reflections.is_empty() {
		out.push_str("--- metacognition ---\n");
		for reflection in &session.reflections {
			match &reflection.result {
				// Ran - show content
				ReflectionResult::Ran { content, .. } => {
					out.push_str(&format!(
						"[{:?}] {content}\n\n",
						reflection.kind
					));
				},
				// Failed open - show error
				ReflectionResult::FailedOpen { error } => {
					out.push_str(&format!(
						"[{:?}] failed open: {error}\n\n",
						reflection.kind
					));
				},
			}
		}
	}

	// Cap output
	if out.chars().count() > VIEW_SESSION_CAP {
		out = out.chars().take(VIEW_SESSION_CAP).collect();
		out.push_str("\n... (truncated)");
	}
	out
}
