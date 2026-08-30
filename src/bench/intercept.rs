//! Watching, and answering, every tool call.
//!
//! This is the seam the bench is built on. A case takes one real Task, the real
//! system prompt for its Role, and the real model — and puts an [`Interceptor`]
//! where the tool registry would be. Every call the model makes is recorded, and
//! the case decides which of them actually happen.
//!
//! That is the only question a case asks: given this Brief, what does the model
//! reach for, with what arguments, and in what order? It is also what keeps a
//! case to one Session — letting the real tools run would drag a web search and
//! three more Workers into a test about one decision.
//!
//! Three modes, and a case usually mixes them: pass a tool through because its
//! effect is what is being asserted on, answer another from a closure because
//! its result is a fixture, deny a third because reaching for it at all is the
//! failure.
//!
//! Defines: [`Interceptor`], [`ToolsChoice`], [`RecordedToolCall`], [`Answer`].

use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::{SessionId, Timestamp, ToolCall, ToolSchema};
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;
use crate::tools::ToolRunner;

/// One tool call a Session made, and what it got back.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedToolCall {
	pub session: SessionId,
	pub name: ToolName,
	/// What the model sent, parsed. Unparseable arguments are recorded as they
	/// arrived — a model that sends broken JSON is a finding, not a hole in the
	/// record.
	pub args: serde_json::Value,
	pub output: String,
	pub at: Timestamp,
	/// Whether the real tool ran, or the case answered for it.
	pub real: bool,
}

/// What the case does about one call.
pub enum Answer {
	/// Let the real tool run.
	Real,
	/// Answer with this text and do nothing.
	Say(String),
	/// Refuse, in words the model can read.
	Deny(String),
}

/// How a Rig gets its tool calls answered.
///
/// There is no "run them all" variant. A case that let the whole registry run
/// would stop being one Session, and that is the one thing the bench does not
/// measure.
pub enum ToolsChoice {
	/// Every call recorded, and each one answered by the case.
	Intercept(Box<dyn Fn(&RecordedCall) -> Answer + Send + Sync>),
	/// Every call recorded and refused. The default, and what a case asserting
	/// that a Session got where it was going without any tool at all wants.
	Deny,
}

/// A call as the interceptor's closure sees it, before it has an answer.
#[derive(Debug, Clone)]
pub struct RecordedCall {
	pub session: SessionId,
	pub name: ToolName,
	pub args: serde_json::Value,
}

/// The real registry, wrapped.
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

	/// Every call so far, in order.
	pub fn calls(&self) -> Vec<RecordedToolCall> {
		self.log.lock().expect("not poisoned").clone()
	}

	/// The calls to one tool, in order. What most assertions want.
	pub fn calls_to(&self, name: ToolName) -> Vec<RecordedToolCall> {
		self.calls()
			.into_iter()
			.filter(|c| c.name == name)
			.collect()
	}

	/// The tools reached for at all, in the order they were first used.
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
	/// Unchanged: the model is offered exactly the schemas it would be offered
	/// in a real run, whatever the case intends to do about the calls. Changing
	/// them would change the thing being measured.
	fn schemas(&self, names: &[ToolName], ctx: &SchemaCtx) -> Vec<ToolSchema> {
		self.inner.schemas(names, ctx)
	}

	/// A name the model sent that matches no [`ToolName`] cannot be recorded as
	/// one — the real registry would fail it the same way, before ever emitting
	/// an Event, so this does the same and leaves no trace.
	async fn run(&self, ctx: &SessionCtx, call: &ToolCall) -> String {
		let Ok(name) = call.name.parse::<ToolName>() else {
			return crate::tools::ToolError::NoSuchTool(call.name.clone())
				.to_string();
		};

		let args = serde_json::from_str::<serde_json::Value>(&call.arguments)
			.unwrap_or_else(|_| {
				serde_json::Value::String(call.arguments.clone())
			});
		let recorded =
			RecordedCall { session: ctx.id, name, args: args.clone() };

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

		// The real registry emits ToolCalled/ToolReturned itself when it runs;
		// an intercepted call never reaches it, so nothing would otherwise say
		// what the model tried. This is the only place that gap is closed.
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
