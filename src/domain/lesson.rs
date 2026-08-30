//! The Lessons: what metacognition kept.
//!
//! Every review and every interrupt may end with a `<lessons>` section, and when
//! it does the Harness keeps it — what the Session struggled with, and what
//! whoever does that kind of work next would want to know.
//!
//! A lesson is anchored on the Session that was judged, because the Session is
//! always the way back to the whole conversation. What the lesson is *about*
//! varies: most come from a Task, but one from a Comms Session has no Task,
//! because a conversation with a human is not one. [`LessonSubject`] carries
//! that difference instead of leaving three fields optional.
//!
//! Nothing reads a lesson back automatically. It is written once, never edited,
//! and found later only by someone looking — which is the whole of what the
//! `memory` Role does. Now that the Lessons persist, they outlive the Run that
//! wrote them, and a search reaches every Run.
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
