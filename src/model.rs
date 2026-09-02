//! The transport — one place that talks to a model over the wire.
//!
//! Construct: `OpenRouter::from_spec(spec)` per `ModelSpec`; `Models::from_config(config)` dedupes by value so one pool per endpoint; `Models::uniform(model)` for benches that script answers.
//! Use: `Models::pick(purpose) → Arc<dyn Model>` then `model.send(request) → Completion`; scheduler decides *when*, `Model` decides *how*.
//! Consumers: `scheduler::Scheduler::request` (sole caller of `send`); `Harness` builds `Models`; `bench` replaces `Model` with scripted replies.
//!
//! Seam: `Model` — real `OpenRouter`, bench scripted. Wire shape is private so recorded reasoning never reaches the wire.
//!
//! | `Model` | `OpenRouter` | bench scripted |
//! | --- | --- | --- |
//! | `name` | `spec.model` | fixed test name |
//! | `send` | POST chat-completions → `Completion` | canned `Completion` |
//!
//! Rules:
//! **Under the scheduler.** Swapping `Model` still exercises queue, tier ordering and one-at-a-time.
//! **One attempt, no retry.** A failure already has a path to `TaskState::Failed` via `SchedulerError::Call`.
//! **Wire shape is private.** `WireMessage` has no `reasoning` field — domain reasoning is dropped on conversion.
//! **Same spec shares a pool.** Equal `ModelSpec`s share one `Arc<dyn Model>`.
//! **No default in code.** `Purpose` has no default variant; `config` falls back to `models.all` once.
//!
//! Defines: `Model`, `Models`, `Purpose`, `OpenRouter`, `ModelError`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::config::{Config, ModelSpec};
use crate::domain::{
	CallRequest, Completion, Cost, NonEmpty, Reply, ToolCall, Usage,
};
use crate::roles::RoleName;

/// One exchange with a model.
///
/// The seam benches replace. `OpenRouter` is the wire adapter.
#[async_trait]
pub trait Model: Send + Sync {
	/// Returns the model name as recorded on the call.
	fn name(&self) -> &str;

	/// Send one request.
	///
	/// One attempt, no retry. Empty answer is `Reply::Text` with empty string.
	async fn send(
		&self,
		request: &CallRequest,
	) -> Result<Completion, ModelError>;

	/// Probe the model with a tiny `max_tokens: 1` request.
	///
	/// Used at startup to fail fast when a model is unreachable.
	async fn probe(&self) -> Result<(), ModelError>;
}

/// Why `Model::send` failed.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
	#[error("could not reach the model: {0}")]
	Transport(String),
	#[error("HTTP {status}: {body}")]
	Status { status: u16, body: String },
	#[error("the model's answer could not be read: {0}")]
	Malformed(String),
}

/// The real transport over OpenRouter.
pub struct OpenRouter {
	client: reqwest::Client,
	endpoint: String,
	api_key: String,
	model: String,
	/// Reasoning effort. `None` sends no thinking; `Some(level)` sends `reasoning: {"effort": level}`.
	effort: Option<String>,
}

impl OpenRouter {
	/// Build one transport from a spec.
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

/// What a call is for.
///
/// Resolved to a `Model` via `Models::pick`. No default variant — fallback is in `Config`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
	/// A Comms Session talking to a human.
	Comms,
	/// A Worker on a Task of this Role.
	Work(RoleName),
	/// A review or an interrupt.
	Metacognition,
}

/// Every model a swarm may talk to, one per `Purpose`.
///
/// Built once at startup. Equal `ModelSpec`s share one adapter and pool.
pub struct Models {
	comms: Arc<dyn Model>,
	metacognition: Arc<dyn Model>,
	/// One entry per `RoleName`, from `VARIANTS`.
	work: HashMap<RoleName, Arc<dyn Model>>,
}

impl Models {
	/// Build from `Config`.
	///
	/// Deduplicates by `ModelSpec` value so one pool per endpoint.
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

	/// Build with one model for every `Purpose`.
	///
	/// For benches that script every answer.
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

	/// Pick the model for a `Purpose`.
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
			max_tokens: None,
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

		// Count and price the exchange. `prompt_tokens` counts the whole prompt,
		// cache hits included, so what was processed is the remainder.
		let wire = parsed.usage.unwrap_or_default();
		let cached = wire.prompt_tokens_details.cached_tokens;
		let usage = Usage {
			cached,
			prefill: wire.prompt_tokens.saturating_sub(cached),
			produced: wire.completion_tokens,
			cost: Cost(
				(wire.cost.unwrap_or(0.0) * 1_000_000_000.0).round() as i64
			),
		};

		Ok(Completion { reply, reasoning: choice.message.reasoning, usage })
	}

	async fn probe(&self) -> Result<(), ModelError> {
		let body = ChatRequest {
			model: &self.model,
			messages: vec![WireMessage {
				role: "user",
				content: Some("ping".to_string()),
				tool_calls: None,
				tool_call_id: None,
			}],
			tools: Vec::new(),
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
			max_tokens: Some(1),
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
		if parsed.choices.is_empty() {
			return Err(ModelError::Malformed("no choices in response".into()));
		}
		Ok(())
	}
}

// Wire shape — private so reasoning never reaches the wire.

/// Wire request body for chat-completions.
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
	#[serde(skip_serializing_if = "Option::is_none")]
	max_tokens: Option<u32>,
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

/// Ask OpenRouter to include cost in the response.
#[derive(serde::Serialize)]
struct WireUsageConfig {
	include: bool,
}

/// One wire message.
///
/// No `reasoning` field — domain reasoning is dropped on conversion.
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

/// Wire response. Only fields this module reads are named.
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
	prompt_tokens: u64,
	#[serde(default)]
	completion_tokens: u64,
	#[serde(default)]
	prompt_tokens_details: WireResponsePromptDetails,
	/// Present when `WireUsageConfig::include` was set. A local provider
	/// bills nothing and sends none.
	#[serde(default)]
	cost: Option<f64>,
}

/// The cache half of the prompt count; absent from providers that do not cache.
#[derive(Default, serde::Deserialize)]
struct WireResponsePromptDetails {
	#[serde(default)]
	cached_tokens: u64,
}
