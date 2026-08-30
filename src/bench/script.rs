//! A model whose replies the test writes.
//!
//! For testing the Harness rather than the model: the turn loop, the scheduler's
//! ordering, the review's parsing, a Worker released from `await_result`. None of
//! those questions need a real model, and asking one makes the answer slow,
//! expensive and only mostly repeatable.
//!
//! A bench case measuring the model does not use this. A test measuring Sandman
//! almost always should.
//!
//! Defines: [`ScriptedModel`].

use async_trait::async_trait;

use crate::domain::{CallRequest, Completion};
use crate::model::{Model, ModelError};

/// Answers from a list, in order.
pub struct ScriptedModel {
	replies: std::sync::Mutex<
		std::collections::VecDeque<Result<Completion, String>>,
	>,
	seen: std::sync::Mutex<Vec<CallRequest>>,
}

impl ScriptedModel {
	/// Answer these, in order.
	///
	/// Running out is a failure of the test, not of the code under test, and it
	/// says so.
	pub fn new(replies: Vec<Completion>) -> Self {
		ScriptedModel {
			replies: std::sync::Mutex::new(
				replies.into_iter().map(Ok).collect(),
			),
			seen: std::sync::Mutex::new(Vec::new()),
		}
	}

	/// Reply with plain text, once. The commonest fixture.
	pub fn saying(text: &str) -> Completion {
		Completion {
			reply: crate::domain::Reply::Text(text.to_string()),
			reasoning: None,
			tokens: 0,
			cost: crate::domain::Cost(0),
		}
	}

	/// Reply by calling one tool, once.
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

	/// A call that fails on the wire, for testing what an unreachable model does
	/// to a Worker, a Comms Session and a review.
	pub fn unreachable(why: &str) -> Result<Completion, String> {
		Err(why.to_string())
	}

	/// Every request this model was sent, in order — so a test can assert on
	/// what a Session actually put in front of the model, including the system
	/// prompt it was given and the tools it was offered.
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
