//! Watching, and answering, every tool call.
//!
//! This is the seam the bench is built on. A case takes one real Task, the real
//! system prompt for its Role, and the real model — and puts an [`Interceptor`]
//! where the tool registry would be. Every call the model makes is recorded, and
//! the case decides which of them actually happen.
//!
//! That is the only question a case asks: given this Brief, what does the model
//! reach for, with what arguments, and in what order? It is also what keeps a
//! case to one Session — letting the real tools run would drag a web search and
//! three more Workers into a test about one decision.
//!
//! Three modes, and a case usually mixes them: pass a tool through because its
//! effect is what is being asserted on, answer another from a closure because
//! its result is a fixture, deny a third because reaching for it at all is the
//! failure.
//!
//! Defines: [`Interceptor`], [`ToolsChoice`], [`RecordedToolCall`], [`Answer`].

use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::{SessionId, Timestamp, ToolCall, ToolSchema};
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;
use crate::tools::ToolRunner;

/// One tool call a Session made, and what it got back.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedToolCall {
    pub session: SessionId,
    pub name: ToolName,
    /// What the model sent, parsed. Unparseable arguments are recorded as they
    /// arrived — a model that sends broken JSON is a finding, not a hole in the
    /// record.
    pub args: serde_json::Value,
    pub output: String,
    pub at: Timestamp,
    /// Whether the real tool ran, or the case answered for it.
    pub real: bool,
}

/// What the case does about one call.
pub enum Answer {
    /// Let the real tool run.
    Real,
    /// Answer with this text and do nothing.
    Say(String),
    /// Refuse, in words the model can read.
    Deny(String),
}

/// How a Rig gets its tool calls answered.
///
/// There is no "run them all" variant. A case that let the whole registry run
/// would stop being one Session, and that is the one thing the bench does not
/// measure.
pub enum ToolsChoice {
    /// Every call recorded, and each one answered by the case.
    Intercept(Box<dyn Fn(&RecordedCall) -> Answer + Send + Sync>),
    /// Every call recorded and refused. The default, and what a case asserting
    /// that a Session got where it was going without any tool at all wants.
    Deny,
}

/// A call as the interceptor's closure sees it, before it has an answer.
#[derive(Debug, Clone)]
pub struct RecordedCall {
    pub session: SessionId,
    pub name: ToolName,
    pub args: serde_json::Value,
}

/// The real registry, wrapped.
pub struct Interceptor {
    inner: Arc<dyn ToolRunner>,
    choice: ToolsChoice,
    log: std::sync::Mutex<Vec<RecordedToolCall>>,
}

impl Interceptor {
    pub fn new(_inner: Arc<dyn ToolRunner>, _choice: ToolsChoice) -> Self {
        unimplemented!()
    }

    /// Every call so far, in order.
    pub fn calls(&self) -> Vec<RecordedToolCall> {
        unimplemented!()
    }

    /// The calls to one tool, in order. What most assertions want.
    pub fn calls_to(&self, _name: ToolName) -> Vec<RecordedToolCall> {
        unimplemented!()
    }

    /// The tools reached for at all, in the order they were first used.
    pub fn tools_used(&self) -> Vec<ToolName> {
        unimplemented!()
    }
}

#[async_trait]
impl ToolRunner for Interceptor {
    /// Unchanged: the model is offered exactly the schemas it would be offered
    /// in a real run, whatever the case intends to do about the calls. Changing
    /// them would change the thing being measured.
    fn schemas(&self, _names: &[ToolName], _ctx: &SchemaCtx) -> Vec<ToolSchema> {
        unimplemented!()
    }

    async fn run(&self, _ctx: &SessionCtx, _call: &ToolCall) -> String {
        unimplemented!()
    }
}
