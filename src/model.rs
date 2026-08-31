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
//! Which model a call goes to is [`Purpose`], resolved through [`Models`]. Two
//! Roles may share one model or have one each; the swarm asks for a purpose and
//! never for a name.
//!
//! Defines: [`Model`], [`Models`], [`Purpose`], [`OpenRouter`], [`ModelError`].

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::config::{Config, ModelSpec};
use crate::domain::{CallRequest, Completion, Cost, NonEmpty, Reply, ToolCall};
use crate::roles::RoleName;

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
	/// One model, as the configuration describes it.
	pub fn from_spec(spec: &ModelSpec) -> Self {
		OpenRouter {
			client: reqwest::Client::new(),
			endpoint: spec.endpoint.clone(),
			api_key: spec.api_key.clone(),
			model: spec.model.clone(),
			effort: spec.effort.clone(),
		}
	}
}

/// What a call is for, which is how it finds its model.
///
/// A property of the Session making it, like [`crate::scheduler::Tier`] is a
/// property of the caller. There is no variant for "whatever the default is":
/// the fallback to `models.all` happens once, in the configuration, so nothing
/// downstream has to know a default exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
	/// A Comms Session, talking to a human.
	Comms,
	/// A Worker on a Task of this Role.
	Work(RoleName),
	/// A review or an interrupt.
	Metacognition,
}

/// Every model a swarm may talk to, one per [`Purpose`].
///
/// Built once at startup. Two purposes that name the same model share one
/// adapter, and so one connection pool — resolving them separately would open a
/// second pool to the same endpoint for no reason.
pub struct Models {
	comms: Arc<dyn Model>,
	metacognition: Arc<dyn Model>,
	/// Total over [`RoleName`]: filled from `VARIANTS`, so every Role has one.
	work: HashMap<RoleName, Arc<dyn Model>>,
}

impl Models {
	/// What the configuration says.
	pub fn from_config(config: &Config) -> Models {
		let mut built: HashMap<ModelSpec, Arc<dyn Model>> = HashMap::new();
		let mut of = |spec: &ModelSpec| -> Arc<dyn Model> {
			built
				.entry(spec.clone())
				.or_insert_with(|| Arc::new(OpenRouter::from_spec(spec)))
				.clone()
		};

		Models {
			comms: of(config.for_comms()),
			metacognition: of(config.for_metacognition()),
			work: <RoleName as strum::VariantArray>::VARIANTS
				.iter()
				.map(|role| (*role, of(config.for_role(*role))))
				.collect(),
		}
	}

	/// One model for everything. What a bench passes: a case that scripts the
	/// answers is not asking which model would have given them.
	pub fn uniform(model: Arc<dyn Model>) -> Models {
		Models {
			comms: model.clone(),
			metacognition: model.clone(),
			work: <RoleName as strum::VariantArray>::VARIANTS
				.iter()
				.map(|role| (*role, model.clone()))
				.collect(),
		}
	}

	/// The model this call goes to.
	pub fn pick(&self, purpose: Purpose) -> &Arc<dyn Model> {
		match purpose {
			Purpose::Comms => &self.comms,
			Purpose::Metacognition => &self.metacognition,
			Purpose::Work(role) => {
				self.work.get(&role).expect("every Role is built above")
			},
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
		// Build request
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

		// Send request
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

		// Check status
		if !status.is_success() {
			return Err(ModelError::Status {
				status: status.as_u16(),
				body: text,
			});
		}

		// Parse response
		let parsed: WireResponse = serde_json::from_str(&text)
			.map_err(|e| ModelError::Malformed(e.to_string()))?;

		let choice = parsed.choices.into_iter().next().ok_or_else(|| {
			ModelError::Malformed("no choices in response".into())
		})?;

		// Build reply
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

		// Compute cost
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
