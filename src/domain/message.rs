//! The conversation: transcript, tool contract, and model exchange.
//!
//! One row per [`Message`] (`role` + `body_json`), oldest first; a transcript
//! is a query, not a blob. [`CallRequest`](super::call::CallRequest)
//! snapshots the transcript and the [`ToolSchema`]s offered to the model, and
//! [`Completion`] is what `Model::send` brings back in one attempt, no retry.
//! [`ToolSchema`] is hand-written JSON Schema per tool — descriptions are
//! prompt text, not derived — and [`NonEmpty`] makes "called tools but empty
//! list" unrepresentable.
//!
//! Construct via `Store::append_message` (one indexed row) and
//! `CallRequest { messages, tools }` before `Model::send`. `model.rs` maps
//! `Message` → private `WireMessage`, which has no `reasoning` field —
//! inspection never leaks back to the model.
//!
//! Consumers: `session::turn` builds the request and matches [`Reply`];
//! `model::OpenRouter` converts `Message` → `WireMessage` and `WireResponse` →
//! `Completion`; `db::rows` encodes `Message` as `role` + `json`;
//! `tools::Tool` supplies `ToolSchema`; `recall` and `web` read `Message` for display.
//!
//! Rules:
//! - **Text alongside tool calls is preamble, not an ending.** [`AssistantBody::Calls`]
//!   and [`Reply::Calls`] carry `preamble`; [`NonEmpty`] forbids empty calls.
//! - **Reasoning is inspection, never context.** Stored on `Assistant { reasoning }`,
//!   absent from the wire — stripping it is not a step to remember.
//! - **One system prompt, first.** A Session has exactly one `Message::System` at idx 0.
//! - **Tool answers are words.** Every tool returns `String`, including failures, so the model can read it.
//!
//! Defines: [`Message`], [`AssistantBody`], [`Reply`], [`ToolCall`],
//! [`ToolSchema`], [`Completion`], [`NonEmpty`].

use super::time::Cost;

/// One entry in a Session transcript.
///
/// Persisted as one row per entry, oldest first. `Assistant` carries `reasoning`
/// for inspection; it is never sent to the model.
#[derive(
	Debug,
	Clone,
	PartialEq,
	Eq,
	serde::Serialize,
	serde::Deserialize,
	strum::Display,
	strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Message {
	/// The Role's system prompt. Exactly one per Session, at idx 0.
	System { content: String },
	/// External input: Brief, mail, a child's Result, or metacognitive feedback.
	User { content: String },
	/// What the model said.
	Assistant {
		body: AssistantBody,
		/// Model reasoning for inspection. Never sent on the wire.
		reasoning: Option<String>,
	},
	/// What one tool answered. Always words, even on failure.
	Tool { tool_call_id: String, content: String },
}

/// What an assistant message carries.
///
/// Either the turn ends with text, or it continues with tool calls. Text
/// alongside calls is preamble, not an ending.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantBody {
	/// Plain text — triggers review for a Worker, said to the human for Comms.
	Text(String),
	/// Tool calls with optional preamble. Preamble is not a turn ending.
	Calls {
		preamble: Option<String>,
		calls: NonEmpty<ToolCall>,
	},
}

/// What one model call returned.
///
/// Mirrors [`AssistantBody`] but without `reasoning`; that travels on [`Completion`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
	Text(String),
	Calls {
		preamble: Option<String>,
		calls: NonEmpty<ToolCall>,
	},
}

/// One tool call as the model asked for it.
///
/// Arguments are a JSON string parsed by the owning tool.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolCall {
	pub id: String,
	pub name: String,
	pub arguments: String,
}

/// One tool as offered to the model.
///
/// `parameters` is a hand-written JSON Schema object; descriptions are prompt
/// text and part of what is tuned.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolSchema {
	pub name: String,
	pub description: String,
	pub parameters: serde_json::Value,
}

/// What the transport returned for one exchange.
///
/// `cost` is taken from the provider's response, not a local price list.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Completion {
	pub reply: Reply,
	pub reasoning: Option<String>,
	pub tokens: u64,
	/// Billed cost from the response. Stays right when pricing changes.
	pub cost: Cost,
}

/// A list that cannot be empty.
///
/// Guarantees `Calls` never holds an empty list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmpty<T> {
	head: T,
	tail: Vec<T>,
}

impl<T> NonEmpty<T> {
	/// Create from head and tail.
	pub fn new(head: T, tail: Vec<T>) -> Self {
		NonEmpty { head, tail }
	}

	/// Collect a vec; `None` if empty.
	pub fn from_vec(items: Vec<T>) -> Option<Self> {
		let mut items = items.into_iter();
		let head = items.next()?;
		Some(NonEmpty { head, tail: items.collect() })
	}

	/// First element.
	pub fn first(&self) -> &T {
		&self.head
	}

	/// Number of elements. Always `>= 1`.
	pub fn len(&self) -> usize {
		1 + self.tail.len()
	}

	/// Always `false`; present for collection parity.
	pub fn is_empty(&self) -> bool {
		false
	}

	/// Iterate head then tail.
	pub fn iter(&self) -> impl Iterator<Item = &T> {
		std::iter::once(&self.head).chain(self.tail.iter())
	}
}

/// Serialise as a plain array — `head`/`tail` never leaks to storage or wire.
impl<T: serde::Serialize> serde::Serialize for NonEmpty<T> {
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.collect_seq(self.iter())
	}
}

/// Deserialise a plain array; reject empty — invariant survives round-trip.
impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for NonEmpty<T> {
	fn deserialize<D: serde::Deserializer<'de>>(
		d: D,
	) -> Result<Self, D::Error> {
		let items = <Vec<T> as serde::Deserialize>::deserialize(d)?;
		NonEmpty::from_vec(items).ok_or_else(|| {
			serde::de::Error::custom("expected a non-empty array")
		})
	}
}

impl Message {
	/// Render for a reader: `[role]` prefix, tool calls inline.
	///
	/// For anything that reads a transcript as text rather than replaying it as
	/// context — `recall`'s Session view and metacognition. `reasoning` stays
	/// out; it is inspection and never travels. No trailing newline: callers
	/// separate entries themselves.
	pub fn render(&self) -> String {
		match self {
			// System - the Role's prompt
			Message::System { content } => format!("[system] {content}"),
			// User - Brief, mail, or feedback
			Message::User { content } => format!("[user] {content}"),
			// Assistant - text, or preamble above its calls
			Message::Assistant { body, .. } => match body {
				AssistantBody::Text(text) => format!("[assistant] {text}"),
				AssistantBody::Calls { preamble, calls } => {
					let mut out = String::new();
					if let Some(preamble) = preamble {
						out.push_str(&format!("[assistant] {preamble}\n"));
					}
					for call in calls.iter() {
						out.push_str(&format!(
							"  tool call: {}({})\n",
							call.name, call.arguments
						));
					}
					out.trim_end().to_string()
				},
			},
			// Tool - result for one call
			Message::Tool { content, .. } => {
				format!("[tool result] {content}")
			},
		}
	}

	/// One-line kind for the log. Body stays in the database.
	pub fn describe(&self) -> String {
		match self {
			// System - single prompt at idx 0
			Message::System { .. } => "system".to_string(),
			// User - Brief, mail, or feedback
			Message::User { .. } => "user".to_string(),
			// Assistant - text or tool calls
			Message::Assistant { body, .. } => match body {
				AssistantBody::Text(_) => "assistant: text".to_string(),
				AssistantBody::Calls { calls, .. } => {
					format!("assistant: {} tool call(s)", calls.len())
				},
			},
			// Tool - result for one call
			Message::Tool { tool_call_id, .. } => {
				format!("tool result for {tool_call_id}")
			},
		}
	}
}
