//! Reaching a human.
//!
//! There is one route to a human and this is it. A Worker cannot address a Comms
//! Session with a Task — every Task becomes a Worker Session — so a Worker that
//! decides a human must know something creates a `planning` Task, and the
//! planning Worker that runs it calls this. The message lands in the Comms
//! Session's mailbox on the Channel it names, and that Session decides how to
//! put it: word for word, or reworded with the context the human needs.
//!
//! The `channel` enum is built from the Channels that are open right now, which
//! is the reason tool schemas are built per Session rather than declared once.
//!
//! Known weak spot: with more than one Channel open, the Worker is choosing
//! *which human to tell*, and it has nothing solid to choose with — a Task
//! carries no record of where it came from, because a Brief stands alone. It
//! reads the Brief and guesses. See TASKS.md.
//!
//! Defines: [`MessageHuman`].

use std::str::FromStr;

use async_trait::async_trait;

use crate::domain::{ChannelId, IncomingFrom, ToolSchema};
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;

use super::{Tool, ToolError};

/// Inject a message into the Comms Session on a named Channel.
pub struct MessageHuman;

#[async_trait]
impl Tool for MessageHuman {
	fn name(&self) -> ToolName {
		ToolName::MessageHuman
	}

	/// The `channel` argument is an enum of the open Channels, each labelled
	/// with its kind, so the model names one that exists.
	fn schema(&self, ctx: &SchemaCtx) -> ToolSchema {
		let ids: Vec<String> = ctx
			.open_channels
			.iter()
			.map(|(id, _)| id.to_string())
			.collect();
		let listing: Vec<String> = ctx
			.open_channels
			.iter()
			.map(|(id, kind)| format!("{id} ({})", kind.discriminant()))
			.collect();
		ToolSchema {
			name: self.name().to_string(),
			description:
				"Send a message into the Comms Session on a named Channel, \
				 so it reaches the human."
					.to_string(),
			parameters: serde_json::json!({
				"type": "object",
				"properties": {
					"channel": {
						"type": "string",
						"enum": ids,
						"description": format!(
							"Which Channel to speak on. Open now: {}.",
							listing.join(", ")
						),
					},
					"text": {
						"type": "string",
						"description": "What to tell the human.",
					},
				},
				"required": ["channel", "text"],
			}),
		}
	}

	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		let channel = match args.get("channel").and_then(|v| v.as_str()) {
			None => return ToolError::Missing { field: "channel" }.to_string(),
			Some(s) => match ChannelId::from_str(s) {
				Ok(id) => id,
				Err(_) => {
					return ToolError::Rejected(format!(
						"`{s}` is not an open Channel."
					))
					.to_string();
				},
			},
		};
		let text = match args.get("text").and_then(|v| v.as_str()) {
			None => return ToolError::Missing { field: "text" }.to_string(),
			Some(t) => t,
		};

		let open = ctx.harness.open_channels();
		if !open.iter().any(|(id, _)| *id == channel) {
			return ToolError::Rejected(format!("{channel} is not open."))
				.to_string();
		}

		ctx.harness
			.receive(channel, text, IncomingFrom::Swarm)
			.await;
		format!("Sent to {channel}.")
	}
}
