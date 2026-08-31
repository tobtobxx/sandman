//! Reaching the outside web. The `research` Role's two tools.
//!
//! Construct: [`WebSearch`] and [`WebFetch`] are stateless [`Tool`]s built in
//! `Registry::all` with no state; [`html_to_text`] is a pure `&str -> String`
//! with no crate dependency.
//! Use: `Tool::call(ctx, args) -> String` — `WebSearch` queries SearXNG at
//! `config.tools.searxng_endpoint` and renders hits; `WebFetch` GETs one
//! `http(s)` URL and strips it via `html_to_text`.
//! Consumers: `roles.rs` assigns both to `Research` only; `Registry`
//! dispatches via [`ToolRunner`], bench wraps it to script answers without
//! touching the network.
//!
//! | Tool | Input | Effect |
//! | --- | --- | --- |
//! | [`WebSearch`] | `query` | GET `searxng_endpoint?q=&format=json` → titles/URLs/snippets + `unresponsive_engines` note |
//! | [`WebFetch`] | `url` (http(s) only) | GET URL → [`html_to_text`] (tags/scripts/styles removed, entities decoded) |
//!
//! Rules: **stateless — answers enter the Session transcript and nowhere else.**
//! **no HTTP seam — intercepted at [`ToolRunner`] like any other tool.**
//! **answers in words, always — network/HTTP/parse failures are sentences.**
//! **http(s) only — other schemes rejected.** **scripts never run —
//! [`html_to_text`] strips rather than executes.** **rate limit told apart
//! from empty — `unresponsive_engines` surfaces in render.**
//!
//! Defines: [`WebSearch`], [`WebFetch`], [`html_to_text`].

use async_trait::async_trait;

use crate::domain::ToolSchema;
use crate::roles::{SchemaCtx, ToolName};
use crate::session::SessionCtx;

use super::{Tool, ToolError};

/// Search the web via SearXNG. Returns titles, URLs and snippets.
pub struct WebSearch;

/// Fetch one http(s) page as readable text. Strips markup, scripts and styles.
pub struct WebFetch;

#[async_trait]
impl Tool for WebSearch {
	fn name(&self) -> ToolName {
		ToolName::WebSearch
	}

	fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
		ToolSchema {
			name: self.name().to_string(),
			description: "Search the web. Returns titles, URLs and snippets. \
			              Has a very low rate limit — search once, maybe \
			              twice with different wording, and no more."
				.to_string(),
			parameters: serde_json::json!({
				"type": "object",
				"properties": {
					"query": {
						"type": "string",
						"description": "What to search for.",
					},
				},
				"required": ["query"],
			}),
		}
	}

	/// Search via SearXNG and render results.
	///
	/// Reads `query`; returns titles/URLs/snippets or a sentence on failure.
	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		// Validate query
		let query = match args.get("query").and_then(|v| v.as_str()) {
			None => return ToolError::Missing { field: "query" }.to_string(),
			Some(q) => q,
		};

		// Send search request
		let response = match reqwest::Client::new()
			.get(&ctx.harness.config.tools.searxng_endpoint)
			.query(&[("q", query), ("format", "json")])
			.send()
			.await
		{
			Ok(r) => r,
			Err(e) => {
				return format!(
					"Error: could not reach the search engine: {e}"
				);
			},
		};

		// Read response body
		let status = response.status();
		let text = match response.text().await {
			Ok(t) => t,
			Err(e) => {
				return format!(
					"Error: could not read the search engine's answer: {e}"
				);
			},
		};
		// Check HTTP status
		if !status.is_success() {
			return format!(
				"Error: the search engine answered HTTP {status}: {text}"
			);
		}

		// Parse and render
		let parsed: SearxResponse = match serde_json::from_str(&text) {
			Ok(p) => p,
			Err(e) => {
				return format!(
					"Error: could not read the search engine's answer: {e}"
				);
			},
		};
		render_results(&parsed)
	}
}

#[async_trait]
impl Tool for WebFetch {
	fn name(&self) -> ToolName {
		ToolName::WebFetch
	}

	fn schema(&self, _ctx: &SchemaCtx) -> ToolSchema {
		ToolSchema {
			name: self.name().to_string(),
			description: "Fetch one web page over http(s) and return its \
			              readable text."
				.to_string(),
			parameters: serde_json::json!({
				"type": "object",
				"properties": {
					"url": {
						"type": "string",
						"description": "The page's http(s) URL.",
					},
				},
				"required": ["url"],
			}),
		}
	}

	/// Fetch one URL and strip it to readable text.
	///
	/// Reads `url`; http(s) only; returns words or a sentence on failure.
	async fn call(&self, _ctx: &SessionCtx, args: serde_json::Value) -> String {
		// Validate URL
		let url = match args.get("url").and_then(|v| v.as_str()) {
			None => return ToolError::Missing { field: "url" }.to_string(),
			Some(u) => u,
		};
		// Check scheme
		if !(url.starts_with("http://") || url.starts_with("https://")) {
			return ToolError::Rejected(format!(
				"`{url}` is not an http(s) URL."
			))
			.to_string();
		}

		// Send request
		let response = match reqwest::Client::new().get(url).send().await {
			Ok(r) => r,
			Err(e) => return format!("Error: could not reach {url}: {e}"),
		};
		// Read response body
		let status = response.status();
		let text = match response.text().await {
			Ok(t) => t,
			Err(e) => return format!("Error: could not read {url}: {e}"),
		};
		// Check HTTP status
		if !status.is_success() {
			return format!("Error: {url} answered HTTP {status}.");
		}

		// Strip to text
		html_to_text(&text)
	}
}

// Wire shape

#[derive(serde::Deserialize)]
struct SearxResponse {
	#[serde(default)]
	results: Vec<SearxResult>,
	#[serde(default)]
	unresponsive_engines: Vec<(String, String)>,
}

#[derive(serde::Deserialize)]
struct SearxResult {
	title: String,
	url: String,
	#[serde(default)]
	content: String,
}

/// Render SearXNG response to the model's next read.
///
/// Joins titles/URLs/snippets; appends note for `unresponsive_engines`.
fn render_results(response: &SearxResponse) -> String {
	// Render hits
	let mut out = if response.results.is_empty() {
		"No results.".to_string()
	} else {
		response
			.results
			.iter()
			.map(|r| format!("{}\n{}\n{}", r.title, r.url, r.content))
			.collect::<Vec<_>>()
			.join("\n\n")
	};

	// Append rate-limit note
	if !response.unresponsive_engines.is_empty() {
		let engines: Vec<String> = response
			.unresponsive_engines
			.iter()
			.map(|(engine, reason)| format!("{engine} ({reason})"))
			.collect();
		out.push_str(&format!(
			"\n\nNote: these engines did not answer (rate limit): {}.",
			engines.join(", ")
		));
	}
	out
}

/// Strip HTML to readable text.
///
/// Removes tags/scripts/styles, collapses whitespace, decodes entities.
pub fn html_to_text(html: &str) -> String {
	// Prepare lowercase copy
	let lower = html.to_ascii_lowercase();
	let mut out = String::with_capacity(html.len());
	let mut i = 0;

	// Walk HTML
	while i < html.len() {
		// Copy text outside tags
		if html.as_bytes()[i] != b'<' {
			let ch = html[i..].chars().next().expect("i is a char boundary");
			out.push(ch);
			i += ch.len_utf8();
			continue;
		}

		// Skip script and style blocks
		let skip_tag = ["<script", "<style"]
			.iter()
			.find(|tag| lower[i..].starts_with(**tag));
		if let Some(tag) = skip_tag {
			let close = if tag.contains("script") {
				"</script"
			} else {
				"</style"
			};
			match lower[i..].find(close) {
				Some(rel) => {
					let after = i + rel;
					i = match html[after..].find('>') {
						Some(gt) => after + gt + 1,
						None => html.len(),
					};
				},
				None => i = html.len(),
			}
			continue;
		}

		// Skip tag
		match html[i..].find('>') {
			Some(gt) => i += gt + 1,
			None => i = html.len(),
		}
		out.push(' ');
	}

	// Decode and collapse
	let out = decode_entities(&out);
	out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Decode the handful of HTML entities that appear in body text.
fn decode_entities(s: &str) -> String {
	s.replace("&nbsp;", " ")
		.replace("&amp;", "&")
		.replace("&lt;", "<")
		.replace("&gt;", ">")
		.replace("&quot;", "\"")
		.replace("&apos;", "'")
		.replace("&#39;", "'")
}
