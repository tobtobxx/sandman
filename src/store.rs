//! Everything the Harness owns, behind one vocabulary.
//!
//! The Store is the only thing that touches the database, and the only thing
//! that emits state [`Event`]s. Its interface is domain-shaped — `start_task`,
//! `complete_task`, `append_message` — rather than field-shaped, so a caller
//! says what happened and the Store decides what that means in rows and in the
//! trace.
//!
//! Two properties hold structurally rather than by discipline:
//!
//! **A change without an Event cannot be written.** The connection is private
//! and there is no method that mutates without emitting. In the prototype the
//! Watcher was kept in step by comparing JSON against a shadow twice a second,
//! because announcing at the site of a change fails silently the first time
//! someone forgets. Here there is no site to forget at.
//!
//! **A lock is never held across an await.** Every method takes `&self`, does
//! its work, and returns; nothing hands a guard to a caller. The Mutex is
//! `std::sync::Mutex` deliberately — if a future ever needed to be awaited
//! inside one of these methods, it would not compile, which is the warning we
//! want.
//!
//! Reads return owned values. A model call already carries a detached copy of
//! the messages it was built from, so this is what the system did anyway.
//!
//! Defines: [`Store`], [`StoreError`], [`Snapshot`], [`ListFilter`].

use std::sync::Arc;

use rusqlite::{OptionalExtension, Row};
use strum::IntoDiscriminant;

use crate::db::{Backing, DbError};
use crate::domain::{
	CallId, CallStatus, ChannelId, ChannelKind, ChannelRecord, Cost, Creator,
	Day, Duration, Incoming, Lesson, LessonId, LlmCall, Message, NewCall,
	NewLesson, NewSession, NewTask, Reflection, Run, RunId, Schedule, Session,
	SessionId, SessionKind, SessionStatus, Spend, Task, TaskId, TaskResult,
	TaskState, TaskStateName, TaskSummary, Timestamp, Title, Utterance,
};
use crate::event::{Event, Events};

/// All of Sandman's state.
pub struct Store {
	conn: std::sync::Mutex<rusqlite::Connection>,
	events: Arc<Events>,
	run: RunId,
	/// `(from, to)` if opening this Store migrated the database, or `None` if
	/// it was already current — for whoever holds the Logger to note once at
	/// startup.
	migration: Option<(u32, u32)>,
	/// Exclusive use of the database file, held for this Store's whole life and
	/// dropped after the connection above it. `None` for an in-memory database,
	/// which no other process can reach anyway. See [`crate::db::Lock`].
	_lock: Option<crate::db::Lock>,
}

/// What can go wrong asking the Store for something.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
	#[error(transparent)]
	Db(#[from] DbError),
	#[error("there is no {what} {id}")]
	NoSuch { what: &'static str, id: String },
	/// The belt behind every caller's own check. A cancelled Task must never
	/// complete, and no Task may complete twice.
	#[error("task {task} is {state}, not running; it will not be completed")]
	NotRunning { task: TaskId, state: TaskStateName },
}

/// A `rusqlite::Result` or a `serde_json::Result` on its way to becoming a
/// [`StoreError`]. Both cross through [`DbError`] first, since that is the
/// variant [`StoreError`] actually converts from.
trait IntoStoreError<T> {
	fn store(self) -> Result<T, StoreError>;
}

impl<T> IntoStoreError<T> for Result<T, rusqlite::Error> {
	fn store(self) -> Result<T, StoreError> {
		self.map_err(|e| StoreError::Db(DbError::from(e)))
	}
}

impl<T> IntoStoreError<T> for Result<T, serde_json::Error> {
	fn store(self) -> Result<T, StoreError> {
		self.map_err(|e| StoreError::Db(DbError::from(e)))
	}
}

/// Run a query expected to return exactly one row.
fn read_required<T>(
	conn: &rusqlite::Connection,
	sql: &str,
	params: impl rusqlite::Params,
	read: fn(&Row<'_>) -> Result<T, DbError>,
) -> Result<T, StoreError> {
	Ok(conn.query_row(sql, params, |row| Ok(read(row))).store()??)
}

/// Run a query that may return no row.
fn read_optional<T>(
	conn: &rusqlite::Connection,
	sql: &str,
	params: impl rusqlite::Params,
	read: fn(&Row<'_>) -> Result<T, DbError>,
) -> Result<Option<T>, StoreError> {
	let row = conn
		.query_row(sql, params, |row| Ok(read(row)))
		.optional()
		.store()?;
	Ok(row.transpose()?)
}

/// Which Channel is waiting on the work this Creator is enqueuing.
///
/// The Channel of a Comms Session, and nothing for anyone else. `sessions.channel`
/// is written from [`SessionKind::Comms`] alone, so a Worker cannot resolve to
/// one; a Creator with no Session cannot even ask.
fn subscriber_of(
	conn: &rusqlite::Connection,
	created_by: Creator,
) -> Result<Option<ChannelId>, StoreError> {
	let Creator::Session(session) = created_by else {
		return Ok(None);
	};
	let channel: Option<i64> = conn
		.query_row(
			"SELECT channel FROM sessions WHERE id = ?1",
			[session.0],
			|row| row.get(0),
		)
		.optional()
		.store()?
		.flatten();
	Ok(channel.map(|c| ChannelId(c as u32)))
}

fn task_state_columns(row: &Row<'_>) -> Result<TaskState, DbError> {
	let tag: String = row.get("state")?;
	let json: String = row.get("state_json")?;
	crate::db::rows::task_state_from_row(&tag, &json)
}

/// Every entity, for a Watcher's first frame. After this a Watcher follows the
/// Event stream and never asks again.
#[derive(Debug, Clone)]
pub struct Snapshot {
	pub run: Run,
	pub tasks: Vec<Task>,
	pub sessions: Vec<Session>,
	pub calls: Vec<LlmCall>,
	pub channels: Vec<ChannelRecord>,
	pub lessons: Vec<Lesson>,
}

/// What `list_tasks` narrows the queue to.
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
	pub state: Option<TaskStateName>,
	pub count: Option<usize>,
}

impl Store {
	/// Open a Store, migrate the database, and start a new Run.
	///
	/// Every Task, Session, call and lesson written afterwards belongs to that
	/// Run. Spend is scoped to it; the Lessons and past Tasks are not.
	///
	/// A file-backed database is locked first, before it is so much as opened,
	/// so a start that is refused neither creates nor migrates anything. The
	/// lock is what [`Store::recover`] below stands on.
	pub fn open(
		backing: Backing,
		events: Arc<Events>,
		model: &str,
		now: Timestamp,
	) -> Result<Self, StoreError> {
		let lock = match &backing {
			Backing::File(path) => Some(crate::db::Lock::take(path)?),
			Backing::Memory => None,
		};
		let (mut conn, migration) = crate::db::open(backing)?;
		let run = {
			let tx = conn.transaction().store()?;
			let id = crate::db::counters::take(&tx, RunId::COUNTER)?;
			tx.execute(
				"INSERT INTO runs (id, started_at, model) VALUES (?1, ?2, ?3)",
				rusqlite::params![id, now.0, model],
			)
			.store()?;
			tx.commit().store()?;
			RunId(id)
		};

		let store = Store {
			conn: std::sync::Mutex::new(conn),
			events,
			run,
			migration,
			_lock: lock,
		};
		store.events.emit(Event::RunStarted(Run {
			id: run,
			started_at: now,
			ended_at: None,
			model: model.to_string(),
		}));
		store.recover(now)?;
		Ok(store)
	}

	/// End what an earlier Run left open.
	///
	/// A Run that died — Ctrl+C, a kill, a crash — leaves rows mid-flight: Tasks
	/// marked running with no Session turning behind them, Sessions with no loop
	/// left, calls queued or still out. **Nothing resumes any of it.** A Session
	/// is a live agent context, not a document, so a new Run closes what it finds
	/// instead of inheriting it, and the terminal states say which end it was.
	///
	/// Pending Tasks are the one thing left alone: they are the queue, and a Task
	/// scheduled for tomorrow has to outlive tonight's restart.
	///
	/// Nothing here is scoped to a Run, and nothing needs to be: the Run that
	/// just started owns nothing yet, and [`crate::db::Lock`] means no other
	/// Sandman has this database open. "Still open" and "left behind" are
	/// therefore the same set. Without the lock they would not be, and this
	/// would cancel a live Run's work mid-Turn.
	fn recover(&self, now: Timestamp) -> Result<(), StoreError> {
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;

		let ids = |sql: &str| -> Result<Vec<u32>, StoreError> {
			let mut stmt = tx.prepare(sql).store()?;
			let mut rows = stmt.query([]).store()?;
			let mut ids = Vec::new();
			while let Some(row) = rows.next().store()? {
				ids.push(row.get::<_, i64>(0).store()? as u32);
			}
			Ok(ids)
		};

		let tasks = ids("SELECT id FROM tasks WHERE state = 'running'")?;
		let task_state = TaskState::Cancelled { at: now };
		let row = crate::db::rows::task_state_to_row(&task_state)?;
		for &id in &tasks {
			tx.execute(
				"UPDATE tasks SET state = ?1, state_json = ?2 WHERE id = ?3",
				rusqlite::params![row.tag, row.json, id],
			)
			.store()?;
		}

		let sessions = ids("SELECT id FROM sessions WHERE ended_at IS NULL")?;
		let status = SessionStatus::Cancelled;
		let row = crate::db::rows::session_status_to_row(&status)?;
		for &id in &sessions {
			tx.execute(
				"UPDATE sessions SET status = ?1, status_json = ?2,
				 ended_at = ?3 WHERE id = ?4",
				rusqlite::params![row.tag, row.json, now.0, id],
			)
			.store()?;
		}

		let calls = ids(
			"SELECT id FROM calls WHERE status IN ('queued', 'in_flight')",
		)?;
		let dropped = CallStatus::Dropped { at: now };
		let row = crate::db::rows::call_status_to_row(&dropped)?;
		for &id in &calls {
			tx.execute(
				"UPDATE calls SET status = ?1, status_json = ?2 WHERE id = ?3",
				rusqlite::params![row.tag, row.json, id],
			)
			.store()?;
		}

		tx.commit().store()?;
		drop(conn);

		for id in tasks {
			self.events.emit(Event::TaskStateChanged {
				task: TaskId(id),
				to: task_state.clone(),
			});
		}
		for id in sessions {
			self.events.emit(Event::SessionStatusChanged {
				session: SessionId(id),
				to: status.clone(),
			});
		}
		for id in calls {
			self.events.emit(Event::CallStatusChanged {
				call: CallId(id),
				to: dropped.clone(),
			});
		}
		Ok(())
	}

	/// The Run this Store is writing.
	pub fn run(&self) -> RunId {
		self.run
	}

	/// `(from, to)` if opening this Store migrated the database, for a Logger
	/// to note once at startup.
	pub fn migration(&self) -> Option<(u32, u32)> {
		self.migration
	}

	/// Mark the Run finished. A Run whose process was killed simply never gets
	/// this.
	pub fn end_run(&self, now: Timestamp) -> Result<(), StoreError> {
		let conn = self.conn.lock().unwrap();
		let updated = conn
			.execute(
				"UPDATE runs SET ended_at = ?1 WHERE id = ?2",
				rusqlite::params![now.0, self.run.0],
			)
			.store()?;
		if updated == 0 {
			return Err(StoreError::NoSuch {
				what: "run",
				id: self.run.to_string(),
			});
		}
		let run = read_required(
			&conn,
			"SELECT * FROM runs WHERE id = ?1",
			[self.run.0],
			crate::db::rows::read_run,
		)?;
		drop(conn);
		self.events.emit(Event::RunEnded(run));
		Ok(())
	}

	// --- Tasks -------------------------------------------------------------

	/// Put a Task on the queue. Emits [`crate::event::Event::TaskCreated`].
	///
	/// The subscriber is derived here, from the Creator, and nowhere else: a
	/// Comms Session subscribes the Channel it stands on, everyone else
	/// subscribes nobody. Derived rather than passed in because a caller that
	/// has to remember will one day not, and the Task would complete with its
	/// answer going nowhere.
	pub fn create_task(
		&self,
		new: NewTask,
		now: Timestamp,
	) -> Result<TaskId, StoreError> {
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;
		let id = TaskId(crate::db::counters::take(&tx, TaskId::COUNTER)?);
		let subscriber = subscriber_of(&tx, new.created_by)?;

		let state = TaskState::Pending;
		let state_row = crate::db::rows::task_state_to_row(&state)?;
		let schedule_row = crate::db::rows::schedule_to_row(&new.schedule)?;
		let not_before = new.schedule.not_before(now);
		let priority_json = serde_json::to_string(&new.priority).store()?;
		let created_by_json = serde_json::to_string(&new.created_by).store()?;

		tx.execute(
			"INSERT INTO tasks (
				id, run, title, brief, role, state, state_json,
				schedule, schedule_json, not_before, subscriber, priority,
				created_by, created_at
			) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
			rusqlite::params![
				id.0,
				self.run.0,
				new.title.as_str(),
				new.brief.as_str(),
				new.role.to_string(),
				state_row.tag,
				state_row.json,
				schedule_row.tag,
				schedule_row.json,
				not_before.map(|t| t.0),
				subscriber.map(|c| c.0),
				priority_json,
				created_by_json,
				now.0,
			],
		)
		.store()?;
		tx.commit().store()?;

		let task = Task {
			id,
			run: self.run,
			title: new.title,
			brief: new.brief,
			role: new.role,
			state,
			schedule: new.schedule,
			subscriber,
			priority: new.priority,
			created_by: new.created_by,
			created_at: now,
		};
		drop(conn);
		self.events.emit(Event::TaskCreated(task));
		Ok(id)
	}

	/// Hand a Pending Task to the Session that will do it.
	pub fn start_task(
		&self,
		id: TaskId,
		session: SessionId,
		now: Timestamp,
	) -> Result<(), StoreError> {
		let conn = self.conn.lock().unwrap();
		let state = TaskState::Running { session, started_at: now };
		let row = crate::db::rows::task_state_to_row(&state)?;
		let updated = conn
			.execute(
				"UPDATE tasks SET state = ?1, state_json = ?2 WHERE id = ?3",
				rusqlite::params![row.tag, row.json, id.0],
			)
			.store()?;
		if updated == 0 {
			return Err(StoreError::NoSuch {
				what: "task",
				id: id.to_string(),
			});
		}
		drop(conn);
		self.events
			.emit(Event::TaskStateChanged { task: id, to: state });
		Ok(())
	}

	/// Record a Task's Result. Fails if the Task is not Running — which is what
	/// stops a cancelled Task completing and a repeating chain re-arming after
	/// it was stopped.
	pub fn complete_task(
		&self,
		id: TaskId,
		result: TaskResult,
		now: Timestamp,
	) -> Result<(), StoreError> {
		let conn = self.conn.lock().unwrap();
		let current = read_optional(
			&conn,
			"SELECT state, state_json FROM tasks WHERE id = ?1",
			[id.0],
			task_state_columns,
		)?
		.ok_or_else(|| StoreError::NoSuch {
			what: "task",
			id: id.to_string(),
		})?;
		if !matches!(current, TaskState::Running { .. }) {
			return Err(StoreError::NotRunning {
				task: id,
				state: current.discriminant(),
			});
		}

		let new_state = TaskState::Completed { result, at: now };
		let row = crate::db::rows::task_state_to_row(&new_state)?;
		conn.execute(
			"UPDATE tasks SET state = ?1, state_json = ?2 WHERE id = ?3",
			rusqlite::params![row.tag, row.json, id.0],
		)
		.store()?;
		drop(conn);
		self.events
			.emit(Event::TaskStateChanged { task: id, to: new_state });
		Ok(())
	}

	/// Stop Tasks. Cancelling is terminal: a pending Task never runs, a running
	/// one ends at its Session's next decision point with no Result, and a
	/// repeating one stops as a chain.
	pub fn cancel_tasks(
		&self,
		ids: &[TaskId],
		now: Timestamp,
	) -> Result<(), StoreError> {
		let conn = self.conn.lock().unwrap();
		let mut cancelled = Vec::new();
		for &id in ids {
			let current = read_optional(
				&conn,
				"SELECT state, state_json FROM tasks WHERE id = ?1",
				[id.0],
				task_state_columns,
			)?;
			let Some(current) = current else { continue };
			if current.is_terminal() {
				continue;
			}
			let state = TaskState::Cancelled { at: now };
			let row = crate::db::rows::task_state_to_row(&state)?;
			conn.execute(
				"UPDATE tasks SET state = ?1, state_json = ?2 WHERE id = ?3",
				rusqlite::params![row.tag, row.json, id.0],
			)
			.store()?;
			cancelled.push((id, state));
		}
		drop(conn);
		for (task, to) in cancelled {
			self.events.emit(Event::TaskStateChanged { task, to });
		}
		Ok(())
	}

	pub fn task(&self, id: TaskId) -> Result<Option<Task>, StoreError> {
		let conn = self.conn.lock().unwrap();
		read_optional(
			&conn,
			"SELECT * FROM tasks WHERE id = ?1",
			[id.0],
			crate::db::rows::read_task,
		)
	}

	/// Just the state, for the cancellation check at the top of a turn — the
	/// hottest read in the system.
	pub fn task_state(
		&self,
		id: TaskId,
	) -> Result<Option<TaskState>, StoreError> {
		let conn = self.conn.lock().unwrap();
		read_optional(
			&conn,
			"SELECT state, state_json FROM tasks WHERE id = ?1",
			[id.0],
			task_state_columns,
		)
	}

	/// The first Pending Task whose time has come.
	///
	/// Time is the one condition on being picked. Waiting on other work is not
	/// here: a Session holds for another Session, through `await_result`, after
	/// a Task has already started.
	pub fn next_pending(
		&self,
		now: Timestamp,
	) -> Result<Option<Task>, StoreError> {
		let conn = self.conn.lock().unwrap();
		read_optional(
			&conn,
			"SELECT * FROM tasks
			 WHERE state = 'pending'
			   AND (not_before IS NULL OR not_before <= ?1)
			 ORDER BY COALESCE(not_before, 0) ASC, id ASC
			 LIMIT 1",
			[now.0],
			crate::db::rows::read_task,
		)
	}

	/// How long until the earliest scheduled Task can run, if one is waiting.
	pub fn next_due_in(
		&self,
		now: Timestamp,
	) -> Result<Option<Duration>, StoreError> {
		let conn = self.conn.lock().unwrap();
		let earliest: Option<i64> = conn
			.query_row(
				"SELECT MIN(COALESCE(not_before, ?1))
				 FROM tasks WHERE state = 'pending'",
				[now.0],
				|row| row.get(0),
			)
			.store()?;
		Ok(earliest.map(|t| now.until(Timestamp(t))))
	}

	/// Every occurrence of one repeating chain that has not finished — so
	/// cancelling one occurrence can stop the chain.
	pub fn chain_of(&self, id: TaskId) -> Result<Vec<TaskId>, StoreError> {
		let conn = self.conn.lock().unwrap();
		let anchor = read_optional(
			&conn,
			"SELECT * FROM tasks WHERE id = ?1",
			[id.0],
			crate::db::rows::read_task,
		)?
		.ok_or_else(|| StoreError::NoSuch {
			what: "task",
			id: id.to_string(),
		})?;
		let Schedule::Repeating { every, .. } = anchor.schedule else {
			return Ok(vec![id]);
		};

		let mut stmt = conn
			.prepare(
				// No `subscriber` here: it is a function of `created_by`, so
				// matching on both would only say the same thing twice.
				"SELECT id, schedule, schedule_json FROM tasks
				 WHERE role = ?1 AND title = ?2 AND brief = ?3
				   AND created_by = ?4
				   AND state IN ('pending', 'running')",
			)
			.store()?;
		let created_by = serde_json::to_string(&anchor.created_by).store()?;
		let mut rows = stmt
			.query(rusqlite::params![
				anchor.role.to_string(),
				anchor.title.as_str(),
				anchor.brief.as_str(),
				created_by,
			])
			.store()?;

		let mut chain = Vec::new();
		while let Some(row) = rows.next().store()? {
			let candidate: i64 = row.get(0).store()?;
			let tag: String = row.get(1).store()?;
			let json: String = row.get(2).store()?;
			let schedule = crate::db::rows::schedule_from_row(&tag, &json)?;
			if matches!(schedule, Schedule::Repeating { every: e, .. } if e == every)
			{
				chain.push(TaskId(candidate as u32));
			}
		}
		Ok(chain)
	}

	pub fn list_tasks(
		&self,
		filter: ListFilter,
	) -> Result<Vec<TaskSummary>, StoreError> {
		let conn = self.conn.lock().unwrap();

		let mut sql = String::from(
			"SELECT id, title, role, state, state_json, schedule,
			        schedule_json, created_at
			 FROM tasks WHERE run = ?",
		);
		let mut params: Vec<Box<dyn rusqlite::ToSql>> =
			vec![Box::new(self.run.0)];

		if let Some(state) = filter.state {
			sql.push_str(" AND state = ?");
			params.push(Box::new(<&str>::from(state)));
		}
		sql.push_str(" ORDER BY id DESC");
		if let Some(count) = filter.count {
			sql.push_str(" LIMIT ?");
			params.push(Box::new(count as i64));
		}

		let mut stmt = conn.prepare(&sql).store()?;
		let mut rows = stmt
			.query(rusqlite::params_from_iter(
				params.iter().map(|p| p.as_ref()),
			))
			.store()?;

		let mut summaries = Vec::new();
		while let Some(row) = rows.next().store()? {
			let id: i64 = row.get(0).store()?;
			let title: String = row.get(1).store()?;
			let role: String = row.get(2).store()?;
			let state_tag: String = row.get(3).store()?;
			let state_json: String = row.get(4).store()?;
			let schedule_tag: String = row.get(5).store()?;
			let schedule_json: String = row.get(6).store()?;
			let created_at: i64 = row.get(7).store()?;

			summaries.push(TaskSummary {
				id: TaskId(id as u32),
				title: Title::try_from(title)
					.map_err(|e| DbError::Corrupt(e.to_string()))?,
				role: role.parse().map_err(|_| DbError::UnknownVariant {
					what: "role",
					tag: role,
				})?,
				state: crate::db::rows::task_state_from_row(
					&state_tag,
					&state_json,
				)?,
				schedule: crate::db::rows::schedule_from_row(
					&schedule_tag,
					&schedule_json,
				)?,
				created_at: Timestamp(created_at),
			});
		}
		Ok(summaries)
	}

	/// Every Task in this Run, newest last. What a one-shot run prints.
	pub fn tasks_of_run(&self, run: RunId) -> Result<Vec<Task>, StoreError> {
		let conn = self.conn.lock().unwrap();
		let mut stmt = conn
			.prepare("SELECT * FROM tasks WHERE run = ?1 ORDER BY id ASC")
			.store()?;
		let mut rows = stmt.query([run.0]).store()?;
		let mut tasks = Vec::new();
		while let Some(row) = rows.next().store()? {
			tasks.push(crate::db::rows::read_task(row)?);
		}
		Ok(tasks)
	}

	/// Every Task ever, for a search by meaning. Not scoped to a Run: what the
	/// swarm already asked and answered is worth finding whenever it happened.
	pub fn all_tasks(&self) -> Result<Vec<Task>, StoreError> {
		let conn = self.conn.lock().unwrap();
		let mut stmt = conn
			.prepare("SELECT * FROM tasks ORDER BY id ASC")
			.store()?;
		let mut rows = stmt.query([]).store()?;
		let mut tasks = Vec::new();
		while let Some(row) = rows.next().store()? {
			tasks.push(crate::db::rows::read_task(row)?);
		}
		Ok(tasks)
	}

	// --- Sessions ------------------------------------------------------------

	pub fn start_session(
		&self,
		new: NewSession,
		now: Timestamp,
	) -> Result<SessionId, StoreError> {
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;
		let id = SessionId(crate::db::counters::take(&tx, SessionId::COUNTER)?);

		let (kind_tag, task, role, channel) = match &new.kind {
			SessionKind::Worker { task, role } => {
				("worker", Some(task.0 as i64), Some(role.to_string()), None)
			},
			SessionKind::Comms { channel, .. } => {
				("comms", None, None, Some(channel.0 as i64))
			},
		};
		let status_row = crate::db::rows::session_status_to_row(&new.status)?;

		tx.execute(
			"INSERT INTO sessions (
				id, run, kind, task, role, channel, status, status_json,
				started_at
			) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
			rusqlite::params![
				id.0,
				self.run.0,
				kind_tag,
				task,
				role,
				channel,
				status_row.tag,
				status_row.json,
				now.0,
			],
		)
		.store()?;

		for (idx, message) in new.messages.iter().enumerate() {
			let row = crate::db::rows::message_to_row(message)?;
			tx.execute(
				"INSERT INTO messages (session, idx, role, body_json)
				 VALUES (?1,?2,?3,?4)",
				rusqlite::params![id.0, idx as i64, row.tag, row.json],
			)
			.store()?;
		}

		tx.commit().store()?;

		let session = Session {
			id,
			run: self.run,
			kind: new.kind,
			status: new.status,
			messages: new.messages,
			reflections: Vec::new(),
			calls: Vec::new(),
			started_at: now,
			ended_at: None,
		};
		drop(conn);
		self.events.emit(Event::SessionStarted(session));
		Ok(id)
	}

	pub fn set_status(
		&self,
		id: SessionId,
		status: SessionStatus,
	) -> Result<(), StoreError> {
		let conn = self.conn.lock().unwrap();
		let row = crate::db::rows::session_status_to_row(&status)?;
		let updated = conn
			.execute(
				"UPDATE sessions SET status = ?1, status_json = ?2
				 WHERE id = ?3",
				rusqlite::params![row.tag, row.json, id.0],
			)
			.store()?;
		if updated == 0 {
			return Err(StoreError::NoSuch {
				what: "session",
				id: id.to_string(),
			});
		}
		drop(conn);
		self.events
			.emit(Event::SessionStatusChanged { session: id, to: status });
		Ok(())
	}

	pub fn end_session(
		&self,
		id: SessionId,
		status: SessionStatus,
		now: Timestamp,
	) -> Result<(), StoreError> {
		let conn = self.conn.lock().unwrap();
		let row = crate::db::rows::session_status_to_row(&status)?;
		let updated = conn
			.execute(
				"UPDATE sessions SET status = ?1, status_json = ?2,
				 ended_at = ?3 WHERE id = ?4",
				rusqlite::params![row.tag, row.json, now.0, id.0],
			)
			.store()?;
		if updated == 0 {
			return Err(StoreError::NoSuch {
				what: "session",
				id: id.to_string(),
			});
		}
		drop(conn);
		self.events
			.emit(Event::SessionStatusChanged { session: id, to: status });
		Ok(())
	}

	/// Append one message and return its index in the conversation.
	pub fn append_message(
		&self,
		id: SessionId,
		message: Message,
	) -> Result<usize, StoreError> {
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;
		let index: i64 = tx
			.query_row(
				"SELECT COUNT(*) FROM messages WHERE session = ?1",
				[id.0],
				|row| row.get(0),
			)
			.store()?;
		let row = crate::db::rows::message_to_row(&message)?;
		tx.execute(
			"INSERT INTO messages (session, idx, role, body_json)
			 VALUES (?1,?2,?3,?4)",
			rusqlite::params![id.0, index, row.tag, row.json],
		)
		.store()?;
		tx.commit().store()?;
		drop(conn);

		let index = index as usize;
		self.events.emit(Event::MessageAppended {
			session: id,
			index,
			message,
		});
		Ok(index)
	}

	/// The whole conversation, as a model call needs it.
	pub fn messages(&self, id: SessionId) -> Result<Vec<Message>, StoreError> {
		let conn = self.conn.lock().unwrap();
		let mut stmt = conn
			.prepare("SELECT * FROM messages WHERE session = ?1 ORDER BY idx")
			.store()?;
		let mut rows = stmt.query([id.0]).store()?;
		let mut out = Vec::new();
		while let Some(row) = rows.next().store()? {
			out.push(crate::db::rows::read_message(row)?);
		}
		Ok(out)
	}

	/// How many messages this Session has. What the interrupt counts.
	pub fn message_count(&self, id: SessionId) -> Result<usize, StoreError> {
		let conn = self.conn.lock().unwrap();
		let count: i64 = conn
			.query_row(
				"SELECT COUNT(*) FROM messages WHERE session = ?1",
				[id.0],
				|row| row.get(0),
			)
			.store()?;
		Ok(count as usize)
	}

	pub fn record_reflection(
		&self,
		id: SessionId,
		r: Reflection,
	) -> Result<(), StoreError> {
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;
		let index: i64 = tx
			.query_row(
				"SELECT COUNT(*) FROM reflections WHERE session = ?1",
				[id.0],
				|row| row.get(0),
			)
			.store()?;
		let row = crate::db::rows::reflection_result_to_row(&r.result)?;
		tx.execute(
			"INSERT INTO reflections (
				session, idx, kind, call, after_message, result, result_json, at
			) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
			rusqlite::params![
				id.0,
				index,
				<&str>::from(r.kind),
				r.call.0,
				r.after_message as i64,
				row.tag,
				row.json,
				r.at.0,
			],
		)
		.store()?;
		tx.commit().store()?;
		drop(conn);
		self.events
			.emit(Event::ReflectionRecorded { session: id, reflection: r });
		Ok(())
	}

	/// The most recent metacognition, whichever kind — what the interrupt counts
	/// from.
	pub fn last_reflection(
		&self,
		id: SessionId,
	) -> Result<Option<Reflection>, StoreError> {
		let conn = self.conn.lock().unwrap();
		read_optional(
			&conn,
			"SELECT * FROM reflections WHERE session = ?1
			 ORDER BY idx DESC LIMIT 1",
			[id.0],
			crate::db::rows::read_reflection,
		)
	}

	pub fn session(
		&self,
		id: SessionId,
	) -> Result<Option<Session>, StoreError> {
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;
		let session = crate::db::rows::load_session(&tx, id)?;
		tx.commit().store()?;
		Ok(session)
	}

	/// The Session that did a Task, if one ever ran it.
	///
	/// A Task carries no Session id once it is done — [`TaskState::Completed`]
	/// has none — so this is the only way back from a `search_tasks` hit to the
	/// conversation behind it.
	pub fn session_for_task(
		&self,
		task: TaskId,
	) -> Result<Option<SessionId>, StoreError> {
		let conn = self.conn.lock().unwrap();
		let session: Option<i64> = conn
			.query_row(
				"SELECT id FROM sessions WHERE task = ?1",
				[task.0],
				|row| row.get(0),
			)
			.optional()
			.store()?;
		Ok(session.map(|v| SessionId(v as u32)))
	}

	/// Put something in a Comms Session's mailbox.
	pub fn receive_mail(
		&self,
		id: SessionId,
		incoming: Incoming,
	) -> Result<(), StoreError> {
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;
		let index: i64 = tx
			.query_row(
				"SELECT COUNT(*) FROM mail WHERE session = ?1",
				[id.0],
				|row| row.get(0),
			)
			.store()?;
		tx.execute(
			"INSERT INTO mail (session, idx, from_who, text, at, read)
			 VALUES (?1,?2,?3,?4,?5,0)",
			rusqlite::params![
				id.0,
				index,
				<&str>::from(incoming.from),
				incoming.text,
				incoming.at.0,
			],
		)
		.store()?;
		tx.commit().store()?;
		drop(conn);
		self.events
			.emit(Event::MailReceived { session: id, incoming });
		Ok(())
	}

	/// Take everything unread from a mailbox, marking it read in the same
	/// transaction. Post that lands mid-turn waits for the next one.
	pub fn take_mail(
		&self,
		id: SessionId,
	) -> Result<Vec<Incoming>, StoreError> {
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;
		let unread = {
			let mut stmt = tx
				.prepare(
					"SELECT * FROM mail WHERE session = ?1 AND read = 0
					 ORDER BY idx",
				)
				.store()?;
			let mut rows = stmt.query([id.0]).store()?;
			let mut out = Vec::new();
			while let Some(row) = rows.next().store()? {
				out.push(crate::db::rows::read_incoming(row)?);
			}
			out
		};
		tx.execute(
			"UPDATE mail SET read = 1 WHERE session = ?1 AND read = 0",
			[id.0],
		)
		.store()?;
		tx.commit().store()?;
		Ok(unread)
	}

	pub fn has_mail(&self, id: SessionId) -> Result<bool, StoreError> {
		let conn = self.conn.lock().unwrap();
		let count: i64 = conn
			.query_row(
				"SELECT COUNT(*) FROM mail WHERE session = ?1 AND read = 0",
				[id.0],
				|row| row.get(0),
			)
			.store()?;
		Ok(count > 0)
	}

	// --- Model calls -----------------------------------------------------------

	/// Record a call as it joins the scheduler's queue, before it is sent, so
	/// waiting is as visible as working.
	pub fn queue_call(
		&self,
		new: NewCall,
		now: Timestamp,
	) -> Result<CallId, StoreError> {
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;
		let id = CallId(crate::db::counters::take(&tx, CallId::COUNTER)?);

		let status = CallStatus::Queued;
		let status_row = crate::db::rows::call_status_to_row(&status)?;
		let request_json = serde_json::to_string(&new.request).store()?;

		tx.execute(
			"INSERT INTO calls (
				id, run, session, tier, model, request_json, status,
				status_json, queued_at
			) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
			rusqlite::params![
				id.0,
				self.run.0,
				new.session.0,
				u8::from(new.tier),
				new.model,
				request_json,
				status_row.tag,
				status_row.json,
				now.0,
			],
		)
		.store()?;
		tx.commit().store()?;

		let call = LlmCall {
			id,
			run: self.run,
			session: new.session,
			tier: new.tier,
			model: new.model,
			request: new.request,
			queued_at: now,
			status,
		};
		drop(conn);
		self.events.emit(Event::CallQueued(call));
		Ok(id)
	}

	pub fn set_call_status(
		&self,
		id: CallId,
		status: CallStatus,
	) -> Result<(), StoreError> {
		let conn = self.conn.lock().unwrap();
		let row = crate::db::rows::call_status_to_row(&status)?;
		let (tokens, cost) = match &status {
			CallStatus::Done { usage, .. } => {
				(Some(usage.tokens as i64), Some(usage.cost.0))
			},
			_ => (None, None),
		};
		let updated = conn
			.execute(
				"UPDATE calls SET status = ?1, status_json = ?2, tokens = ?3,
				 cost = ?4 WHERE id = ?5",
				rusqlite::params![row.tag, row.json, tokens, cost, id.0],
			)
			.store()?;
		if updated == 0 {
			return Err(StoreError::NoSuch {
				what: "call",
				id: id.to_string(),
			});
		}
		drop(conn);
		self.events
			.emit(Event::CallStatusChanged { call: id, to: status });
		Ok(())
	}

	pub fn call(&self, id: CallId) -> Result<Option<LlmCall>, StoreError> {
		let conn = self.conn.lock().unwrap();
		read_optional(
			&conn,
			"SELECT * FROM calls WHERE id = ?1",
			[id.0],
			crate::db::rows::read_call,
		)
	}

	/// What this Run has cost. Summed from the calls that finished, never
	/// accumulated, so it cannot drift from them.
	pub fn spend(&self, run: RunId) -> Result<Spend, StoreError> {
		let conn = self.conn.lock().unwrap();
		let (calls, tokens, cost): (i64, Option<i64>, Option<i64>) = conn
			.query_row(
				"SELECT COUNT(*), SUM(tokens), SUM(cost) FROM calls
				 WHERE run = ?1 AND status = 'done'",
				[run.0],
				|row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
			)
			.store()?;
		Ok(Spend {
			calls: calls as u32,
			tokens: tokens.unwrap_or(0) as u64,
			cost: Cost(cost.unwrap_or(0)),
		})
	}

	/// Whether any call is still queued or in flight — what a wind-down waits
	/// for, so the last call's cost reaches the record.
	pub fn calls_outstanding(&self) -> Result<bool, StoreError> {
		let conn = self.conn.lock().unwrap();
		let count: i64 = conn
			.query_row(
				"SELECT COUNT(*) FROM calls
				 WHERE run = ?1 AND status IN ('queued', 'in_flight')",
				[self.run.0],
				|row| row.get(0),
			)
			.store()?;
		Ok(count > 0)
	}

	// --- Channels ----------------------------------------------------------

	/// Open a Channel and stand a fresh Comms Session on it, minting both ids
	/// in one transaction.
	///
	/// Neither `start_session` nor `open_channel` alone can do this: a Comms
	/// Session's `kind` carries the Channel it stands on, and a Channel's row
	/// is a real foreign key to its Session. Whichever went first would need
	/// an id that does not exist yet. Minting both here, in one transaction,
	/// is what breaks the cycle.
	pub fn open_comms(
		&self,
		kind: ChannelKind,
		messages: Vec<Message>,
		now: Timestamp,
	) -> Result<(SessionId, ChannelId), StoreError> {
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;
		let session_id =
			SessionId(crate::db::counters::take(&tx, SessionId::COUNTER)?);
		let channel_id =
			ChannelId(crate::db::counters::take(&tx, ChannelId::COUNTER)?);

		let session_kind =
			SessionKind::Comms { channel: channel_id, mailbox: Vec::new() };
		let status = SessionStatus::Idle;
		let status_row = crate::db::rows::session_status_to_row(&status)?;

		tx.execute(
			"INSERT INTO sessions (
				id, run, kind, task, role, channel, status, status_json,
				started_at
			) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
			rusqlite::params![
				session_id.0,
				self.run.0,
				"comms",
				Option::<i64>::None,
				Option::<String>::None,
				channel_id.0,
				status_row.tag,
				status_row.json,
				now.0,
			],
		)
		.store()?;

		for (idx, message) in messages.iter().enumerate() {
			let row = crate::db::rows::message_to_row(message)?;
			tx.execute(
				"INSERT INTO messages (session, idx, role, body_json)
				 VALUES (?1,?2,?3,?4)",
				rusqlite::params![session_id.0, idx as i64, row.tag, row.json],
			)
			.store()?;
		}

		tx.execute(
			"INSERT INTO channels (id, kind, session) VALUES (?1,?2,?3)",
			rusqlite::params![channel_id.0, <&str>::from(kind), session_id.0],
		)
		.store()?;

		tx.commit().store()?;

		let session = Session {
			id: session_id,
			run: self.run,
			kind: session_kind,
			status,
			messages,
			reflections: Vec::new(),
			calls: Vec::new(),
			started_at: now,
			ended_at: None,
		};
		drop(conn);
		self.events.emit(Event::SessionStarted(session));
		self.events.emit(Event::ChannelOpened {
			channel: channel_id,
			session: session_id,
		});
		Ok((session_id, channel_id))
	}

	pub fn open_channel(
		&self,
		kind: ChannelKind,
		session: SessionId,
	) -> Result<ChannelId, StoreError> {
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;
		let id = ChannelId(crate::db::counters::take(&tx, ChannelId::COUNTER)?);
		tx.execute(
			"INSERT INTO channels (id, kind, session) VALUES (?1,?2,?3)",
			rusqlite::params![id.0, <&str>::from(kind), session.0],
		)
		.store()?;
		tx.commit().store()?;
		drop(conn);
		self.events
			.emit(Event::ChannelOpened { channel: id, session });
		Ok(id)
	}

	pub fn say(
		&self,
		channel: ChannelId,
		utterance: Utterance,
	) -> Result<(), StoreError> {
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;
		let index: i64 = tx
			.query_row(
				"SELECT COUNT(*) FROM utterances WHERE channel = ?1",
				[channel.0],
				|row| row.get(0),
			)
			.store()?;
		tx.execute(
			"INSERT INTO utterances (channel, idx, who, text, at)
			 VALUES (?1,?2,?3,?4,?5)",
			rusqlite::params![
				channel.0,
				index,
				<&str>::from(utterance.who),
				utterance.text,
				utterance.at.0,
			],
		)
		.store()?;
		tx.commit().store()?;
		drop(conn);
		self.events.emit(Event::Said { channel, utterance });
		Ok(())
	}

	pub fn transcript(
		&self,
		channel: ChannelId,
	) -> Result<Vec<Utterance>, StoreError> {
		let conn = self.conn.lock().unwrap();
		let mut stmt = conn
			.prepare("SELECT * FROM utterances WHERE channel = ?1 ORDER BY idx")
			.store()?;
		let mut rows = stmt.query([channel.0]).store()?;
		let mut out = Vec::new();
		while let Some(row) = rows.next().store()? {
			out.push(crate::db::rows::read_utterance(row)?);
		}
		Ok(out)
	}

	pub fn channels(&self) -> Result<Vec<ChannelRecord>, StoreError> {
		let conn = self.conn.lock().unwrap();
		let mut heads = Vec::new();
		{
			let mut stmt = conn
				.prepare(
					"SELECT c.* FROM channels c
					 JOIN sessions s ON s.id = c.session
					 WHERE s.run = ?1
					 ORDER BY c.id",
				)
				.store()?;
			let mut rows = stmt.query([self.run.0]).store()?;
			while let Some(row) = rows.next().store()? {
				heads.push(crate::db::rows::read_channel(row)?);
			}
		}
		for ch in &mut heads {
			let mut stmt = conn
				.prepare(
					"SELECT * FROM utterances WHERE channel = ?1 ORDER BY idx",
				)
				.store()?;
			let mut rows = stmt.query([ch.id.0]).store()?;
			while let Some(row) = rows.next().store()? {
				ch.transcript.push(crate::db::rows::read_utterance(row)?);
			}
		}
		Ok(heads)
	}

	/// One Channel, with its whole transcript, for a Watcher patching a single
	/// entity — see [`crate::web::wire`].
	pub fn channel(
		&self,
		id: ChannelId,
	) -> Result<Option<ChannelRecord>, StoreError> {
		let conn = self.conn.lock().unwrap();
		let Some(mut head) = read_optional(
			&conn,
			"SELECT * FROM channels WHERE id = ?1",
			[id.0],
			crate::db::rows::read_channel,
		)?
		else {
			return Ok(None);
		};
		let mut stmt = conn
			.prepare("SELECT * FROM utterances WHERE channel = ?1 ORDER BY idx")
			.store()?;
		let mut rows = stmt.query([id.0]).store()?;
		while let Some(row) = rows.next().store()? {
			head.transcript.push(crate::db::rows::read_utterance(row)?);
		}
		Ok(Some(head))
	}

	/// The Comms Session standing on a Channel.
	pub fn channel_session(
		&self,
		channel: ChannelId,
	) -> Result<Option<SessionId>, StoreError> {
		let conn = self.conn.lock().unwrap();
		let session: Option<i64> = conn
			.query_row(
				"SELECT session FROM channels WHERE id = ?1",
				[channel.0],
				|row| row.get(0),
			)
			.optional()
			.store()?;
		Ok(session.map(|v| SessionId(v as u32)))
	}

	// --- Lessons -------------------------------------------------------------

	pub fn keep_lesson(
		&self,
		new: NewLesson,
		now: Timestamp,
	) -> Result<LessonId, StoreError> {
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;
		let id = LessonId(crate::db::counters::take(&tx, LessonId::COUNTER)?);
		let day = Day::today(now);
		let about_row = crate::db::rows::lesson_subject_to_row(&new.about)?;

		tx.execute(
			"INSERT INTO lessons (
				id, run, text, day, session, about, about_json, at
			) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
			rusqlite::params![
				id.0,
				self.run.0,
				new.text,
				day.as_str(),
				new.session.0,
				about_row.tag,
				about_row.json,
				now.0,
			],
		)
		.store()?;
		tx.commit().store()?;

		let lesson = Lesson {
			id,
			run: self.run,
			text: new.text,
			day,
			session: new.session,
			about: new.about,
			at: now,
		};
		drop(conn);
		self.events.emit(Event::LessonKept(lesson));
		Ok(id)
	}

	/// Every lesson ever written, across every Run.
	pub fn all_lessons(&self) -> Result<Vec<Lesson>, StoreError> {
		let conn = self.conn.lock().unwrap();
		let mut stmt =
			conn.prepare("SELECT * FROM lessons ORDER BY id").store()?;
		let mut rows = stmt.query([]).store()?;
		let mut out = Vec::new();
		while let Some(row) = rows.next().store()? {
			out.push(crate::db::rows::read_lesson(row)?);
		}
		Ok(out)
	}

	/// A cached embedding, and where to put one. Kept out of the entity tables:
	/// a vector is several hundred floats no human reads.
	pub fn vector(
		&self,
		key: &str,
		model: &str,
	) -> Result<Option<Vec<f32>>, StoreError> {
		let conn = self.conn.lock().unwrap();
		let blob: Option<Vec<u8>> = conn
			.query_row(
				"SELECT vector FROM vectors WHERE key = ?1 AND model = ?2",
				rusqlite::params![key, model],
				|row| row.get(0),
			)
			.optional()
			.store()?;
		Ok(blob.map(|b| crate::db::rows::vector_from_blob(&b)))
	}

	pub fn put_vector(
		&self,
		key: &str,
		model: &str,
		v: &[f32],
	) -> Result<(), StoreError> {
		let conn = self.conn.lock().unwrap();
		conn.execute(
			"INSERT INTO vectors (key, model, vector) VALUES (?1,?2,?3)
			 ON CONFLICT(key) DO UPDATE SET
			 	model = excluded.model, vector = excluded.vector",
			rusqlite::params![key, model, crate::db::rows::vector_to_blob(v)],
		)
		.store()?;
		Ok(())
	}

	// --- Watching --------------------------------------------------------------

	/// Everything, for a Watcher's first frame.
	pub fn snapshot(&self) -> Result<Snapshot, StoreError> {
		let mut conn = self.conn.lock().unwrap();

		let run = read_required(
			&conn,
			"SELECT * FROM runs WHERE id = ?1",
			[self.run.0],
			crate::db::rows::read_run,
		)?;

		let mut tasks = Vec::new();
		{
			let mut stmt = conn
				.prepare("SELECT * FROM tasks WHERE run = ?1 ORDER BY id")
				.store()?;
			let mut rows = stmt.query([self.run.0]).store()?;
			while let Some(row) = rows.next().store()? {
				tasks.push(crate::db::rows::read_task(row)?);
			}
		}

		let session_ids: Vec<i64> = {
			let mut stmt = conn
				.prepare("SELECT id FROM sessions WHERE run = ?1 ORDER BY id")
				.store()?;
			let mut rows = stmt.query([self.run.0]).store()?;
			let mut ids = Vec::new();
			while let Some(row) = rows.next().store()? {
				ids.push(row.get::<_, i64>(0).store()?);
			}
			ids
		};
		let mut sessions = Vec::new();
		{
			let tx = conn.transaction().store()?;
			for id in session_ids {
				if let Some(session) =
					crate::db::rows::load_session(&tx, SessionId(id as u32))?
				{
					sessions.push(session);
				}
			}
			tx.commit().store()?;
		}

		let mut calls = Vec::new();
		{
			let mut stmt = conn
				.prepare("SELECT * FROM calls WHERE run = ?1 ORDER BY id")
				.store()?;
			let mut rows = stmt.query([self.run.0]).store()?;
			while let Some(row) = rows.next().store()? {
				calls.push(crate::db::rows::read_call(row)?);
			}
		}

		let mut channels = Vec::new();
		{
			let mut stmt = conn
				.prepare(
					"SELECT c.* FROM channels c
					 JOIN sessions s ON s.id = c.session
					 WHERE s.run = ?1
					 ORDER BY c.id",
				)
				.store()?;
			let mut rows = stmt.query([self.run.0]).store()?;
			while let Some(row) = rows.next().store()? {
				channels.push(crate::db::rows::read_channel(row)?);
			}
		}
		for ch in &mut channels {
			let mut stmt = conn
				.prepare(
					"SELECT * FROM utterances WHERE channel = ?1 ORDER BY idx",
				)
				.store()?;
			let mut rows = stmt.query([ch.id.0]).store()?;
			while let Some(row) = rows.next().store()? {
				ch.transcript.push(crate::db::rows::read_utterance(row)?);
			}
		}

		let mut lessons = Vec::new();
		{
			let mut stmt =
				conn.prepare("SELECT * FROM lessons ORDER BY id").store()?;
			let mut rows = stmt.query([]).store()?;
			while let Some(row) = rows.next().store()? {
				lessons.push(crate::db::rows::read_lesson(row)?);
			}
		}

		Ok(Snapshot { run, tasks, sessions, calls, channels, lessons })
	}

	/// Copy the whole database to a file while it is in use. How a bench case
	/// keeps its artifact.
	pub fn save_copy(&self, to: &std::path::Path) -> Result<(), StoreError> {
		let conn = self.conn.lock().unwrap();
		crate::db::save_copy(&conn, to)?;
		Ok(())
	}
}
