//! The conversation: what a Session holds, and what a model call gives back.
//!
//! Two rules are carried by the types rather than by comments.
//!
//! **Text alongside tool calls is preamble, not an ending.** A model that both
//! says something and calls a tool has not finished its turn. [`AssistantBody`]
//! and [`Reply`] make that one match rather than a test on the length of a list,
//! and [`NonEmpty`] means "called tools but the list is empty" cannot be built.
//!
//! **Reasoning is inspection, never context.** Some models expose their
//! reasoning. It is recorded on the assistant message so a Watcher can read it,
//! and it must never go back to the model. The wire shape lives privately inside
//! `model.rs` and simply has no field for it, so sending it is not something to
//! remember not to do.
//!
//! Defines: [`Message`], [`AssistantBody`], [`Reply`], [`ToolCall`],
//! [`ToolSchema`], [`Completion`], [`NonEmpty`].

use super::time::Cost;

/// One message in a Session's context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
	/// The Role's system prompt. A Session has exactly one, first.
	System { content: String },
	/// Everything put into the context from outside: the Brief, mail, a Result
	/// a child produced, and the feedback metacognition wrote.
	User { content: String },
	/// What the model said.
	Assistant {
		body: AssistantBody,
		/// Recorded for inspection only. Never sent.
		reasoning: Option<String>,
	},
	/// What one tool answered.
	Tool { tool_call_id: String, content: String },
}

/// What an assistant message carries: an ending, or work in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssistantBody {
	/// Plain text and no tool calls. For a Worker this triggers a review; for a
	/// Comms Session it is something to say to the human.
	Text(String),
	/// Tool calls, and whatever the model said alongside them.
	Calls {
		preamble: Option<String>,
		calls: NonEmpty<ToolCall>,
	},
}

/// What one model call gave back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
	Text(String),
	Calls {
		preamble: Option<String>,
		calls: NonEmpty<ToolCall>,
	},
	/// The model replied with nothing at all. A Worker is sent back to work; a
	/// Comms Session simply says nothing.
	Empty,
}

/// One tool call, as the model asked for it. Arguments arrive as a JSON string
/// and are parsed by the tool that owns them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
	pub id: String,
	pub name: String,
	pub arguments: String,
}

/// One tool as it is offered to the model.
///
/// `parameters` is a JSON Schema object. It is written by hand in each tool
/// rather than derived, because the descriptions in it are prompt text and are
/// part of what is being tuned.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSchema {
	pub name: String,
	pub description: String,
	pub parameters: serde_json::Value,
}

/// What the transport brings back from one exchange: the reply, and what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
	pub reply: Reply,
	pub reasoning: Option<String>,
	pub tokens: u64,
	/// What the provider billed, taken from the response rather than worked out
	/// from a price list, so it stays right when pricing changes.
	pub cost: Cost,
}

/// A list that cannot be empty.
///
/// Used for the tool calls on an assistant message, where "called tools" and
/// "called no tools" are two different messages and must not be spelled the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmpty<T> {
	head: T,
	tail: Vec<T>,
}

impl<T> NonEmpty<T> {
	pub fn new(_head: T, _tail: Vec<T>) -> Self {
		unimplemented!()
	}

	/// Build from a list, or fail because it was empty.
	pub fn from_vec(_items: Vec<T>) -> Option<Self> {
		unimplemented!()
	}

	pub fn first(&self) -> &T {
		unimplemented!()
	}

	pub fn len(&self) -> usize {
		unimplemented!()
	}

	pub fn is_empty(&self) -> bool {
		false
	}

	pub fn iter(&self) -> impl Iterator<Item = &T> {
		std::iter::once(&self.head).chain(self.tail.iter())
	}
}

impl Message {
	/// One line naming what this message is, for the log. The body stays in the
	/// database.
	pub fn describe(&self) -> String {
		unimplemented!()
	}
}
