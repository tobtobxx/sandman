//! Row <-> domain translation — the one spelling of domain types in SQLite.
//!
//! The Store speaks domain types; SQLite speaks columns. This module is the
//! only place that knows both spellings, so a tag and its payload can never
//! disagree.
//!
//! Build: nothing to build — free functions over `Row`, `Transaction` and
//! `Tagged`.
//! Use: `*_to_row` before a write, `*_from_row` / `read_*` after a read,
//! `load_session` for the four-table join.
//! Consumers: `store.rs` only — no other module imports `rows`. SQL outside
//! Store means Store is missing a word.
//!
//! Seam — variant down, storage across:
//!
//! | Kind | On disk | Helpers | Filter |
//! | TaskState, Schedule, CallStatus, ReflectionResult, LessonSubject, SessionStatus, Message | `tag TEXT` + `json TEXT` | `*_to_row` / `*_from_row` via `payload_of` / `from_tagged` | tag only, json is opaque |
//! | ChannelKind, ReflectionKind, IncomingFrom, Who | bare `TEXT` (`strum`) | `variant_from_str` | whole value |
//! | Tier | `INTEGER 1..=5` | `tier_from_i64` | whole value |
//! | `Vec<f32>` | `BLOB` little-endian `f32` | `vector_to_blob` / `vector_from_blob` | never |
//! | Whole rows | `SELECT *` | `read_run`, `read_task`, `read_session_head`, `load_session`, `read_call` … | `id` / `(session, idx)` |
//!
//! Rules:
//! - **Tag and payload are written together.** One `Tagged` per call so they cannot diverge.
//! - **Payload normalises serde external tagging.** Unit variant → `"null"`, else the single value under the variant key; rebuilt as `{tag: payload}` for `DeserializeOwned`.
//! - **Session is split.** Head is one row; `load_session` appends messages, reflections, calls and unread mail in order.
//! - **Vectors are opaque.** No caller interprets the blob; nothing outside this file reads `vectors.vector`.

use rusqlite::{Row, Transaction};
use strum::IntoDiscriminant;

use super::DbError;
use crate::domain::{
	Brief, CallId, CallStatus, ChannelId, ChannelKind, ChannelRecord, Day,
	Incoming, Lesson, LessonId, LessonSubject, LlmCall, Message, Reflection,
	ReflectionResult, Run, RunId, Schedule, Session, SessionId, SessionKind,
	SessionStatus, Task, TaskId, TaskState, Timestamp, Title, Utterance,
};
use crate::scheduler::Tier;

/// A sum type as stored: discriminant plus JSON payload.
///
/// Split so the discriminant is an indexed column and the payload stays opaque.
pub struct Tagged {
	pub tag: &'static str,
	pub json: String,
}

// --- Generic tag + JSON payload helpers -------------------------------------

/// Extract the payload of a serde externally-tagged sum type.
///
/// Unit variant → `"null"`, otherwise the single value under the variant key.
fn payload_of<T: serde::Serialize>(value: &T) -> Result<String, DbError> {
	match serde_json::to_value(value)? {
		serde_json::Value::Object(obj) => {
			let (_, payload) = obj
				.into_iter()
				.next()
				.expect("a sum type variant with a payload has one key");
			Ok(payload.to_string())
		},
		serde_json::Value::String(_) => Ok("null".to_string()),
		other => Err(DbError::Corrupt(format!(
			"a sum type serialised as `{other}`, neither a tagged object nor a bare string"
		))),
	}
}

/// Reassemble `{tag: payload}` and deserialize the sum type.
///
/// Bad tag or mismatched payload both become `UnknownVariant`.
fn from_tagged<T: serde::de::DeserializeOwned>(
	what: &'static str,
	tag: &str,
	json: &str,
) -> Result<T, DbError> {
	let payload: serde_json::Value = serde_json::from_str(json)?;
	let composite = serde_json::json!({ tag: payload });
	serde_json::from_value(composite)
		.map_err(|_| DbError::UnknownVariant { what, tag: tag.to_string() })
}

pub fn task_state_to_row(state: &TaskState) -> Result<Tagged, DbError> {
	Ok(Tagged {
		tag: state.discriminant().into(),
		json: payload_of(state)?,
	})
}

pub fn task_state_from_row(
	tag: &str,
	json: &str,
) -> Result<TaskState, DbError> {
	from_tagged("task state", tag, json)
}

pub fn schedule_to_row(schedule: &Schedule) -> Result<Tagged, DbError> {
	Ok(Tagged { tag: schedule.into(), json: payload_of(schedule)? })
}

pub fn schedule_from_row(tag: &str, json: &str) -> Result<Schedule, DbError> {
	from_tagged("schedule", tag, json)
}

pub fn call_status_to_row(status: &CallStatus) -> Result<Tagged, DbError> {
	Ok(Tagged { tag: status.into(), json: payload_of(status)? })
}

pub fn call_status_from_row(
	tag: &str,
	json: &str,
) -> Result<CallStatus, DbError> {
	from_tagged("call status", tag, json)
}

pub fn reflection_result_to_row(
	r: &ReflectionResult,
) -> Result<Tagged, DbError> {
	Ok(Tagged { tag: r.into(), json: payload_of(r)? })
}

pub fn reflection_result_from_row(
	tag: &str,
	json: &str,
) -> Result<ReflectionResult, DbError> {
	from_tagged("reflection result", tag, json)
}

pub fn lesson_subject_to_row(s: &LessonSubject) -> Result<Tagged, DbError> {
	Ok(Tagged { tag: s.into(), json: payload_of(s)? })
}

pub fn lesson_subject_from_row(
	tag: &str,
	json: &str,
) -> Result<LessonSubject, DbError> {
	from_tagged("lesson subject", tag, json)
}

/// Encode/decode `SessionStatus` as tag + JSON.
///
/// Session has no separate table; its status column needs the same pair.
pub fn session_status_to_row(
	status: &SessionStatus,
) -> Result<Tagged, DbError> {
	Ok(Tagged { tag: status.into(), json: payload_of(status)? })
}

pub fn session_status_from_row(
	tag: &str,
	json: &str,
) -> Result<SessionStatus, DbError> {
	from_tagged("session status", tag, json)
}

/// Encode `Message` as `role` + `body_json`.
pub fn message_to_row(message: &Message) -> Result<Tagged, DbError> {
	Ok(Tagged { tag: message.into(), json: payload_of(message)? })
}

// --- Small enums stored as a bare string, no payload ------------------------
//
// These have no payload, so they are a column of their own rather than
// a tag and a JSON blob. `strum` spells each one the same way it spells a tag.

fn variant_from_str<T: std::str::FromStr>(
	what: &'static str,
	s: &str,
) -> Result<T, DbError> {
	s.parse()
		.map_err(|_| DbError::UnknownVariant { what, tag: s.to_string() })
}

pub fn channel_kind_from_str(s: &str) -> Result<ChannelKind, DbError> {
	variant_from_str("channel kind", s)
}

/// Decode `calls.tier` from `INTEGER 1..=5`.
fn tier_from_i64(n: i64) -> Result<Tier, DbError> {
	u8::try_from(n)
		.ok()
		.and_then(|n| Tier::try_from(n).ok())
		.ok_or_else(|| {
			DbError::Corrupt(format!("calls.tier is {n}, not 1..=5"))
		})
}

/// Encode `Vec<f32>` as little-endian bytes for `vectors.vector`.
pub fn vector_to_blob(v: &[f32]) -> Vec<u8> {
	v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Decode little-endian bytes back to `Vec<f32>`.
pub fn vector_from_blob(bytes: &[u8]) -> Vec<f32> {
	bytes
		.chunks_exact(4)
		.map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
		.collect()
}

// --- Whole entities ----------------------------------------------------------

pub fn read_run(row: &Row<'_>) -> Result<Run, DbError> {
	let id: i64 = row.get("id")?;
	let started_at: i64 = row.get("started_at")?;
	let ended_at: Option<i64> = row.get("ended_at")?;
	let model: String = row.get("model")?;
	Ok(Run {
		id: RunId(id as u32),
		started_at: Timestamp(started_at),
		ended_at: ended_at.map(Timestamp),
		model,
	})
}

pub fn read_task(row: &Row<'_>) -> Result<Task, DbError> {
	// Read columns
	let id: i64 = row.get("id")?;
	let run: i64 = row.get("run")?;
	let title: String = row.get("title")?;
	let brief: String = row.get("brief")?;
	let role: String = row.get("role")?;
	let state_tag: String = row.get("state")?;
	let state_json: String = row.get("state_json")?;
	let schedule_tag: String = row.get("schedule")?;
	let schedule_json: String = row.get("schedule_json")?;
	let subscriber: Option<i64> = row.get("subscriber")?;
	let priority: String = row.get("priority")?;
	let created_by: String = row.get("created_by")?;
	let created_at: i64 = row.get("created_at")?;

	// Build task
	Ok(Task {
		id: TaskId(id as u32),
		run: RunId(run as u32),
		title: Title::try_from(title)
			.map_err(|e| DbError::Corrupt(e.to_string()))?,
		brief: Brief::try_from(brief)
			.map_err(|e| DbError::Corrupt(e.to_string()))?,
		role: role
			.parse()
			.map_err(|_| DbError::UnknownVariant { what: "role", tag: role })?,
		state: task_state_from_row(&state_tag, &state_json)?,
		schedule: schedule_from_row(&schedule_tag, &schedule_json)?,
		subscriber: subscriber.map(|v| ChannelId(v as u32)),
		priority: serde_json::from_str(&priority)?,
		created_by: serde_json::from_str(&created_by)?,
		created_at: Timestamp(created_at),
	})
}

/// Read a session head without child tables.
///
/// Messages, reflections and mail live in separate tables.
pub fn read_session_head(row: &Row<'_>) -> Result<Session, DbError> {
	// Read common columns
	let id: i64 = row.get("id")?;
	let run: i64 = row.get("run")?;
	let kind_tag: String = row.get("kind")?;
	let status_tag: String = row.get("status")?;
	let status_json: String = row.get("status_json")?;
	let started_at: i64 = row.get("started_at")?;
	let ended_at: Option<i64> = row.get("ended_at")?;

	// Decode kind
	let kind = match kind_tag.as_str() {
		"worker" => {
			let task: i64 = row.get("task")?;
			let role: String = row.get("role")?;
			SessionKind::Worker {
				task: TaskId(task as u32),
				role: role.parse().map_err(|_| DbError::UnknownVariant {
					what: "role",
					tag: role,
				})?,
			}
		},
		"comms" => {
			let channel: i64 = row.get("channel")?;
			SessionKind::Comms {
				channel: ChannelId(channel as u32),
				mailbox: Vec::new(),
			}
		},
		other => {
			return Err(DbError::UnknownVariant {
				what: "session kind",
				tag: other.to_string(),
			});
		},
	};

	// Build session
	Ok(Session {
		id: SessionId(id as u32),
		run: RunId(run as u32),
		kind,
		status: session_status_from_row(&status_tag, &status_json)?,
		messages: Vec::new(),
		reflections: Vec::new(),
		calls: Vec::new(),
		started_at: Timestamp(started_at),
		ended_at: ended_at.map(Timestamp),
	})
}

/// Run one query against a `session`-keyed table and collect every row.
fn collect<T>(
	tx: &Transaction<'_>,
	sql: &str,
	session: u32,
	read: impl Fn(&Row<'_>) -> Result<T, DbError>,
) -> Result<Vec<T>, DbError> {
	let mut stmt = tx.prepare(sql)?;
	let mut rows = stmt.query([session])?;
	let mut out = Vec::new();
	while let Some(row) = rows.next()? {
		out.push(read(row)?);
	}
	Ok(out)
}

/// Load a whole session: head plus ordered child tables.
///
/// Joins messages, reflections, calls and unread mail.
pub fn load_session(
	tx: &Transaction<'_>,
	id: SessionId,
) -> Result<Option<Session>, DbError> {
	// Load head
	let mut session = {
		let mut stmt = tx.prepare("SELECT * FROM sessions WHERE id = ?1")?;
		let mut rows = stmt.query([id.0])?;
		match rows.next()? {
			Some(row) => read_session_head(row)?,
			None => return Ok(None),
		}
	};

	// Collect messages
	session.messages = collect(
		tx,
		"SELECT * FROM messages WHERE session = ?1 ORDER BY idx",
		id.0,
		read_message,
	)?;
	// Collect reflections
	session.reflections = collect(
		tx,
		"SELECT * FROM reflections WHERE session = ?1 ORDER BY idx",
		id.0,
		read_reflection,
	)?;
	// Collect calls
	session.calls = collect(
		tx,
		"SELECT id FROM calls WHERE session = ?1 ORDER BY id",
		id.0,
		|row| Ok(CallId(row.get::<_, i64>(0)? as u32)),
	)?;

	// Collect unread mail
	if let SessionKind::Comms { mailbox, .. } = &mut session.kind {
		*mailbox = collect(
			tx,
			"SELECT * FROM mail WHERE session = ?1 AND read = 0 ORDER BY idx",
			id.0,
			read_incoming,
		)?;
	}

	Ok(Some(session))
}

pub fn read_call(row: &Row<'_>) -> Result<LlmCall, DbError> {
	let id: i64 = row.get("id")?;
	let run: i64 = row.get("run")?;
	let session: i64 = row.get("session")?;
	let tier: i64 = row.get("tier")?;
	let model: String = row.get("model")?;
	let request_json: String = row.get("request_json")?;
	let status_tag: String = row.get("status")?;
	let status_json: String = row.get("status_json")?;
	let queued_at: i64 = row.get("queued_at")?;

	Ok(LlmCall {
		id: CallId(id as u32),
		run: RunId(run as u32),
		session: SessionId(session as u32),
		tier: tier_from_i64(tier)?,
		model,
		request: serde_json::from_str(&request_json)?,
		queued_at: Timestamp(queued_at),
		status: call_status_from_row(&status_tag, &status_json)?,
	})
}

pub fn read_channel(row: &Row<'_>) -> Result<ChannelRecord, DbError> {
	let id: i64 = row.get("id")?;
	let kind: String = row.get("kind")?;
	let session: i64 = row.get("session")?;
	Ok(ChannelRecord {
		id: ChannelId(id as u32),
		kind: channel_kind_from_str(&kind)?,
		session: SessionId(session as u32),
		transcript: Vec::new(),
	})
}

pub fn read_message(row: &Row<'_>) -> Result<Message, DbError> {
	let role: String = row.get("role")?;
	let body_json: String = row.get("body_json")?;
	from_tagged("message", &role, &body_json)
}

pub fn read_reflection(row: &Row<'_>) -> Result<Reflection, DbError> {
	let kind: String = row.get("kind")?;
	let call: i64 = row.get("call")?;
	let after_message: i64 = row.get("after_message")?;
	let result_tag: String = row.get("result")?;
	let result_json: String = row.get("result_json")?;
	let at: i64 = row.get("at")?;
	Ok(Reflection {
		kind: variant_from_str("reflection kind", &kind)?,
		call: CallId(call as u32),
		after_message: after_message as usize,
		at: Timestamp(at),
		result: reflection_result_from_row(&result_tag, &result_json)?,
	})
}

pub fn read_incoming(row: &Row<'_>) -> Result<Incoming, DbError> {
	let from_who: String = row.get("from_who")?;
	let text: String = row.get("text")?;
	let at: i64 = row.get("at")?;
	Ok(Incoming {
		from: variant_from_str("incoming from", &from_who)?,
		text,
		at: Timestamp(at),
	})
}

pub fn read_utterance(row: &Row<'_>) -> Result<Utterance, DbError> {
	let who: String = row.get("who")?;
	let text: String = row.get("text")?;
	let at: i64 = row.get("at")?;
	Ok(Utterance {
		who: variant_from_str("who", &who)?,
		text,
		at: Timestamp(at),
	})
}

pub fn read_lesson(row: &Row<'_>) -> Result<Lesson, DbError> {
	let id: i64 = row.get("id")?;
	let run: i64 = row.get("run")?;
	let text: String = row.get("text")?;
	let day: String = row.get("day")?;
	let session: i64 = row.get("session")?;
	let about_tag: String = row.get("about")?;
	let about_json: String = row.get("about_json")?;
	let at: i64 = row.get("at")?;
	Ok(Lesson {
		id: LessonId(id as u32),
		run: RunId(run as u32),
		text,
		day: Day::try_from(day).map_err(|e| DbError::Corrupt(e.to_string()))?,
		session: SessionId(session as u32),
		about: lesson_subject_from_row(&about_tag, &about_json)?,
		at: Timestamp(at),
	})
}
