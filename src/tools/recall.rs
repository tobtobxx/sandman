//! Finding out what this swarm already did. The `memory` Role's tools.
//!
//! The `memory` Role does no new work. It searches what the swarm has already
//! done: the Lessons metacognition kept, the Tasks that were asked and answered,
//! and the conversations behind them. Now that the state persists, those reach
//! back across every Run rather than only the one in progress.
//!
//! Searching is by meaning, not by keyword — see `memory.rs` for the ranking.
//! `search_tasks` deliberately ranks a Task by what it *asked for*, its Title
//! and Brief, never by its Result: a hit is found by the question, and the
//! answer is shown once it is a hit.
//!
//! Defines: [`SearchLessons`], [`SearchTasks`], [`ViewSession`], [`CurrentTime`].

use async_trait::async_trait;

use crate::domain::ToolSchema;
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;

use super::Tool;

/// How much of one conversation `view_session` will show. A whole Session can be
/// longer than the Session reading it can hold.
pub const VIEW_SESSION_CAP: usize = 40_000;

/// Rank the Lessons against a query by meaning.
pub struct SearchLessons;

/// Rank Tasks by what they asked for. Shows the Result on a hit.
pub struct SearchTasks;

/// One Session's whole conversation as text, metacognition included, capped.
pub struct ViewSession;

/// The current weekday, date and time, one line.
pub struct CurrentTime;

#[async_trait]
impl Tool for SearchLessons {
    fn name(&self) -> ToolName {
        ToolName::SearchLessons
    }

    fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
        unimplemented!()
    }

    /// Each hit names its day, what it was about, and the Session to open to
    /// read the whole conversation behind it.
    async fn call(&self, _ctx: &SessionCtx, _args: serde_json::Value) -> String {
        unimplemented!()
    }
}

#[async_trait]
impl Tool for SearchTasks {
    fn name(&self) -> ToolName {
        ToolName::SearchTasks
    }

    fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
        unimplemented!()
    }

    async fn call(&self, _ctx: &SessionCtx, _args: serde_json::Value) -> String {
        unimplemented!()
    }
}

#[async_trait]
impl Tool for ViewSession {
    fn name(&self) -> ToolName {
        ToolName::ViewSession
    }

    fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
        unimplemented!()
    }

    /// Takes a Task id too, and resolves it to the Session that did it — a hit
    /// from `search_tasks` names a Task, and asking the reader to translate that
    /// is asking it to guess.
    async fn call(&self, _ctx: &SessionCtx, _args: serde_json::Value) -> String {
        unimplemented!()
    }
}

#[async_trait]
impl Tool for CurrentTime {
    fn name(&self) -> ToolName {
        ToolName::CurrentTime
    }

    fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
        unimplemented!()
    }

    /// Reads the Clock, holds nothing.
    async fn call(&self, _ctx: &SessionCtx, _args: serde_json::Value) -> String {
        unimplemented!()
    }
}
