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

use super::{Tool, ToolError};

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

	/// Reads `unresponsive_engines` off the response, so a rate limit can be
	/// told apart from an empty web — the two look identical in the results and
	/// mean opposite things to whoever reads them.
	async fn call(&self, ctx: &SessionCtx, args: serde_json::Value) -> String {
		let query = match args.get("query").and_then(|v| v.as_str()) {
			None => return ToolError::Missing { field: "query" }.to_string(),
			Some(q) => q,
		};

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

		let status = response.status();
		let text = match response.text().await {
			Ok(t) => t,
			Err(e) => {
				return format!(
					"Error: could not read the search engine's answer: {e}"
				);
			},
		};
		if !status.is_success() {
			return format!(
				"Error: the search engine answered HTTP {status}: {text}"
			);
		}

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

	/// http(s) only, and scripts are never run: the page is stripped to words.
	async fn call(&self, _ctx: &SessionCtx, args: serde_json::Value) -> String {
		let url = match args.get("url").and_then(|v| v.as_str()) {
			None => return ToolError::Missing { field: "url" }.to_string(),
			Some(u) => u,
		};
		if !(url.starts_with("http://") || url.starts_with("https://")) {
			return ToolError::Rejected(format!(
				"`{url}` is not an http(s) URL."
			))
			.to_string();
		}

		let response = match reqwest::Client::new().get(url).send().await {
			Ok(r) => r,
			Err(e) => return format!("Error: could not reach {url}: {e}"),
		};
		let status = response.status();
		let text = match response.text().await {
			Ok(t) => t,
			Err(e) => return format!("Error: could not read {url}: {e}"),
		};
		if !status.is_success() {
			return format!("Error: {url} answered HTTP {status}.");
		}

		html_to_text(&text)
	}
}

// --- The wire shape ----------------------------------------------------------

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

/// Titles, URLs and snippets, then a note about any engine that did not
/// answer — a rate limit and an empty web look identical in the results
/// alone.
fn render_results(response: &SearxResponse) -> String {
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

/// HTML to the words a model should read: markup, scripts and styles removed,
/// whitespace collapsed.
///
/// Hand-rolled rather than pulled in from a crate: this only has to be good
/// enough for a model to read, not a browser-grade parser.
pub fn html_to_text(html: &str) -> String {
	let lower = html.to_ascii_lowercase();
	let mut out = String::with_capacity(html.len());
	let mut i = 0;

	while i < html.len() {
		if html.as_bytes()[i] != b'<' {
			let ch = html[i..].chars().next().expect("i is a char boundary");
			out.push(ch);
			i += ch.len_utf8();
			continue;
		}

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

		match html[i..].find('>') {
			Some(gt) => i += gt + 1,
			None => i = html.len(),
		}
		out.push(' ');
	}

	let out = decode_entities(&out);
	out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The handful of HTML entities that actually show up in body text.
fn decode_entities(s: &str) -> String {
	s.replace("&nbsp;", " ")
		.replace("&amp;", "&")
		.replace("&lt;", "<")
		.replace("&gt;", ">")
		.replace("&quot;", "\"")
		.replace("&apos;", "'")
		.replace("&#39;", "'")
}
