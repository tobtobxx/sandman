//! The transport: the one place that talks to a model over the wire.
//!
//! [`Model`] is a seam, and it sits *under* the scheduler on purpose. A bench
//! that swaps in a scripted model still exercises the real queue, the real tier
//! ordering and the real one-call-at-a-time invariant; only the answer is
//! written by the test.
//!
//! The wire shape is private to this module. That is what keeps recorded
//! reasoning off the wire: a stored assistant message carries the model's
//! reasoning for a Watcher to read, and [`WireMessage`] simply has no field for
//! it. Stripping it is not a step anyone has to remember.
//!
//! Ordering — which call runs next — is `scheduler.rs`'s. What lives here is one
//! request, sent once, with no retry: a failed call already has a full path to a
//! failed Result.
//!
//! Defines: [`Model`], [`OpenRouter`], [`ModelError`], [`MODEL`],
//! [`GRADER_MODEL`], [`API_KEY`].

use async_trait::async_trait;

use crate::domain::{CallRequest, Completion, Cost, NonEmpty, Reply, ToolCall};

/// This is a prototype key with a low limit; leaking it costs nothing.
pub const API_KEY: &str =
	"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";

/// The model every Session, every review and every interrupt talks to.
pub const MODEL: &str = "qwen/qwen3.6-35b-a3b";

/// The model a bench grader talks to. Deliberately stronger than [`MODEL`]: a
/// judge no better than what it judges is not a judge. Nothing in the swarm ever
/// uses it, and what it costs is never Spend.
pub const GRADER_MODEL: &str = "z-ai/glm-5.3-flash";

/// Chat completions, as OpenRouter speaks them.
pub const ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";

/// One exchange with a model.
///
/// The seam a bench replaces. Two adapters exist: [`OpenRouter`], and the
/// scripted model in [`crate::bench`].
#[async_trait]
pub trait Model: Send + Sync {
	/// The model's name, as it goes on the call record.
	fn name(&self) -> &str;

	/// Send one request and bring back what came of it.
	///
	/// One attempt, no retry. A transport failure is a [`ModelError`]; a model
	/// that answers with nothing is a successful call carrying
	/// [`crate::domain::Reply::Text`] holding an empty string.
	async fn send(
		&self,
		request: &CallRequest,
	) -> Result<Completion, ModelError>;
}

/// Why an exchange did not happen.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
	#[error("could not reach the model: {0}")]
	Transport(String),
	#[error("HTTP {status}: {body}")]
	Status { status: u16, body: String },
	#[error("the model's answer could not be read: {0}")]
	Malformed(String),
}

/// The real transport.
pub struct OpenRouter {
	client: reqwest::Client,
	endpoint: String,
	api_key: String,
	model: String,
	/// How much the model may think before it answers. `None` asks for no
	/// thinking at all; `Some(effort)` is sent as `reasoning: {"effort":
	/// effort}` verbatim.
	effort: Option<String>,
}

impl OpenRouter {
	/// Read the endpoint, key and model off the constants above.
	pub fn from_env() -> Self {
		Self::new(ENDPOINT, API_KEY, MODEL, None)
	}

	pub fn new(
		endpoint: &str,
		api_key: &str,
		model: &str,
		effort: Option<String>,
	) -> Self {
		OpenRouter {
			client: reqwest::Client::new(),
			endpoint: endpoint.to_string(),
			api_key: api_key.to_string(),
			model: model.to_string(),
			effort,
		}
	}
}

#[async_trait]
impl Model for OpenRouter {
	fn name(&self) -> &str {
		&self.model
	}

	async fn send(
		&self,
		request: &CallRequest,
	) -> Result<Completion, ModelError> {
		let body = ChatRequest {
			model: &self.model,
			messages: request.messages.iter().map(WireMessage::from).collect(),
			tools: request
				.tools
				.iter()
				.map(|t| WireTool {
					kind: "function",
					function: WireFunction {
						name: t.name.clone(),
						description: t.description.clone(),
						parameters: t.parameters.clone(),
					},
				})
				.collect(),
			reasoning: Some(match &self.effort {
				None => WireReasoning { enabled: Some(false), effort: None },
				Some(effort) => WireReasoning {
					enabled: None,
					effort: Some(effort.clone()),
				},
			}),
			chat_template_kwargs: self
				.effort
				.is_none()
				.then_some(WireChatTemplateKwargs { enable_thinking: false }),
			usage: WireUsageConfig { include: true },
		};

		let response = self
			.client
			.post(&self.endpoint)
			.bearer_auth(&self.api_key)
			.json(&body)
			.send()
			.await
			.map_err(|e| ModelError::Transport(e.to_string()))?;

		let status = response.status();
		let text = response
			.text()
			.await
			.map_err(|e| ModelError::Transport(e.to_string()))?;

		if !status.is_success() {
			return Err(ModelError::Status {
				status: status.as_u16(),
				body: text,
			});
		}

		let parsed: WireResponse = serde_json::from_str(&text)
			.map_err(|e| ModelError::Malformed(e.to_string()))?;

		let choice = parsed.choices.into_iter().next().ok_or_else(|| {
			ModelError::Malformed("no choices in response".into())
		})?;

		let reply = match choice.message.tool_calls.filter(|c| !c.is_empty()) {
			Some(calls) => {
				let calls = calls
					.into_iter()
					.map(|c| ToolCall {
						id: c.id,
						name: c.function.name,
						arguments: c.function.arguments,
					})
					.collect();
				let calls =
					NonEmpty::from_vec(calls).expect("checked non-empty above");
				Reply::Calls { preamble: choice.message.content, calls }
			},
			None => Reply::Text(choice.message.content.unwrap_or_default()),
		};

		let usage = parsed.usage.unwrap_or_default();
		let cost =
			Cost((usage.cost.unwrap_or(0.0) * 1_000_000_000.0).round() as i64);

		Ok(Completion {
			reply,
			reasoning: choice.message.reasoning,
			tokens: usage.total_tokens.unwrap_or(0),
			cost,
		})
	}
}

// --- The wire shape --------------------------------------------------------
// Private on purpose. Nothing outside this module builds a request body, so
// nothing outside this module can put recorded reasoning back on the wire.

/// The request body, as the chat-completions API wants it.
#[derive(serde::Serialize)]
struct ChatRequest<'a> {
	model: &'a str,
	messages: Vec<WireMessage>,
	#[serde(skip_serializing_if = "Vec::is_empty")]
	tools: Vec<WireTool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	reasoning: Option<WireReasoning>,
	#[serde(skip_serializing_if = "Option::is_none")]
	chat_template_kwargs: Option<WireChatTemplateKwargs>,
	usage: WireUsageConfig,
}

#[derive(serde::Serialize)]
struct WireReasoning {
	#[serde(skip_serializing_if = "Option::is_none")]
	enabled: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	effort: Option<String>,
}

#[derive(serde::Serialize)]
struct WireChatTemplateKwargs {
	enable_thinking: bool,
}

/// Asks OpenRouter to report what a call actually cost, so [`Completion::cost`]
/// comes off the response rather than a price list kept here.
#[derive(serde::Serialize)]
struct WireUsageConfig {
	include: bool,
}

/// One message, as the chat-completions API wants it.
///
/// Note what is missing: there is no `reasoning` field. A domain message carries
/// the model's own reasoning for inspection, and the conversion below drops it.
#[derive(serde::Serialize)]
struct WireMessage {
	role: &'static str,
	#[serde(skip_serializing_if = "Option::is_none")]
	content: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	tool_calls: Option<Vec<WireToolCall>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	tool_call_id: Option<String>,
}

#[derive(serde::Serialize)]
struct WireToolCall {
	id: String,
	#[serde(rename = "type")]
	kind: &'static str,
	function: WireFunctionCall,
}

#[derive(serde::Serialize)]
struct WireFunctionCall {
	name: String,
	arguments: String,
}

#[derive(serde::Serialize)]
struct WireTool {
	#[serde(rename = "type")]
	kind: &'static str,
	function: WireFunction,
}

#[derive(serde::Serialize)]
struct WireFunction {
	name: String,
	description: String,
	parameters: serde_json::Value,
}

impl From<&crate::domain::Message> for WireMessage {
	fn from(m: &crate::domain::Message) -> WireMessage {
		use crate::domain::{AssistantBody, Message};
		match m {
			Message::System { content } => WireMessage {
				role: "system",
				content: Some(content.clone()),
				tool_calls: None,
				tool_call_id: None,
			},
			Message::User { content } => WireMessage {
				role: "user",
				content: Some(content.clone()),
				tool_calls: None,
				tool_call_id: None,
			},
			Message::Assistant { body, .. } => match body {
				AssistantBody::Text(text) => WireMessage {
					role: "assistant",
					content: Some(text.clone()),
					tool_calls: None,
					tool_call_id: None,
				},
				AssistantBody::Calls { preamble, calls } => WireMessage {
					role: "assistant",
					content: preamble.clone(),
					tool_calls: Some(
						calls
							.iter()
							.map(|c| WireToolCall {
								id: c.id.clone(),
								kind: "function",
								function: WireFunctionCall {
									name: c.name.clone(),
									arguments: c.arguments.clone(),
								},
							})
							.collect(),
					),
					tool_call_id: None,
				},
			},
			Message::Tool { tool_call_id, content } => WireMessage {
				role: "tool",
				content: Some(content.clone()),
				tool_calls: None,
				tool_call_id: Some(tool_call_id.clone()),
			},
		}
	}
}

/// The response body, as the chat-completions API returns it. Only the fields
/// this module reads are named; everything else is dropped on deserialize.
#[derive(serde::Deserialize)]
struct WireResponse {
	choices: Vec<WireChoice>,
	#[serde(default)]
	usage: Option<WireResponseUsage>,
}

#[derive(serde::Deserialize)]
struct WireChoice {
	message: WireResponseMessage,
}

#[derive(serde::Deserialize)]
struct WireResponseMessage {
	content: Option<String>,
	#[serde(default)]
	reasoning: Option<String>,
	#[serde(default)]
	tool_calls: Option<Vec<WireResponseToolCall>>,
}

#[derive(serde::Deserialize)]
struct WireResponseToolCall {
	id: String,
	function: WireResponseFunctionCall,
}

#[derive(serde::Deserialize)]
struct WireResponseFunctionCall {
	name: String,
	arguments: String,
}

#[derive(Default, serde::Deserialize)]
struct WireResponseUsage {
	#[serde(default)]
	total_tokens: Option<u64>,
	/// Present because the request set [`WireUsageConfig::include`].
	#[serde(default)]
	cost: Option<f64>,
}
