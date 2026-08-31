//! What metacognition kept.
//!
//! A `<lessons>` section outlives the Session judged. Nothing reads it back
//! automatically; it is found later by meaning via the `memory` Role.
//!
//! Construct: `reflect::keep_lessons` parses `<lessons>` and builds
//! `NewLesson { text, session, about }`; `Store::keep_lesson` mints
//! `LessonId`, persists the row, and emits `Event::LessonKept`. Empty sections
//! write nothing.
//!
//! Use: `memory::lesson_corpus` + `memory::rank` — cosine, brute force —
//! behind `tools::recall::SearchLessons` and `web::server`'s search box.
//! `Hit<T>` carries `score`. Indexing is lazy: first search embeds uncached
//! rows in one batch via `Store::vector` / `put_vector`.
//!
//! Consumers:
//! - `reflect.rs` — produces `NewLesson` from `SessionKind`
//! - `store.rs` / `db/rows.rs` — persists, emits `LessonKept` → `web/wire`, `log`
//! - `memory.rs` — `lesson_corpus` + `rank`, shared by tool and Watcher
//! - `tools/recall.rs` — `render_lesson_hits` via `LessonSubject::describe`
//!
//! Rules:
//! - **Write-once, never edited** — cached vectors never stale.
//! - **Only `text` is searched** — `LessonSubject` is placement, not corpus.
//! - **Search reaches every Run** — `run` recorded but never a filter.
//! - **Lessons never re-enter the judged Session.**
//!
//! | `LessonSubject` | when | carries |
//! | --- | --- | --- |
//! | `Task` | Worker Session | `TaskId` + `RoleName` + `Title` |
//! | `Conversation` | Comms Session (no Task) | `ChannelId` |
//!
//! Defines: [`Lesson`], [`LessonSubject`], [`NewLesson`], [`Hit`].

use super::ids::{ChannelId, LessonId, RunId, SessionId, TaskId};
use super::text::{Day, Title};
use super::time::Timestamp;
use crate::roles::RoleName;

/// One thing metacognition thought was worth keeping.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Lesson {
	pub id: LessonId,
	/// The Run it was written in. Recorded, but not a filter: a search reaches
	/// every Run, which is the point of keeping them.
	pub run: RunId,
	/// The bullets the metacognition wrote. This, and only this, is searched.
	pub text: String,
	pub day: Day,
	/// The Session that was judged — how a hit leads back to the conversation.
	pub session: SessionId,
	pub about: LessonSubject,
	pub at: Timestamp,
}

/// What a lesson is about, so a search hit can be placed without a second lookup.
#[derive(
	Debug,
	Clone,
	PartialEq,
	Eq,
	serde::Serialize,
	serde::Deserialize,
	strum::Display,
	strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum LessonSubject {
	/// From a Worker Session: the work it was doing.
	Task { task: TaskId, role: RoleName, title: Title },
	/// From a Comms Session, which has no Task and no Role. What it is about is
	/// a conversation, and its Channel is where that happened.
	Conversation { channel: ChannelId },
}

/// Everything needed to keep a lesson. The Store mints the id.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NewLesson {
	pub text: String,
	pub session: SessionId,
	pub about: LessonSubject,
}

/// One result of a search by meaning, and how close it was.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Hit<T> {
	pub item: T,
	pub score: f32,
}

impl LessonSubject {
	/// One line naming what this lesson is about, for a search hit.
	pub fn describe(&self) -> String {
		match self {
			LessonSubject::Task { task, role, title } => {
				format!("{title} ({role}, {task})")
			},
			LessonSubject::Conversation { channel } => {
				format!("conversation on {channel}")
			},
		}
	}
}
