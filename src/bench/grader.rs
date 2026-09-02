//! Model judgement about model judgement, for what counts cannot decide.
//!
//! A grader exists because some questions have no countable answer — whether a
//! Brief kept the greeting, kept the delay, added nothing. That is a judgement,
//! not a count; it costs a call that varies between runs and can be wrong both
//! ways. Reach for one only when nothing countable answers the question.
//!
//! Construct: `Grader { name, input, judge }` — built by the case from the run's
//! state and its `RecordedToolCall`s; `judge` defaults to [`default_judge`].
//! Use: `run(grader, spec)` → [`GraderOutcome`]; `spec` is `config.for_grader()`.
//! A `Fail` verdict is a normal outcome, only transport returns `Err`.
//! Consume: `report::assemble` drives graders after tripwires and goal checks
//! have passed; cases supply them via `cases::finish`'s `graders` vec.
//!
//! **Seam — swarm call vs grader call:**
//!
//! | | Swarm | Grader |
//! |---|---|---|
//! | path | `Models` → `Scheduler` → `Model::send` | `OpenRouter::from_spec` → `Model::send` direct |
//! | cost | `Spend` | `Cost` kept apart, never in `Spend` |
//! | model | `config.for_all` / `for_role` / `for_comms` | `[bench].grader` — strictly stronger than the swarm's |
//!
//! Rules: **graders run only after every check passes** — nothing to judge on a
//! run that already failed countably. **No verdict is a Fail** — an unparseable
//! reply must never quietly pass. **Bench machinery, not swarm** — a grader call
//! never touches the scheduler or [`crate::model::Models`].
//!
//! Defines: [`Grader`], [`GraderOutcome`], [`Verdict`], [`run`], [`default_judge`].

use crate::config::ModelSpec;
use crate::domain::{CallRequest, Cost, Message, Reply};
use crate::model::{Model, OpenRouter};

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

/// How to read a grader's reply: maps the whole reply text to a verdict and
/// a human-readable detail line.
pub type Judge = Box<dyn Fn(&str) -> (Verdict, String) + Send + Sync>;

/// One question put to a model about a finished run.
pub struct Grader {
	pub name: String,
	/// The whole user message the grader judges. Built by the case that owns it,
	/// out of the run's state and the calls the Session made.
	pub input: String,
	/// How to read the reply. [`default_judge`] looks for the verdict tag.
	pub judge: Option<Judge>,
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

/// Send one grader request and judge its reply.
///
/// Builds the request from `GRADER_SYSTEM` and the grader's input, waits on the
/// model, and parses the verdict. Returns an outcome on any reply; fails only
/// on transport.
pub async fn run(
	grader: &Grader,
	spec: &ModelSpec,
) -> Result<GraderOutcome, crate::model::ModelError> {
	// Build grader model
	let model = OpenRouter::from_spec(spec);

	// Build request
	let request = CallRequest {
		messages: vec![
			Message::System { content: GRADER_SYSTEM.to_string() },
			Message::User { content: grader.input.clone() },
		],
		tools: Vec::new(),
	};

	// Send request
	let completion = model.send(&request).await?;

	// Extract reply text
	let text = match &completion.reply {
		// Only text — use directly
		Reply::Text(text) => text.clone(),
		// Tool calls — use preamble
		Reply::Calls { preamble, .. } => preamble.clone().unwrap_or_default(),
	};

	// Judge verdict
	let (verdict, detail) = match &grader.judge {
		Some(judge) => judge(&text),
		None => default_judge(&text),
	};

	// Build outcome
	Ok(GraderOutcome {
		name: grader.name.clone(),
		verdict,
		detail,
		raw: text,
		cost: completion.cost,
	})
}

/// Parse a verdict tag from a grader reply.
///
/// Looks for `<verdict>pass</verdict>` or `<verdict>fail</verdict>`.
/// No tag is a `Fail`.
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
