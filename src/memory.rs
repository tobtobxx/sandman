//! The swarm's memory — text to vectors and ranking by meaning.
//!
//! Brute-force cosine over Lessons and Tasks. Right for tens of thousands;
//! wrong beyond and never indexed approximately.
//! **Indexing is lazy** — nothing embedded at creation; first `rank` embeds
//! uncached rows in one batch with the query riding along. A cached vector is
//! never stale: corpus is write-once and keyed by `Embedder::model` so spaces
//! cannot mix.
//!
//! Construct: `OpenRouterEmbedder::from_spec(&Embedding)` from
//! `Config::embedding`; bench supplies its own `Embedder`.
//! Use: `lesson_corpus(lessons) -> Vec<(key, text, Lesson)>` plus
//! `rank(store, embedder, query, corpus, n) -> Vec<Hit<T>>` best first;
//! `cosine(a, b)` in -1..1.
//! Consumers: `tools/recall::{SearchLessons, SearchTasks}` and
//! `web::server::on_search` share `rank` so Worker and Watcher see same scores.
//!
//! Seam: `Embedder` — text in, vectors out, model-qualified cache in
//! `store::vector` / `put_vector`.
//!
//! | Trait | Real | Bench |
//! | --- | --- | --- |
//! | `Embedder` | `OpenRouterEmbedder` (skips scheduler, never Spend) | deterministic stub |
//!
//! Rules: **embedder never goes through scheduler** — no queue, no Spend, no
//! Session turn. **brute force, no index** — `rank` scans every vector.
//!
//! Defines: [`Embedder`], [`OpenRouterEmbedder`], [`EmbedError`], [`cosine`],
//! [`rank`], [`lesson_corpus`], [`search_failed`].

use async_trait::async_trait;

use crate::config::Embedding;

/// Text to vectors.
#[async_trait]
pub trait Embedder: Send + Sync {
	/// Model that produced these vectors — cache key so spaces never mix.
	fn model(&self) -> &str;

	/// Embed a batch, order preserved.
	async fn embed(
		&self,
		texts: &[String],
	) -> Result<Vec<Vec<f32>>, EmbedError>;
}

/// Real embedder — calls the embedding service directly.
///
/// Bypasses the scheduler; not a Session turn and never counted as Spend.
pub struct OpenRouterEmbedder {
	client: reqwest::Client,
	endpoint: String,
	api_key: String,
	model: String,
}

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
	#[error("could not reach the embedding service: {0}")]
	Transport(String),
	#[error("HTTP {status}: {body}")]
	Status { status: u16, body: String },
	#[error("the embedding service answered with something else: {0}")]
	Malformed(String),
}

impl OpenRouterEmbedder {
	/// Build from `Config::embedding`.
	pub fn from_spec(spec: &Embedding) -> Self {
		OpenRouterEmbedder {
			client: reqwest::Client::new(),
			endpoint: spec.endpoint.clone(),
			api_key: spec.api_key.clone(),
			model: spec.model.clone(),
		}
	}
}

#[async_trait]
impl Embedder for OpenRouterEmbedder {
	fn model(&self) -> &str {
		&self.model
	}

	async fn embed(
		&self,
		texts: &[String],
	) -> Result<Vec<Vec<f32>>, EmbedError> {
		// Build request
		let body = EmbedRequest { model: &self.model, input: texts };

		// Send request
		let response = self
			.client
			.post(&self.endpoint)
			.bearer_auth(&self.api_key)
			.json(&body)
			.send()
			.await
			.map_err(|e| EmbedError::Transport(e.to_string()))?;

		let status = response.status();

		// Read body
		let text = response
			.text()
			.await
			.map_err(|e| EmbedError::Transport(e.to_string()))?;

		// Check status
		if !status.is_success() {
			return Err(EmbedError::Status {
				status: status.as_u16(),
				body: text,
			});
		}

		// Parse response
		let mut parsed: EmbedResponse = serde_json::from_str(&text)
			.map_err(|e| EmbedError::Malformed(e.to_string()))?;
		parsed.data.sort_by_key(|d| d.index);
		Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
	}
}

// Wire shapes

#[derive(serde::Serialize)]
struct EmbedRequest<'a> {
	model: &'a str,
	input: &'a [String],
}

#[derive(serde::Deserialize)]
struct EmbedResponse {
	data: Vec<EmbedDatum>,
}

#[derive(serde::Deserialize)]
struct EmbedDatum {
	embedding: Vec<f32>,
	index: usize,
}

/// Closeness of two vectors, from -1 to 1.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
	let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
	let norm_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
	let norm_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
	if norm_a == 0.0 || norm_b == 0.0 {
		0.0
	} else {
		dot / (norm_a * norm_b)
	}
}

/// Build a Lesson corpus for [`rank`].
///
/// Keyed by `lesson/{id}`, searched by `text`. Shared by tool and Watcher.
pub fn lesson_corpus(
	lessons: Vec<crate::domain::Lesson>,
) -> Vec<(String, String, crate::domain::Lesson)> {
	lessons
		.into_iter()
		.map(|l| (format!("lesson/{}", l.id), l.text.clone(), l))
		.collect()
}

/// Rank a corpus against a query by cosine, best first.
///
/// Lazily embeds uncached entries in one batch; caches and scores all.
/// Query rides in same batch — one search, one call.
pub async fn rank<T: Clone>(
	store: &crate::store::Store,
	embedder: &dyn Embedder,
	query: &str,
	corpus: &[(String, String, T)],
	count: usize,
) -> Result<Vec<crate::domain::Hit<T>>, EmbedError> {
	let model = embedder.model();
	let vector_of = |key: &str| -> Result<Option<Vec<f32>>, EmbedError> {
		store
			.vector(key, model)
			.map_err(|e| EmbedError::Malformed(format!("database: {e}")))
	};

	// Check cache
	let mut vectors: Vec<Option<Vec<f32>>> = Vec::with_capacity(corpus.len());
	let mut missing_idx = Vec::new();
	let mut batch = Vec::new();
	for (i, (key, text, _)) in corpus.iter().enumerate() {
		match vector_of(key)? {
			Some(v) => vectors.push(Some(v)),
			None => {
				vectors.push(None);
				missing_idx.push(i);
				batch.push(text.clone());
			},
		}
	}
	batch.push(query.to_string());

	// Embed missing plus query
	let mut embedded = embedder.embed(&batch).await?;
	let query_vector = embedded.pop().expect("the query was just appended");

	// Cache new vectors
	for (idx, vector) in missing_idx.into_iter().zip(embedded) {
		let key = &corpus[idx].0;
		store
			.put_vector(key, model, &vector)
			.map_err(|e| EmbedError::Malformed(format!("database: {e}")))?;
		vectors[idx] = Some(vector);
	}

	// Score and rank
	let mut hits: Vec<crate::domain::Hit<T>> = corpus
		.iter()
		.zip(vectors)
		.map(|((_, _, item), vector)| crate::domain::Hit {
			item: item.clone(),
			score: cosine(
				&query_vector,
				vector
					.as_deref()
					.expect("every corpus item has a vector by now"),
			),
		})
		.collect();

	hits.sort_by(|a, b| {
		b.score
			.partial_cmp(&a.score)
			.unwrap_or(std::cmp::Ordering::Equal)
	});
	hits.truncate(count);
	Ok(hits)
}

/// Tool-visible error when a search could not be made.
pub fn search_failed(what: &str, err: &EmbedError) -> String {
	format!("Could not search {what}: {err}")
}
