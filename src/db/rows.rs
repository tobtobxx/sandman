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
	Brief, CallId, CallStatus, ChannelId, ChannelKind, ChannelRecord, Day,
	Incoming, IncomingFrom, Lesson, LessonId, LessonSubject, LlmCall, Message,
	Reflection, ReflectionKind, ReflectionResult, Run, RunId, Schedule,
	Session, SessionId, SessionKind, SessionStatus, Task, TaskId, TaskState,
	Timestamp, Title, Utterance, Who,
};
use crate::roles::RoleName;
use crate::scheduler::Tier;

/// A sum type as it is stored: the variant's name, and what it carries.
///
/// Split rather than a single tagged JSON blob because the name is a column an
/// index can use.
pub struct Tagged {
	pub tag: &'static str,
	pub json: String,
}

// --- Generic tag + JSON payload helpers -------------------------------------
//
// Every sum type here derives `serde`'s default external tagging: a unit
// variant serialises as a bare string, anything else as `{"variant": data}`.
// `payload_of` normalises both to a JSON payload (`"null"` for the unit case),
// and `from_tagged` reassembles `{tag: payload}` and lets serde do the
// checking — a bad tag or a payload that does not fit the variant both land on
// the one error a caller must handle.

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
		tag: state.discriminant(),
		json: payload_of(state)?,
	})
}

pub fn task_state_from_row(
	tag: &str,
	json: &str,
) -> Result<TaskState, DbError> {
	from_tagged("task state", tag, json)
}

fn schedule_tag(schedule: &Schedule) -> &'static str {
	match schedule {
		Schedule::Now => "now",
		Schedule::At(_) => "at",
		Schedule::Repeating { .. } => "repeating",
	}
}

pub fn schedule_to_row(schedule: &Schedule) -> Result<Tagged, DbError> {
	Ok(Tagged {
		tag: schedule_tag(schedule),
		json: payload_of(schedule)?,
	})
}

pub fn schedule_from_row(tag: &str, json: &str) -> Result<Schedule, DbError> {
	from_tagged("schedule", tag, json)
}

pub fn call_status_to_row(status: &CallStatus) -> Result<Tagged, DbError> {
	Ok(Tagged {
		tag: status.discriminant(),
		json: payload_of(status)?,
	})
}

pub fn call_status_from_row(
	tag: &str,
	json: &str,
) -> Result<CallStatus, DbError> {
	from_tagged("call status", tag, json)
}

fn reflection_result_tag(result: &ReflectionResult) -> &'static str {
	match result {
		ReflectionResult::Ran { .. } => "ran",
		ReflectionResult::FailedOpen { .. } => "failed_open",
	}
}

pub fn reflection_result_to_row(
	r: &ReflectionResult,
) -> Result<Tagged, DbError> {
	Ok(Tagged {
		tag: reflection_result_tag(r),
		json: payload_of(r)?,
	})
}

pub fn reflection_result_from_row(
	tag: &str,
	json: &str,
) -> Result<ReflectionResult, DbError> {
	from_tagged("reflection result", tag, json)
}

pub fn lesson_subject_to_row(s: &LessonSubject) -> Result<Tagged, DbError> {
	Ok(Tagged { tag: s.discriminant(), json: payload_of(s)? })
}

pub fn lesson_subject_from_row(
	tag: &str,
	json: &str,
) -> Result<LessonSubject, DbError> {
	from_tagged("lesson subject", tag, json)
}

/// Not a persisted entity of its own, but `sessions.status`/`status_json` need
/// the same treatment as the tagged pairs above.
pub fn session_status_to_row(
	status: &SessionStatus,
) -> Result<Tagged, DbError> {
	Ok(Tagged {
		tag: status.discriminant(),
		json: payload_of(status)?,
	})
}

pub fn session_status_from_row(
	tag: &str,
	json: &str,
) -> Result<SessionStatus, DbError> {
	from_tagged("session status", tag, json)
}

fn message_tag(message: &Message) -> &'static str {
	match message {
		Message::System { .. } => "system",
		Message::User { .. } => "user",
		Message::Assistant { .. } => "assistant",
		Message::Tool { .. } => "tool",
	}
}

/// `messages.role`/`body_json`, for `append_message`.
pub fn message_to_row(message: &Message) -> Result<Tagged, DbError> {
	Ok(Tagged {
		tag: message_tag(message),
		json: payload_of(message)?,
	})
}

// --- Small enums stored as a bare string, no payload ------------------------

pub fn channel_kind_from_str(s: &str) -> Result<ChannelKind, DbError> {
	match s {
		"stdio" => Ok(ChannelKind::Stdio),
		"web" => Ok(ChannelKind::Web),
		"scripted" => Ok(ChannelKind::Scripted),
		other => Err(DbError::UnknownVariant {
			what: "channel kind",
			tag: other.to_string(),
		}),
	}
}

pub fn who_to_str(who: Who) -> &'static str {
	match who {
		Who::Human => "human",
		Who::Sandman => "sandman",
	}
}

fn who_from_str(s: &str) -> Result<Who, DbError> {
	match s {
		"human" => Ok(Who::Human),
		"sandman" => Ok(Who::Sandman),
		other => {
			Err(DbError::UnknownVariant { what: "who", tag: other.to_string() })
		},
	}
}

pub fn incoming_from_to_str(from: IncomingFrom) -> &'static str {
	match from {
		IncomingFrom::Human => "human",
		IncomingFrom::Swarm => "swarm",
	}
}

fn incoming_from_from_str(s: &str) -> Result<IncomingFrom, DbError> {
	match s {
		"human" => Ok(IncomingFrom::Human),
		"swarm" => Ok(IncomingFrom::Swarm),
		other => Err(DbError::UnknownVariant {
			what: "incoming from",
			tag: other.to_string(),
		}),
	}
}

pub fn reflection_kind_to_str(kind: ReflectionKind) -> &'static str {
	match kind {
		ReflectionKind::Review => "review",
		ReflectionKind::Interrupt => "interrupt",
	}
}

fn reflection_kind_from_str(s: &str) -> Result<ReflectionKind, DbError> {
	match s {
		"review" => Ok(ReflectionKind::Review),
		"interrupt" => Ok(ReflectionKind::Interrupt),
		other => Err(DbError::UnknownVariant {
			what: "reflection kind",
			tag: other.to_string(),
		}),
	}
}

/// `calls.tier`, as [`Tier::as_number`] — the same 1..=5 a Watcher shows.
fn tier_from_i64(n: i64) -> Result<Tier, DbError> {
	match n {
		1 => Ok(Tier::Comms),
		2 => Ok(Tier::TaskHigh),
		3 => Ok(Tier::Metacognition),
		4 => Ok(Tier::TaskNormal),
		5 => Ok(Tier::TaskLow),
		other => Err(DbError::Corrupt(format!(
			"calls.tier is {other}, not 1..=5"
		))),
	}
}

/// A `Vec<f32>` as bytes, for the `vectors` table. Little-endian, four bytes a
/// float; nothing reads this column but this module.
pub fn vector_to_blob(v: &[f32]) -> Vec<u8> {
	v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

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

	Ok(Task {
		id: TaskId(id as u32),
		run: RunId(run as u32),
		title: Title::try_from(title)
			.map_err(|e| DbError::Corrupt(e.to_string()))?,
		brief: Brief::try_from(brief)
			.map_err(|e| DbError::Corrupt(e.to_string()))?,
		role: RoleName::parse(&role).ok_or_else(|| {
			DbError::UnknownVariant { what: "role", tag: role }
		})?,
		state: task_state_from_row(&state_tag, &state_json)?,
		schedule: schedule_from_row(&schedule_tag, &schedule_json)?,
		subscriber: subscriber.map(|v| ChannelId(v as u32)),
		priority: serde_json::from_str(&priority)?,
		created_by: serde_json::from_str(&created_by)?,
		created_at: Timestamp(created_at),
	})
}

/// A Session without its messages, reflections or mail. Those are separate
/// tables; [`load_session`] joins them.
pub fn read_session_head(row: &Row<'_>) -> Result<Session, DbError> {
	let id: i64 = row.get("id")?;
	let run: i64 = row.get("run")?;
	let kind_tag: String = row.get("kind")?;
	let status_tag: String = row.get("status")?;
	let status_json: String = row.get("status_json")?;
	let started_at: i64 = row.get("started_at")?;
	let ended_at: Option<i64> = row.get("ended_at")?;

	let kind = match kind_tag.as_str() {
		"worker" => {
			let task: i64 = row.get("task")?;
			let role: String = row.get("role")?;
			SessionKind::Worker {
				task: TaskId(task as u32),
				role: RoleName::parse(&role).ok_or_else(|| {
					DbError::UnknownVariant { what: "role", tag: role }
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

/// Run one query against a `session`-keyed table and read every row back.
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

/// A whole Session: its head, then its messages, reflections and unread mail in
/// order.
pub fn load_session(
	tx: &Transaction<'_>,
	id: SessionId,
) -> Result<Option<Session>, DbError> {
	let mut session = {
		let mut stmt = tx.prepare("SELECT * FROM sessions WHERE id = ?1")?;
		let mut rows = stmt.query([id.0])?;
		match rows.next()? {
			Some(row) => read_session_head(row)?,
			None => return Ok(None),
		}
	};

	session.messages = collect(
		tx,
		"SELECT * FROM messages WHERE session = ?1 ORDER BY idx",
		id.0,
		read_message,
	)?;
	session.reflections = collect(
		tx,
		"SELECT * FROM reflections WHERE session = ?1 ORDER BY idx",
		id.0,
		read_reflection,
	)?;
	session.calls = collect(
		tx,
		"SELECT id FROM calls WHERE session = ?1 ORDER BY id",
		id.0,
		|row| Ok(CallId(row.get::<_, i64>(0)? as u32)),
	)?;

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
		kind: reflection_kind_from_str(&kind)?,
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
		from: incoming_from_from_str(&from_who)?,
		text,
		at: Timestamp(at),
	})
}

pub fn read_utterance(row: &Row<'_>) -> Result<Utterance, DbError> {
	let who: String = row.get("who")?;
	let text: String = row.get("text")?;
	let at: i64 = row.get("at")?;
	Ok(Utterance { who: who_from_str(&who)?, text, at: Timestamp(at) })
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
