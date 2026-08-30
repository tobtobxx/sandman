//! Holding for another Session's answer.
//!
//! The tool that makes "nothing waits" survivable. A Worker that needs a child's
//! answer calls this and its turn suspends inside the call — no context is torn
//! down, nothing is registered on the Task, and when the answer exists it comes
//! back as this call's result. The Worker carries on remembering why it asked.
//!
//! A Task already finished resolves at once. A Task that is cancelled resolves
//! with the notice that stands in for a Result, so nobody hangs on work that
//! will never produce one.
//!
//! Defines: [`AwaitResult`].

use async_trait::async_trait;

use crate::domain::ToolSchema;
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;

use super::Tool;

/// Block this turn until a Task completes, then return its answer.
pub struct AwaitResult;

#[async_trait]
impl Tool for AwaitResult {
    fn name(&self) -> ToolName {
        ToolName::AwaitResult
    }

    fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
        unimplemented!()
    }

    /// Reads the Task id, then hands off to [`crate::waiters::Waiters::wait`].
    ///
    /// Any Task may be awaited by id, not only one this Session created.
    async fn call(&self, _ctx: &SessionCtx, _args: serde_json::Value) -> String {
        unimplemented!()
    }
}
