//! The swarm's own history, searchable by meaning.
//!
//! Two things live here: turning text into a vector, and ranking a corpus
//! against a query. The `memory` Role's tools are a thin layer of formatting
//! over this file.
//!
//! Search is by meaning rather than by keyword, and by brute force over every
//! vector. That is the right shape: the corpus is the Lessons and the Tasks, and
//! an approximate index would be solving a problem this system does not have. It
//! stops being right somewhere in the tens of thousands of entries.
//!
//! **Indexing is lazy.** Nothing is embedded when a Task or a lesson is created —
//! that would put a network call on the path of `create_task`, which is
//! synchronous and should stay that way. The first search embeds what is not
//! cached in one batch, with the query riding along. A cached vector is never
//! stale, because nothing in the corpus is edited after it is written; the cache
//! is in the database now, so it survives a restart along with everything else.
//!
//! [`Embedder`] is a seam: a bench that wants a deterministic ranking supplies
//! its own rather than paying for one.
//!
//! Defines: [`Embedder`], [`OpenRouterEmbedder`], [`EmbedError`], [`cosine`],
//! [`rank`].

use async_trait::async_trait;

/// Long inputs cost more and embed worse — a vector of a whole page is a vector
/// of nothing in particular. Briefs and lessons are shorter than this in
/// practice; the cap is here so one runaway Brief cannot fail a whole batch.
pub const MAX_INPUT_CHARS: usize = 6_000;

pub const EMBED_MODEL: &str = "liquid/lfm-2.5-embedding-350m:free";
pub const EMBED_ENDPOINT: &str = "https://openrouter.ai/api/v1/embeddings";

/// Text to vectors.
#[async_trait]
pub trait Embedder: Send + Sync {
	/// The model these vectors come from. Cached vectors are keyed on it, so
	/// changing the model does not silently mix two vector spaces.
	fn model(&self) -> &str;

	/// Embed a batch, order preserved.
	async fn embed(
		&self,
		texts: &[String],
	) -> Result<Vec<Vec<f32>>, EmbedError>;
}

/// The real embedder.
///
/// It deliberately does not go through the scheduler. A model call belongs to a
/// Session, carries a conversation, and waits in a queue so a human can follow
/// the run line by line. An embedding is none of that, and putting it in that
/// queue would show it in the UI as work being done and hold it behind whatever
/// the swarm is currently saying.
///
/// The cost of that choice is honest: an embedding is not a model call, so what
/// it spends never reaches Spend. See TASKS.md.
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
	/// Read the endpoint, key and model off the constants above and
	/// [`crate::model::API_KEY`]. The same prototype key `OpenRouter` uses —
	/// configurability comes later.
	pub fn from_env() -> Self {
		Self::new(EMBED_ENDPOINT, crate::model::API_KEY, EMBED_MODEL)
	}

	pub fn new(endpoint: &str, api_key: &str, model: &str) -> Self {
		OpenRouterEmbedder {
			client: reqwest::Client::new(),
			endpoint: endpoint.to_string(),
			api_key: api_key.to_string(),
			model: model.to_string(),
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
		let body = EmbedRequest { model: &self.model, input: texts };

		let response = self
			.client
			.post(&self.endpoint)
			.bearer_auth(&self.api_key)
			.json(&body)
			.send()
			.await
			.map_err(|e| EmbedError::Transport(e.to_string()))?;

		let status = response.status();
		let text = response
			.text()
			.await
			.map_err(|e| EmbedError::Transport(e.to_string()))?;

		if !status.is_success() {
			return Err(EmbedError::Status {
				status: status.as_u16(),
				body: text,
			});
		}

		let mut parsed: EmbedResponse = serde_json::from_str(&text)
			.map_err(|e| EmbedError::Malformed(e.to_string()))?;
		parsed.data.sort_by_key(|d| d.index);
		Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
	}
}

// --- The wire shape ---------------------------------------------------------

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

/// Cut text down to [`MAX_INPUT_CHARS`], so one runaway Brief cannot fail a
/// whole batch.
fn truncated(text: &str) -> String {
	text.chars().take(MAX_INPUT_CHARS).collect()
}

/// Rank a corpus against a query, best first.
///
/// Embeds whatever the Store has no vector for, caches what comes back, and
/// scores everything by cosine. The query rides in the same batch, so one search
/// is one call.
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

	let mut vectors: Vec<Option<Vec<f32>>> = Vec::with_capacity(corpus.len());
	let mut missing_idx = Vec::new();
	let mut batch = Vec::new();
	for (i, (key, text, _)) in corpus.iter().enumerate() {
		match vector_of(key)? {
			Some(v) => vectors.push(Some(v)),
			None => {
				vectors.push(None);
				missing_idx.push(i);
				batch.push(truncated(text));
			},
		}
	}
	batch.push(truncated(query));

	let mut embedded = embedder.embed(&batch).await?;
	let query_vector = embedded.pop().expect("the query was just appended");

	for (idx, vector) in missing_idx.into_iter().zip(embedded) {
		let key = &corpus[idx].0;
		store
			.put_vector(key, model, &vector)
			.map_err(|e| EmbedError::Malformed(format!("database: {e}")))?;
		vectors[idx] = Some(vector);
	}

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

/// What a tool says when a search could not be made.
///
/// A sentence the model can act on, not a stack trace: the tool that called this
/// has to answer its Session either way.
pub fn search_failed(what: &str, err: &EmbedError) -> String {
	format!("Could not search {what}: {err}")
}
