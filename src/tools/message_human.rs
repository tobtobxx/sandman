//! Push path: swarm-initiated speech to a human.
//!
//! The only tool that reaches a human without a Task. A Worker cannot address
//! a Comms Session with a Task — every Task becomes a Worker — so a planning
//! Worker that decides a human must know something calls this; the text lands
//! as `IncomingFrom::Swarm` mail on the Channel it names, where that Session
//! decides verbatim vs reworded.
//!
//! Construct: `MessageHuman` implements [`crate::tools::Tool`], created in
//! `Registry::all` with no state; `schema` takes `SchemaCtx { open_channels }`.
//! Use: `call(ctx, {channel?, text}) -> String` parses `ChannelId` — or takes
//! the favorite when `channel` is absent — validates open, then
//! `harness.receive(channel, text, Swarm).await` → `Sent to …`.
//! Consumers: `session::turn` via `ToolRunner::run` (only `RoleName::Planning`
//! holds it — see `roles::tools_for`); `Harness::drive_comms` → `comms::respond`
//! drains the mail and `say`s it through `Channel::send`.
//!
//! Call trace: `turn → tools.run(message_human) → harness.receive → store.receive_mail`
//! `→ drive_comms → respond → say → Channel.send`.
//!
//! Rules: **only Planning has this tool — other Roles cannot push.**
//! **channel enum is per-Session, built from open Channels, never global.**
//! **`channels.favorite` takes what names no Channel — a kind, resolved when called; turned off, the message is dropped.**
//! **Comms decides wording; this tool only injects mail.**
//! **Brief carries no origin, so multi-Channel choice is a guess — see TASKS.md.**
//! **failure returns a sentence for the model, never an `Err`.**
//!
//! Defines: [`MessageHuman`].

use std::str::FromStr;

use async_trait::async_trait;

use crate::domain::{ChannelId, IncomingFrom, ToolSchema};
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;

use super::{Tool, ToolError};

/// Answer when the favorite Channel is turned off and the message goes nowhere.
const DROPPED: &str = "The favorite Channel is not open. Nothing was sent.";

/// Push text into the Comms mailbox on a named Channel.
pub struct MessageHuman;

#[async_trait]
impl Tool for MessageHuman {
	fn name(&self) -> ToolName {
		ToolName::MessageHuman
	}

	/// Build per-Session schema; `channel` is the enum of open Channels.
	///
	/// `channel` is optional — absent, the favorite Channel takes the message.
	fn schema(&self, ctx: &SchemaCtx) -> ToolSchema {
		// Collect open ids
		let ids: Vec<String> = ctx
			.open_channels
			.iter()
			.map(|(id, _)| id.to_string())
			.collect();
		// Build display listing
		let listing: Vec<String> = ctx
			.open_channels
			.iter()
			.map(|(id, kind)| format!("{id} ({kind})"))
			.collect();
		// Build schema
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
							"Which Channel to speak on. Open now: {}. \
							 Leave it out for the favorite Channel.",
							listing.join(", ")
						),
					},
					"text": {
						"type": "string",
						"description": "What to tell the human.",
					},
				},
				"required": ["text"],
			}),
		}
	}

	/// Parse `channel` and `text`, validate the Channel is open, then enqueue as swarm mail.
	///
	/// An absent `channel` is the favorite Channel. A favorite that is turned
	/// off drops the message — the answer is the only trace it leaves.
	/// Returns `Sent to …` or a sentence. Never fails as `Err`.
	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		// Parse channel
		let channel = match args.get("channel").and_then(|v| v.as_str()) {
			None => match ctx.harness.favorite_channel() {
				Some(id) => id,
				None => return DROPPED.to_string(),
			},
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
		// Parse text
		let text = match args.get("text").and_then(|v| v.as_str()) {
			None => return ToolError::Missing { field: "text" }.to_string(),
			Some(t) => t,
		};

		// Validate open
		let open = ctx.harness.open_channels();
		if !open.iter().any(|(id, _)| *id == channel) {
			return ToolError::Rejected(format!("{channel} is not open."))
				.to_string();
		}

		// Enqueue mail
		ctx.harness
			.receive(channel, text, IncomingFrom::Swarm)
			.await;
		format!("Sent to {channel}.")
	}
}
