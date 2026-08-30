//! Rows to domain values, and back.
//!
//! The one place that knows how a [`crate::domain`] type is spelled in SQLite.
//! Everything else — the Store, the tools, the Watcher — works in domain types
//! and never sees a column name.
//!
//! Sum types cross this boundary as a discriminant plus a JSON payload. The
//! discriminant is what queries filter on; the payload is what the variant
//! carries. Both are written here, together, so the pair cannot disagree.
//!
//! Defines: read and write helpers for every persisted entity.

use rusqlite::{Row, Transaction};

use super::DbError;
use crate::domain::{
	CallStatus, ChannelRecord, Incoming, LessonSubject, LlmCall, Message,
	Reflection, ReflectionResult, Run, Schedule, Session, SessionId, Task,
	TaskState, Utterance,
};

/// A sum type as it is stored: the variant's name, and what it carries.
///
/// Split rather than a single tagged JSON blob because the name is a column an
/// index can use.
pub struct Tagged {
	pub tag: &'static str,
	pub json: String,
}

pub fn task_state_to_row(_state: &TaskState) -> Result<Tagged, DbError> {
	unimplemented!()
}

pub fn task_state_from_row(
	_tag: &str,
	_json: &str,
) -> Result<TaskState, DbError> {
	unimplemented!()
}

pub fn schedule_to_row(_schedule: &Schedule) -> Result<Tagged, DbError> {
	unimplemented!()
}

pub fn schedule_from_row(_tag: &str, _json: &str) -> Result<Schedule, DbError> {
	unimplemented!()
}

pub fn call_status_to_row(_status: &CallStatus) -> Result<Tagged, DbError> {
	unimplemented!()
}

pub fn call_status_from_row(
	_tag: &str,
	_json: &str,
) -> Result<CallStatus, DbError> {
	unimplemented!()
}

pub fn reflection_result_to_row(
	_r: &ReflectionResult,
) -> Result<Tagged, DbError> {
	unimplemented!()
}

pub fn reflection_result_from_row(
	_tag: &str,
	_json: &str,
) -> Result<ReflectionResult, DbError> {
	unimplemented!()
}

pub fn lesson_subject_to_row(_s: &LessonSubject) -> Result<Tagged, DbError> {
	unimplemented!()
}

pub fn lesson_subject_from_row(
	_tag: &str,
	_json: &str,
) -> Result<LessonSubject, DbError> {
	unimplemented!()
}

// --- Whole entities --------------------------------------------------------

pub fn read_run(_row: &Row<'_>) -> Result<Run, DbError> {
	unimplemented!()
}

pub fn read_task(_row: &Row<'_>) -> Result<Task, DbError> {
	unimplemented!()
}

/// A Session without its messages, reflections or mail. Those are separate
/// tables; [`load_session`] joins them.
pub fn read_session_head(_row: &Row<'_>) -> Result<Session, DbError> {
	unimplemented!()
}

/// A whole Session: its head, then its messages, reflections and unread mail in
/// order.
pub fn load_session(
	_tx: &Transaction<'_>,
	_id: SessionId,
) -> Result<Option<Session>, DbError> {
	unimplemented!()
}

pub fn read_call(_row: &Row<'_>) -> Result<LlmCall, DbError> {
	unimplemented!()
}

pub fn read_channel(_row: &Row<'_>) -> Result<ChannelRecord, DbError> {
	unimplemented!()
}

pub fn read_message(_row: &Row<'_>) -> Result<Message, DbError> {
	unimplemented!()
}

pub fn read_reflection(_row: &Row<'_>) -> Result<Reflection, DbError> {
	unimplemented!()
}

pub fn read_incoming(_row: &Row<'_>) -> Result<Incoming, DbError> {
	unimplemented!()
}

pub fn read_utterance(_row: &Row<'_>) -> Result<Utterance, DbError> {
	unimplemented!()
}
