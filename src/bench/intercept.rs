//! The `ToolRunner` bench seam — record and answer every tool call without changing what the model sees.
//!
//! A case measures one real Session (real Brief, real Role prompt, real scheduler) but must stay
//! one Session: letting tools run would pull web searches and child Workers into a decision test.
//! `Interceptor` wraps the real [`Registry`] so schemas pass through unchanged and `run` records
//! and answers per [`ToolsChoice`].
//!
//! Construct: `Interceptor::new(registry, choice)` — installed by `RigBuilder::tools`; `Rig` owns the `Arc`.
//! Use: `schemas(names, ctx) -> Vec<ToolSchema>` unchanged; `run(ctx, call) -> String` records a
//! [`RecordedToolCall`]; read via `calls()` / `calls_to()` / `tools_used()` or `Rig::tool_calls` / `Watch::calls`.
//! Consumers: `Rig` (driver) and every bench case/tripwire (assertions on order, args, and `real`).
//!
//! | `ToolsChoice` | `Answer` | `run` does | `real` | Events |
//! | --- | --- | --- | --- | --- |
//! | `Deny` | — | refuses with text | `false` | synthetic `ToolCalled`/`ToolReturned` |
//! | `Intercept(f)` | `Real` | delegates to inner | `true` | inner emits |
//! | `Intercept(f)` | `Say(text)` | returns fixture | `false` | synthetic |
//! | `Intercept(f)` | `Deny(reason)` | returns reason | `false` | synthetic |
//!
//! Rules:
//! - **`schemas` never changes** — filtering would measure a different model.
//! - **No "run all" variant** — the bench is one Session by construction.
//! - **Unknown `ToolName` leaves no trace** — same as `Registry`, returns `NoSuchTool` before any Event or log.
//! - **Unparseable arguments kept as `String`** — broken JSON is a finding, not a lost call.
//! - **Only non-`real` calls emit here** — `Registry` emits its own pair when it runs.
//!
//! Defines: [`Interceptor`], [`ToolsChoice`], [`Answer`], [`RecordedToolCall`], [`RecordedCall`].

use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::{SessionId, Timestamp, ToolCall, ToolSchema};
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;
use crate::tools::ToolRunner;

/// One tool call and what it returned, in order made.
///
/// Records parsed `args`, output text, timestamp and whether `real` ran.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedToolCall {
	pub session: SessionId,
	pub name: ToolName,
	/// Parsed arguments; unparseable JSON kept as `String`.
	pub args: serde_json::Value,
	pub output: String,
	pub at: Timestamp,
	/// Whether the real tool ran.
	pub real: bool,
}

/// Answer for one intercepted call.
///
/// `Real` delegates to inner, `Say` returns fixture, `Deny` refuses in model-readable words.
pub enum Answer {
	Real,
	Say(String),
	Deny(String),
}

/// How the `Rig` answers tool calls. No "run all" variant.
///
/// `Intercept` records and answers per call; `Deny` records and refuses all.
pub enum ToolsChoice {
	Intercept(Box<dyn Fn(&RecordedCall) -> Answer + Send + Sync>),
	Deny,
}

/// A call as the `Intercept` closure sees it, before answering.
///
/// Session, name and parsed args only; output not yet chosen.
#[derive(Debug, Clone)]
pub struct RecordedCall {
	pub session: SessionId,
	pub name: ToolName,
	pub args: serde_json::Value,
}

/// Wraps the real registry to record and answer calls.
pub struct Interceptor {
	inner: Arc<dyn ToolRunner>,
	choice: ToolsChoice,
	log: std::sync::Mutex<Vec<RecordedToolCall>>,
}

impl Interceptor {
	pub fn new(inner: Arc<dyn ToolRunner>, choice: ToolsChoice) -> Self {
		Interceptor {
			inner,
			choice,
			log: std::sync::Mutex::new(Vec::new()),
		}
	}

	/// Returns all recorded calls, in order made.
	pub fn calls(&self) -> Vec<RecordedToolCall> {
		self.log.lock().expect("not poisoned").clone()
	}

	/// Returns calls to `name`, in order made.
	pub fn calls_to(&self, name: ToolName) -> Vec<RecordedToolCall> {
		self.calls()
			.into_iter()
			.filter(|c| c.name == name)
			.collect()
	}

	/// Returns distinct tools used, in first-seen order.
	pub fn tools_used(&self) -> Vec<ToolName> {
		let mut used = Vec::new();
		for call in self.log.lock().expect("not poisoned").iter() {
			if !used.contains(&call.name) {
				used.push(call.name);
			}
		}
		used
	}
}

#[async_trait]
impl ToolRunner for Interceptor {
	/// Returns schemas for `names` unchanged.
	///
	/// Delegates to inner; the model sees the real tool set.
	fn schemas(&self, names: &[ToolName], ctx: &SchemaCtx) -> Vec<ToolSchema> {
		self.inner.schemas(names, ctx)
	}

	/// Answers one tool call and records it.
	///
	/// Returns `NoSuchTool` text for unknown names without recording or emitting.
	/// Otherwise records parsed args, answers per `ToolsChoice`, emits synthetic
	/// events when not `real`, and appends to log.
	async fn run(&self, ctx: &SessionCtx, call: &ToolCall) -> String {
		// Parse tool name
		let Ok(name) = call.name.parse::<ToolName>() else {
			return crate::tools::ToolError::NoSuchTool(call.name.clone())
				.to_string();
		};

		// Parse arguments
		let args = serde_json::from_str::<serde_json::Value>(&call.arguments)
			.unwrap_or_else(|_| {
				serde_json::Value::String(call.arguments.clone())
			});
		let recorded =
			RecordedCall { session: ctx.id, name, args: args.clone() };

		// Decide answer
		let (output, real) = match &self.choice {
			ToolsChoice::Deny => (
				format!("Error: {name} is not available in this case."),
				false,
			),
			ToolsChoice::Intercept(answer) => match answer(&recorded) {
				Answer::Real => (self.inner.run(ctx, call).await, true),
				Answer::Say(text) => (text, false),
				Answer::Deny(reason) => (reason, false),
			},
		};

		// Emit synthetic events
		if !real {
			ctx.events.emit(crate::event::Event::ToolCalled {
				session: ctx.id,
				name,
				args: args.clone(),
			});
			ctx.events.emit(crate::event::Event::ToolReturned {
				session: ctx.id,
				name,
				output: output.clone(),
			});
		}

		// Record call
		self.log
			.lock()
			.expect("not poisoned")
			.push(RecordedToolCall {
				session: ctx.id,
				name,
				args,
				output: output.clone(),
				at: ctx.clock.now(),
				real,
			});

		output
	}
}
