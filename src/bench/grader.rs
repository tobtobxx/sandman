//! Verification a model has to do.
//!
//! **Rare, and warranted.** A grader is a model judgement about a model
//! judgement: it costs a call, it varies between runs, and it can be wrong in
//! both directions. Reach for one only when nothing countable answers the
//! question. That exactly one `create_task` call was made is a count; that it
//! carries *the Brief that was wanted* — that it kept the greeting, kept the
//! delay, and added nothing — is a judgement.
//!
//! A grader runs against [`crate::model::GRADER_MODEL`], which is stronger than
//! the one the swarm uses: a judge no better than what it judges is not a judge.
//! It is bench machinery and not part of the swarm: the call goes straight to
//! the model, not through the scheduler, and what it costs is reported
//! separately and never counts as Spend.
//!
//! Graders run only after every goal check has passed. There is nothing to judge
//! about a run that already failed on something countable.
//!
//! **A reply with no verdict in it is a FAIL.** An unparseable judgement must
//! never quietly pass.
//!
//! Defines: [`Grader`], [`GraderOutcome`], [`Verdict`], [`run`], [`default_judge`].

use crate::domain::{CallRequest, Cost, Message, Reply};
use crate::model::{Model, OpenRouter, API_KEY, ENDPOINT, GRADER_MODEL};

/// What a grader is told it is doing.
pub const GRADER_SYSTEM: &str = "\
You grade what an agent did against what was wanted.
Be strict and literal: grade what is written, not what was probably meant.
End your reply with a verdict on its own line: <verdict>pass</verdict> or <verdict>fail</verdict>.";

/// A model's judgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
	Pass,
	Fail,
}

/// One question put to a model about a finished run.
pub struct Grader {
	pub name: String,
	/// The whole user message the grader judges. Built by the case that owns it,
	/// out of the run's state and the calls the Session made.
	pub input: String,
	/// How to read the reply. [`default_judge`] looks for the verdict tag.
	pub judge: Option<Box<dyn Fn(&str) -> (Verdict, String) + Send + Sync>>,
}

/// What one grader found.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GraderOutcome {
	pub name: String,
	pub verdict: Verdict,
	pub detail: String,
	/// The grader's whole reply, kept for when a marginal verdict needs reading.
	pub raw: String,
	pub cost: Cost,
}

/// Run one grader.
///
/// Fails only on transport trouble; a `fail` verdict is a normal outcome, not an
/// error.
pub async fn run(
	grader: &Grader,
) -> Result<GraderOutcome, crate::model::ModelError> {
	// Unlike the swarm's own model, `GRADER_MODEL` refuses to have reasoning
	// disabled outright (HTTP 400: "Reasoning is mandatory for this endpoint");
	// asking for the least of it is the equivalent for a model that insists.
	let model = OpenRouter::new(
		ENDPOINT,
		API_KEY,
		GRADER_MODEL,
		Some("low".to_string()),
	);
	let request = CallRequest {
		messages: vec![
			Message::System { content: GRADER_SYSTEM.to_string() },
			Message::User { content: grader.input.clone() },
		],
		tools: Vec::new(),
	};
	let completion = model.send(&request).await?;

	let text = match &completion.reply {
		Reply::Text(text) => text.clone(),
		Reply::Calls { preamble, .. } => preamble.clone().unwrap_or_default(),
	};
	let (verdict, detail) = match &grader.judge {
		Some(judge) => judge(&text),
		None => default_judge(&text),
	};

	Ok(GraderOutcome {
		name: grader.name.clone(),
		verdict,
		detail,
		raw: text,
		cost: completion.cost,
	})
}

/// Look for `<verdict>pass</verdict>` or `<verdict>fail</verdict>`.
///
/// No tag is a FAIL.
pub fn default_judge(reply: &str) -> (Verdict, String) {
	let lower = reply.to_lowercase();
	if lower.contains("<verdict>pass</verdict>") {
		(Verdict::Pass, reply.to_string())
	} else if lower.contains("<verdict>fail</verdict>") {
		(Verdict::Fail, reply.to_string())
	} else {
		(
			Verdict::Fail,
			format!("no <verdict> tag in the reply: {reply}"),
		)
	}
}
