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
	pub fn new(_replies: Vec<Completion>) -> Self {
		unimplemented!()
	}

	/// Reply with plain text, once. The commonest fixture.
	pub fn saying(_text: &str) -> Completion {
		unimplemented!()
	}

	/// Reply by calling one tool, once.
	pub fn calling(_name: &str, _arguments: serde_json::Value) -> Completion {
		unimplemented!()
	}

	/// A call that fails on the wire, for testing what an unreachable model does
	/// to a Worker, a Comms Session and a review.
	pub fn unreachable(_why: &str) -> Result<Completion, String> {
		unimplemented!()
	}

	/// Every request this model was sent, in order — so a test can assert on
	/// what a Session actually put in front of the model, including the system
	/// prompt it was given and the tools it was offered.
	pub fn requests(&self) -> Vec<CallRequest> {
		unimplemented!()
	}
}

#[async_trait]
impl Model for ScriptedModel {
	fn name(&self) -> &str {
		"scripted"
	}

	async fn send(
		&self,
		_request: &CallRequest,
	) -> Result<Completion, ModelError> {
		unimplemented!()
	}
}
