//! What the tools are, and what runs them.
//!
//! Tools are independent of Roles: `roles.rs` decides which Role holds which of
//! these, and more than one Role may hold the same tool.
//!
//! Two traits, and the difference matters. [`Tool`] is one capability —
//! its schema and what it does. [`ToolRunner`] is *how a Session's tool calls
//! get answered at all*, and it is the seam a bench replaces: wrapping the real
//! [`Registry`] in a recorder is how a unit bench watches every call a model
//! makes without changing a single prompt, and answering from a closure is how
//! it drives a model down a path without paying for the work behind it.
//!
//! A tool returns a `String` to the model, always — including when it failed.
//! An error a model can read is domain output, not a Rust error, and the same
//! reasoning applies here as to a Task's Result: a failure is something that
//! says so, not something missing.
//!
//! Schemas are built per Session rather than declared once, because
//! `message_human` must offer the Channels that are open now.
//!
//! Files: [`create_task`] the three enqueue tools; [`await_result`] holding for
//! an answer; [`message_human`] reaching a human; [`web`] searching and reading
//! the world; [`recall`] searching what the swarm already did; [`queue`] running
//! the queue.
//!
//! Defines: [`Tool`], [`ToolRunner`], [`Registry`], [`ToolError`].

pub mod await_result;
pub mod create_task;
pub mod message_human;
pub mod queue;
pub mod recall;
pub mod web;

use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::{ToolCall, ToolSchema};
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;

/// One capability a Session may hold.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> ToolName;

    /// How this tool describes itself to the model.
    ///
    /// Takes the world it is described against, because at least one tool —
    /// `message_human` — must name the Channels that actually exist.
    fn schema(&self, ctx: &SchemaCtx) -> ToolSchema;

    /// Do it, and answer the model in words.
    ///
    /// `args` is whatever the model sent, already parsed as JSON. Reading it
    /// wrongly is not a failure of the system: return a sentence saying what was
    /// wrong and the model tries again.
    async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String;
}

/// How a Session's tool calls get answered.
///
/// The seam. [`Registry`] is the real one; a bench substitutes a recorder, a
/// script, or a refusal.
#[async_trait]
pub trait ToolRunner: Send + Sync {
    /// The schemas for a set of tools, as they go to the model.
    fn schemas(&self, names: &[ToolName], ctx: &SchemaCtx) -> Vec<ToolSchema>;

    /// Answer one tool call. Never fails: everything the model needs to know
    /// comes back as the string it reads.
    async fn run(&self, ctx: &SessionCtx, call: &ToolCall) -> String;
}

/// The real runner: every tool, by name.
pub struct Registry {
    tools: Vec<Arc<dyn Tool>>,
    events: Arc<crate::event::Events>,
}

impl Registry {
    /// Every tool Sandman has.
    pub fn all(_events: Arc<crate::event::Events>) -> Self {
        unimplemented!()
    }
}

#[async_trait]
impl ToolRunner for Registry {
    fn schemas(&self, _names: &[ToolName], _ctx: &SchemaCtx) -> Vec<ToolSchema> {
        unimplemented!()
    }

    /// Parse the arguments, emit [`crate::event::Event::ToolCalled`], dispatch,
    /// emit [`crate::event::Event::ToolReturned`].
    ///
    /// A name that matches no tool, and arguments that are not JSON, both come
    /// back as a sentence the model can act on rather than as anything that
    /// stops the turn.
    async fn run(&self, _ctx: &SessionCtx, _call: &ToolCall) -> String {
        unimplemented!()
    }
}

/// Why a tool could not do what it was asked, worded for the model that asked.
///
/// Never returned as an `Err` past the runner: it becomes the string the model
/// reads. It exists as a type so the wording lives in one place.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("Error: your arguments were not valid JSON. Try again.")]
    BadJson,
    #[error("Error: `{field}` is required.")]
    Missing { field: &'static str },
    #[error("Error: {0}")]
    Rejected(String),
    #[error("Error: there is no tool called {0}.")]
    NoSuchTool(String),
    #[error("Error: there is no Task {0}.")]
    NoSuchTask(String),
    #[error("Error: `{given}` is not a Role. The Roles are:\n{catalogue}")]
    NoSuchRole { given: String, catalogue: String },
}
