//! Looking at the world. The `research` Role's two tools.
//!
//! Both are stateless and hold nothing: what they find goes into the Session's
//! context and nowhere else. Neither needs a seam of its own — they are [`Tool`]
//! implementations, so a bench that wants to answer them without touching the
//! network intercepts them at the [`super::ToolRunner`] like any other.
//!
//! Defines: [`WebSearch`], [`WebFetch`], [`html_to_text`].

use async_trait::async_trait;

use crate::domain::ToolSchema;
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;

use super::Tool;

/// The SearXNG instance searches go to.
pub const SEARX_ENDPOINT: &str = "https://searx.be/search";

/// Search the web. Returns titles, URLs and snippets.
pub struct WebSearch;

/// Fetch one page over http(s) and return its readable text.
pub struct WebFetch;

#[async_trait]
impl Tool for WebSearch {
    fn name(&self) -> ToolName {
        ToolName::WebSearch
    }

    fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
        unimplemented!()
    }

    /// Reads `unresponsive_engines` off the response, so a rate limit can be
    /// told apart from an empty web — the two look identical in the results and
    /// mean opposite things to whoever reads them.
    async fn call(&self, _ctx: &SessionCtx, _args: serde_json::Value) -> String {
        unimplemented!()
    }
}

#[async_trait]
impl Tool for WebFetch {
    fn name(&self) -> ToolName {
        ToolName::WebFetch
    }

    fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
        unimplemented!()
    }

    /// http(s) only, and scripts are never run: the page is stripped to words.
    async fn call(&self, _ctx: &SessionCtx, _args: serde_json::Value) -> String {
        unimplemented!()
    }
}

/// HTML to the words a model should read: markup, scripts and styles removed,
/// whitespace collapsed.
pub fn html_to_text(_html: &str) -> String {
    unimplemented!()
}
