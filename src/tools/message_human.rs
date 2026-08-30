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

use async_trait::async_trait;

use crate::domain::ToolSchema;
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;

use super::Tool;

/// Inject a message into the Comms Session on a named Channel.
pub struct MessageHuman;

#[async_trait]
impl Tool for MessageHuman {
    fn name(&self) -> ToolName {
        ToolName::MessageHuman
    }

    /// The `channel` argument is an enum of the open Channels, each labelled
    /// with its kind, so the model names one that exists.
    fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
        unimplemented!()
    }

    async fn call(&self, _ctx: &SessionCtx, _args: serde_json::Value) -> String {
        unimplemented!()
    }
}
