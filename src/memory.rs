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
	pub fn from_env() -> Self {
		unimplemented!()
	}
}

#[async_trait]
impl Embedder for OpenRouterEmbedder {
	fn model(&self) -> &str {
		unimplemented!()
	}

	async fn embed(
		&self,
		_texts: &[String],
	) -> Result<Vec<Vec<f32>>, EmbedError> {
		unimplemented!()
	}
}

/// Closeness of two vectors, from -1 to 1.
pub fn cosine(_a: &[f32], _b: &[f32]) -> f32 {
	unimplemented!()
}

/// Rank a corpus against a query, best first.
///
/// Embeds whatever the Store has no vector for, caches what comes back, and
/// scores everything by cosine. The query rides in the same batch, so one search
/// is one call.
pub async fn rank<T: Clone>(
	_store: &crate::store::Store,
	_embedder: &dyn Embedder,
	_query: &str,
	_corpus: &[(String, String, T)],
	_count: usize,
) -> Result<Vec<crate::domain::Hit<T>>, EmbedError> {
	unimplemented!()
}

/// What a tool says when a search could not be made.
///
/// A sentence the model can act on, not a stack trace: the tool that called this
/// has to answer its Session either way.
pub fn search_failed(_what: &str, _err: &EmbedError) -> String {
	unimplemented!()
}
