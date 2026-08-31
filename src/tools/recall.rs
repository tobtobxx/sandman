//! Finding out what this swarm already did. The `memory` Role's tools.
//!
//! The `memory` Role does no new work. It searches what the swarm has already
//! done: the Lessons metacognition kept, the Tasks that were asked and answered,
//! and the conversations behind them. Now that the state persists, those reach
//! back across every Run rather than only the one in progress.
//!
//! Searching is by meaning, not by keyword — see `memory.rs` for the ranking.
//! `search_tasks` deliberately ranks a Task by what it *asked for*, its Title
//! and Brief, never by its Result: a hit is found by the question, and the
//! answer is shown once it is a hit.
//!
//! Defines: [`SearchLessons`], [`SearchTasks`], [`ViewSession`].

use std::str::FromStr;

use async_trait::async_trait;

use crate::domain::{
	AssistantBody, Hit, Lesson, Message, ReflectionResult, Session, SessionId,
	Task, TaskId, TaskState, ToolSchema,
};
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;

use super::{Tool, ToolError};

/// How many hits a search returns unless the model asks for more.
const DEFAULT_HITS: usize = 5;

/// How much of one conversation `view_session` will show. A whole Session can be
/// longer than the Session reading it can hold.
pub const VIEW_SESSION_CAP: usize = 40_000;

/// Rank the Lessons against a query by meaning.
pub struct SearchLessons;

/// Rank Tasks by what they asked for. Shows the Result on a hit.
pub struct SearchTasks;

/// One Session's whole conversation as text, metacognition included, capped.
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

	/// Each hit names its day, what it was about, and the Session to open to
	/// read the whole conversation behind it.
	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		let (query, count) = match parse_search_args(&args) {
			Ok(v) => v,
			Err(e) => return e.to_string(),
		};

		let lessons = match ctx.harness.store.all_lessons() {
			Ok(l) => l,
			Err(e) => return format!("Error: {e}"),
		};
		let corpus = crate::memory::lesson_corpus(lessons);

		let embedder = crate::memory::OpenRouterEmbedder::from_env();
		match crate::memory::rank(
			&ctx.harness.store,
			&embedder,
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

	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		let (query, count) = match parse_search_args(&args) {
			Ok(v) => v,
			Err(e) => return e.to_string(),
		};

		let tasks = match ctx.harness.store.all_tasks() {
			Ok(t) => t,
			Err(e) => return format!("Error: {e}"),
		};
		let corpus: Vec<(String, String, Task)> = tasks
			.into_iter()
			.map(|t| {
				let text = format!("{}\n{}", t.title, t.brief.as_str());
				(format!("task/{}", t.id), text, t)
			})
			.collect();

		let embedder = crate::memory::OpenRouterEmbedder::from_env();
		match crate::memory::rank(
			&ctx.harness.store,
			&embedder,
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

	/// Takes a Task id too, and resolves it to the Session that did it — a hit
	/// from `search_tasks` names a Task, and asking the reader to translate that
	/// is asking it to guess.
	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		let given_session = args.get("session_id").and_then(|v| v.as_str());
		let given_task = args.get("task_id").and_then(|v| v.as_str());

		let session_id = if let Some(s) = given_session {
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
			return ToolError::Rejected(
				"Give either session_id or task_id.".to_string(),
			)
			.to_string();
		};

		match ctx.harness.store.session(session_id) {
			Ok(Some(session)) => render_session(&session),
			Ok(None) => ToolError::Rejected(format!(
				"There is no Session {session_id}."
			))
			.to_string(),
			Err(e) => format!("Error: {e}"),
		}
	}
}

/// The schema shared by both searches: a query, and how many hits to bring
/// back.
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

/// Read `query` and `count` off a search tool's arguments.
fn parse_search_args(
	args: &serde_json::Value,
) -> Result<(String, usize), ToolError> {
	let query = args
		.get("query")
		.and_then(|v| v.as_str())
		.ok_or(ToolError::Missing { field: "query" })?;
	let count = args
		.get("count")
		.and_then(|v| v.as_u64())
		.map(|n| n as usize)
		.unwrap_or(DEFAULT_HITS);
	Ok((query.to_string(), count))
}

/// Each hit, with its day, what it was about, and where to read more.
fn render_lesson_hits(hits: &[Hit<Lesson>]) -> String {
	if hits.is_empty() {
		return "No lessons match.".to_string();
	}
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

/// Each hit, with its Title and, once it is a hit, its Result.
fn render_task_hits(hits: &[Hit<Task>]) -> String {
	if hits.is_empty() {
		return "No Tasks match.".to_string();
	}
	hits.iter()
		.map(|h| {
			let task = &h.item;
			let outcome = match &task.state {
				TaskState::Completed { result, .. } => {
					result.content().to_string()
				},
				TaskState::Cancelled { .. } => {
					"Cancelled; no answer.".to_string()
				},
				TaskState::Pending => "Still pending.".to_string(),
				TaskState::Running { .. } => "Still running.".to_string(),
			};
			format!("{} — {}\n{}", task.id, task.title, outcome)
		})
		.collect::<Vec<_>>()
		.join("\n\n")
}

/// One Session's whole conversation, metacognition included, capped at
/// [`VIEW_SESSION_CAP`] characters.
fn render_session(session: &Session) -> String {
	let mut out = String::new();
	for message in &session.messages {
		match message {
			Message::System { content } => {
				out.push_str(&format!("[system] {content}\n\n"));
			},
			Message::User { content } => {
				out.push_str(&format!("[user] {content}\n\n"));
			},
			Message::Assistant { body, .. } => match body {
				AssistantBody::Text(text) => {
					out.push_str(&format!("[assistant] {text}\n\n"));
				},
				AssistantBody::Calls { preamble, calls } => {
					if let Some(preamble) = preamble {
						out.push_str(&format!("[assistant] {preamble}\n"));
					}
					for call in calls.iter() {
						out.push_str(&format!(
							"  tool call: {}({})\n",
							call.name, call.arguments
						));
					}
					out.push('\n');
				},
			},
			Message::Tool { content, .. } => {
				out.push_str(&format!("[tool result] {content}\n\n"));
			},
		}
	}

	if !session.reflections.is_empty() {
		out.push_str("--- metacognition ---\n");
		for reflection in &session.reflections {
			match &reflection.result {
				ReflectionResult::Ran { content, .. } => {
					out.push_str(&format!(
						"[{:?}] {content}\n\n",
						reflection.kind
					));
				},
				ReflectionResult::FailedOpen { error } => {
					out.push_str(&format!(
						"[{:?}] failed open: {error}\n\n",
						reflection.kind
					));
				},
			}
		}
	}

	if out.chars().count() > VIEW_SESSION_CAP {
		out = out.chars().take(VIEW_SESSION_CAP).collect();
		out.push_str("\n... (truncated)");
	}
	out
}
