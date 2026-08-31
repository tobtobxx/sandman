//! Bench `Model` — replies the test writes, not the wire.
//!
//! Exercises the Harness (turn loop, scheduler ordering, review parsing,
//! `await_result` release) without a real model: same queue, same tier,
//! same one-call-at-a-time, only the answer is canned.
//!
//! Construct: [`ScriptedModel::new`] with ordered [`crate::domain::Completion`]s;
//! helpers [`ScriptedModel::saying`] (text), [`ScriptedModel::calling`] (one tool),
//! [`ScriptedModel::unreachable`] (transport error).
//! Use: [`crate::model::Model::send`] clones the [`crate::domain::CallRequest`] into
//! `seen` and returns the next queued reply FIFO; exhaustion is a transport error.
//! Inspect via [`ScriptedModel::requests`] for prompts and offered schemas.
//! Consumers: [`crate::bench::rig::RigBuilder`] via [`crate::bench::rig::ModelChoice::Scripted`]
//! (`Models::uniform`) and unit tests driving a [`crate::session::SessionCtx`] directly.
//!
//! Seam — sits *under* the scheduler so queue and tier stay real:
//! | Seam | Real | Bench |
//! | --- | --- | --- |
//! | `Model` | `OpenRouter` | `ScriptedModel` |
//!
//! Rules:
//! - **One queue for every [`crate::model::Purpose`]** — a scripted case is not asking which model would have replied (`Models::uniform`).
//! - **FIFO, no retry** — one `send` consumes one reply; `Transport` on empty or on queued `Err`.
//! - **Zero-cost fixtures** — helpers set tokens and cost to zero; build [`crate::domain::Completion`] directly to override.
//! - **Recording is a clone** — `requests()` returns a snapshot for system-prompt and tool-schema assertions.
//!
//! Defines: [`ScriptedModel`].

use async_trait::async_trait;

use crate::domain::{CallRequest, Completion};
use crate::model::{Model, ModelError};

/// Replies written by the test, returned FIFO.
///
/// Records every [`crate::domain::CallRequest`] for later assertion.
pub struct ScriptedModel {
	replies: std::sync::Mutex<
		std::collections::VecDeque<Result<Completion, String>>,
	>,
	seen: std::sync::Mutex<Vec<CallRequest>>,
}

impl ScriptedModel {
	/// Queue these completions for successive [`crate::model::Model::send`] calls.
	///
	/// Exhaustion returns a transport error naming the shortage.
	pub fn new(replies: Vec<Completion>) -> Self {
		ScriptedModel {
			replies: std::sync::Mutex::new(
				replies.into_iter().map(Ok).collect(),
			),
			seen: std::sync::Mutex::new(Vec::new()),
		}
	}

	/// Build a text [`crate::domain::Completion`] saying `text`.
	///
	/// Zero tokens and cost; the commonest fixture.
	pub fn saying(text: &str) -> Completion {
		Completion {
			reply: crate::domain::Reply::Text(text.to_string()),
			reasoning: None,
			tokens: 0,
			cost: crate::domain::Cost(0),
		}
	}

	/// Build a [`crate::domain::Completion`] that calls one tool `name`.
	///
	/// Stringifies `arguments`; id is `call-0`.
	pub fn calling(name: &str, arguments: serde_json::Value) -> Completion {
		let call = crate::domain::ToolCall {
			id: "call-0".to_string(),
			name: name.to_string(),
			arguments: arguments.to_string(),
		};
		Completion {
			reply: crate::domain::Reply::Calls {
				preamble: None,
				calls: crate::domain::NonEmpty::new(call, Vec::new()),
			},
			reasoning: None,
			tokens: 0,
			cost: crate::domain::Cost(0),
		}
	}

	/// Build a transport failure carrying `why`.
	///
	/// Queue as an `Err` reply to drive unreachable-model paths.
	pub fn unreachable(why: &str) -> Result<Completion, String> {
		Err(why.to_string())
	}

	/// Return every [`crate::domain::CallRequest`] sent so far, in order.
	///
	/// Cloned snapshot including system prompt and offered tools.
	pub fn requests(&self) -> Vec<CallRequest> {
		self.seen.lock().expect("not poisoned").clone()
	}
}

#[async_trait]
impl Model for ScriptedModel {
	fn name(&self) -> &str {
		"scripted"
	}

	async fn send(
		&self,
		request: &CallRequest,
	) -> Result<Completion, ModelError> {
		self.seen
			.lock()
			.expect("not poisoned")
			.push(request.clone());
		let next = self
			.replies
			.lock()
			.expect("not poisoned")
			.pop_front()
			.unwrap_or_else(|| {
				Err("ScriptedModel ran out of replies".to_string())
			});
		next.map_err(ModelError::Transport)
	}
}
