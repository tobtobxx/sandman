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
//! Defines: [`Model`], [`OpenRouter`], [`ModelError`], [`ReasoningEffort`],
//! [`MODEL`], [`GRADER_MODEL`], [`API_KEY`].

use async_trait::async_trait;

use crate::domain::{CallRequest, Completion};

/// This is a prototype key with a low limit; leaking it costs nothing.
/// `OPENROUTER_API_KEY` overrides it.
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

/// How much the model may think before it answers.
///
/// `SANDMAN_REASONING_EFFORT` overrides the default, so a bench run can compare
/// levels without an edit here. It applies to Workers, Comms and metacognition
/// alike; the bench's grader talks to a different model, builds its own request
/// and is left at that model's own default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReasoningEffort {
	/// No thinking at all.
	///
	/// Sends two fields, because the two endpoints this project talks to want
	/// different ones and each ignores the other's without complaint:
	/// `reasoning: {enabled: false}`, which OpenRouter honours, and
	/// `chat_template_kwargs: {enable_thinking: false}`, which a local llama.cpp
	/// server honours.
	///
	/// The trap: `openai/gpt-oss-*` on OpenRouter answers the first of those
	/// with HTTP 400. Pointing [`MODEL`] at a gpt-oss means raising this to
	/// `Minimal` in the same edit.
	#[default]
	None,
	Minimal,
	Low,
	Medium,
	High,
}

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
	/// [`crate::domain::Reply::Empty`].
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
	effort: ReasoningEffort,
}

impl OpenRouter {
	/// Read the endpoint, key, model and reasoning effort from the constants
	/// above, letting the environment override each.
	pub fn from_env() -> Self {
		unimplemented!()
	}

	pub fn new(
		_endpoint: &str,
		_api_key: &str,
		_model: &str,
		_effort: ReasoningEffort,
	) -> Self {
		unimplemented!()
	}
}

#[async_trait]
impl Model for OpenRouter {
	fn name(&self) -> &str {
		unimplemented!()
	}

	async fn send(
		&self,
		_request: &CallRequest,
	) -> Result<Completion, ModelError> {
		unimplemented!()
	}
}

// --- The wire shape --------------------------------------------------------
// Private on purpose. Nothing outside this module builds a request body, so
// nothing outside this module can put recorded reasoning back on the wire.

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
	fn from(_m: &crate::domain::Message) -> WireMessage {
		unimplemented!()
	}
}
