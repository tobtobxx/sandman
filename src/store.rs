//! SQLite as single source of truth, behind one vocabulary.
//!
//! What it is: the only writer to SQLite — vocabulary calls like `create_task`,
//! `start_session`, `queue_call` decide rows and emit one [`Event`].
//! Construct: `Store::open(backing, events, model, now) -> Store` takes
//! `db::Lock` (file) or none (memory), migrates via `db::open`, mints `Run`
//! via `counters::take` and calls `recover` before returning.
//! Use: vocabulary mutations each emit one `Event`; reads return owned values:
//! tasks `create/start/complete/cancel/fire_cron/next_pending/list`, sessions
//! `start/append/messages/reflect/mail`, calls `queue/set/spend`, channels
//! `open_comms/say/transcript`, lessons/vectors, `snapshot`/`save_copy`.
//! Consumers: `Harness` owns `Arc<Store>` and threads via `SessionCtx` into
//! `session::turn`, `worker::work_turn`, `comms::respond`, `tools`, `scheduler`,
//! `channels`, `web`, `bench`; nothing else imports `rusqlite`.
//! Seam: Store is the seam — private `std::sync::Mutex<Connection>`; every
//! mutation emits on [`Events`]; [`Snapshot`] seeds Watchers, `Event` stream
//! keeps them current (`log.rs`, `web::wire`, `bench::Rig`).
//! | Entity | Writers | Readers | Event |
//! | --- | --- | --- | --- |
//! | `Task` | `create/start/complete/cancel/fire_cron` | `task/list/next_pending` | `TaskCreated/TaskStateChanged/TaskReArmed` |
//! | `Session` | `start/set/end/append/record` | `session/messages/last_reflection` | `SessionStarted/StatusChanged/MessageAppended/ReflectionRecorded` |
//! | `LlmCall` | `queue/set/recover` | `call/spend/outstanding` | `CallQueued/CallStatusChanged` |
//! | `Channel` | `open_comms/open/say/receive` | `channels/transcript/has_mail` | `ChannelOpened/Said/MailReceived` |
//! Call trace: `open → Lock::take → db::open → counters::take(Run) → emit RunStarted → recover → emit cancels`
//! · `create_task → subscriber_of → counters::take → insert → emit TaskCreated`
//! · `snapshot → run/tasks/sessions/calls/channels/lessons`.
//! Rules: **only Store touches the database; private `std::sync::Mutex` so await inside does not compile.** **no mutation without an Event.** **no lock across an await (`&self` everywhere).** **transcript is rows keyed `(session, idx)`, not a blob.** **sum types are discriminant + JSON, queue scan is index lookup.** **ids from `counters` inside the transaction using them.** **Spend is re-summed, never accumulated.**
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

/// All of Sandman's state. Only writer to SQLite and sole emitter of `Event`s.
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

/// Store failure. `Db` for SQLite/JSON, `NoSuch` for missing row, `NotRunning` guards double-complete.
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

/// Convert SQLite/JSON errors into [`StoreError::Db`]. Keeps call sites as `.store()?`.
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

/// Run query that must return one row. Maps via `read`. Fails if missing.
fn read_required<T>(
	conn: &rusqlite::Connection,
	sql: &str,
	params: impl rusqlite::Params,
	read: fn(&Row<'_>) -> Result<T, DbError>,
) -> Result<T, StoreError> {
	Ok(conn.query_row(sql, params, |row| Ok(read(row))).store()??)
}

/// Run query that may return no row. Maps via `read`. Returns `None` if missing.
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

/// Resolve Channel awaiting Creator's Task. Reads `sessions.channel`. Returns `None` for non-Comms creators.
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

/// Insert one `Pending` Task, minting its id in the same transaction.
///
/// The row and the `Task` returned say the same thing, so the caller emits
/// `TaskCreated` without reading back. Takes `subscriber` rather than deriving
/// it: a daughter resolves through its `Creator::CronTask`, everyone else
/// derives from Creator.
fn insert_task(
	tx: &rusqlite::Transaction<'_>,
	run: RunId,
	new: NewTask,
	subscriber: Option<ChannelId>,
	now: Timestamp,
) -> Result<Task, StoreError> {
	// Prepare rows
	let id = TaskId(crate::db::counters::take(tx, TaskId::COUNTER)?);
	let state = TaskState::Pending;
	let state_row = crate::db::rows::task_state_to_row(&state)?;
	let schedule_row = crate::db::rows::schedule_to_row(&new.schedule)?;
	let priority_json = serde_json::to_string(&new.priority).store()?;
	let created_by_json = serde_json::to_string(&new.created_by).store()?;

	// Insert task
	tx.execute(
		"INSERT INTO tasks (
			id, run, title, brief, role, state, state_json,
			schedule, schedule_json, not_before, subscriber, priority,
			created_by, created_at
		) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
		rusqlite::params![
			id.0,
			run.0,
			new.title.as_str(),
			new.brief.as_str(),
			new.role.to_string(),
			state_row.tag,
			state_row.json,
			schedule_row.tag,
			schedule_row.json,
			new.schedule.not_before().map(|t| t.0),
			subscriber.map(|c| c.0),
			priority_json,
			created_by_json,
			now.0,
		],
	)
	.store()?;

	Ok(Task {
		id,
		run,
		title: new.title,
		brief: new.brief,
		role: new.role,
		state,
		schedule: new.schedule,
		subscriber,
		priority: new.priority,
		created_by: new.created_by,
		created_at: now,
	})
}

fn task_state_columns(row: &Row<'_>) -> Result<TaskState, DbError> {
	let tag: String = row.get("state")?;
	let json: String = row.get("state_json")?;
	crate::db::rows::task_state_from_row(&tag, &json)
}

/// Every entity for Watcher's first frame. Afterwards Watchers follow `Event` stream.
#[derive(Debug, Clone)]
pub struct Snapshot {
	pub run: Run,
	pub tasks: Vec<Task>,
	pub sessions: Vec<Session>,
	pub calls: Vec<LlmCall>,
	pub channels: Vec<ChannelRecord>,
	pub lessons: Vec<Lesson>,
}

/// Filter for `list_tasks`. Optional state and limit.
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
	pub state: Option<TaskStateName>,
	pub count: Option<usize>,
}

impl Store {
	/// Open Store and start a new Run. Takes file lock, migrates, mints Run, recovers leftovers. Emits `RunStarted`.
	pub fn open(
		backing: Backing,
		events: Arc<Events>,
		model: &str,
		now: Timestamp,
	) -> Result<Self, StoreError> {
		// Take lock
		let lock = match &backing {
			Backing::File(path) => Some(crate::db::Lock::take(path)?),
			Backing::Memory => None,
		};
		// Open and migrate
		let (mut conn, migration) = crate::db::open(backing)?;
		// Start new Run
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
		// Emit and recover
		store.events.emit(Event::RunStarted(Run {
			id: run,
			started_at: now,
			ended_at: None,
			model: model.to_string(),
		}));
		store.recover(now)?;
		Ok(store)
	}

	/// Cancel leftovers from previous Run. Cancels running Tasks, open Sessions, queued calls. Leaves pending Tasks.
	fn recover(&self, now: Timestamp) -> Result<(), StoreError> {
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;

		// Collect ids

		let ids = |sql: &str| -> Result<Vec<u32>, StoreError> {
			let mut stmt = tx.prepare(sql).store()?;
			let mut rows = stmt.query([]).store()?;
			let mut ids = Vec::new();
			while let Some(row) = rows.next().store()? {
				ids.push(row.get::<_, i64>(0).store()? as u32);
			}
			Ok(ids)
		};

		// Cancel running Tasks
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

		// Cancel open Sessions
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

		// Drop queued calls
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

		// Emit events
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

	/// Return the `Run` this Store is writing.
	pub fn run(&self) -> RunId {
		self.run
	}

	/// Return `Some((from, to))` if `open` migrated, else `None`.
	pub fn migration(&self) -> Option<(u32, u32)> {
		self.migration
	}

	/// Mark this Run finished at `now`. Reads back `Run` and emits `RunEnded`. Fails if missing.
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

	/// Enqueue a Task. Mints id, derives subscriber, computes `not_before`. Emits `TaskCreated`.
	pub fn create_task(
		&self,
		new: NewTask,
		now: Timestamp,
	) -> Result<TaskId, StoreError> {
		// Derive subscriber
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;
		let subscriber = subscriber_of(&tx, new.created_by)?;

		// Insert task
		let task = insert_task(&tx, self.run, new, subscriber, now)?;
		tx.commit().store()?;

		// Emit event
		let id = task.id;
		drop(conn);
		self.events.emit(Event::TaskCreated(task));
		Ok(id)
	}

	/// Copy a cron Task into a daughter due now and re-arm the cron Task.
	///
	/// The daughter carries everything but the schedule, names the cron Task
	/// as its `Creator::CronTask`, and inherits the subscriber, so its answer
	/// reaches whoever the cron Task's would have. The cron Task stays
	/// `Pending` throughout. `Ok(None)` means the expression has no
	/// occurrence left, and the cron Task was cancelled rather than left due
	/// forever. Emits `TaskCreated` plus `TaskReArmed`, or `TaskStateChanged`
	/// when it runs out.
	pub fn fire_cron(
		&self,
		cron: &Task,
		now: Timestamp,
	) -> Result<Option<TaskId>, StoreError> {
		// Arm the next occurrence, or retire the Task
		let mut conn = self.conn.lock().unwrap();
		let Some(schedule) = cron.schedule.re_armed(now) else {
			let state = TaskState::Cancelled { at: now };
			let row = crate::db::rows::task_state_to_row(&state)?;
			conn.execute(
				"UPDATE tasks SET state = ?1, state_json = ?2 WHERE id = ?3",
				rusqlite::params![row.tag, row.json, cron.id.0],
			)
			.store()?;
			drop(conn);
			self.events
				.emit(Event::TaskStateChanged { task: cron.id, to: state });
			return Ok(None);
		};

		// Create the daughter and re-arm in one transaction
		let tx = conn.transaction().store()?;
		let daughter = insert_task(
			&tx,
			self.run,
			NewTask {
				title: cron.title.clone(),
				brief: cron.brief.clone(),
				role: cron.role,
				schedule: Schedule::Now,
				priority: cron.priority,
				created_by: Creator::CronTask(cron.id),
			},
			cron.subscriber,
			now,
		)?;
		let schedule_row = crate::db::rows::schedule_to_row(&schedule)?;
		tx.execute(
			"UPDATE tasks SET schedule = ?1, schedule_json = ?2,
			                  not_before = ?3 WHERE id = ?4",
			rusqlite::params![
				schedule_row.tag,
				schedule_row.json,
				schedule.not_before().map(|t| t.0),
				cron.id.0,
			],
		)
		.store()?;
		tx.commit().store()?;

		// Emit events
		let id = daughter.id;
		drop(conn);
		self.events.emit(Event::TaskCreated(daughter));
		self.events
			.emit(Event::TaskReArmed { task: cron.id, to: schedule });
		Ok(Some(id))
	}

	/// Move Task to `Running` for Session. Emits `TaskStateChanged`. Fails if missing.
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

	/// Complete a `Running` Task with `TaskResult`. Emits `TaskStateChanged`. Fails if not `Running`.
	pub fn complete_task(
		&self,
		id: TaskId,
		result: TaskResult,
		now: Timestamp,
	) -> Result<(), StoreError> {
		// Check state
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

		// Update state
		let new_state = TaskState::Completed { result, at: now };
		let row = crate::db::rows::task_state_to_row(&new_state)?;
		conn.execute(
			"UPDATE tasks SET state = ?1, state_json = ?2 WHERE id = ?3",
			rusqlite::params![row.tag, row.json, id.0],
		)
		.store()?;
		drop(conn);
		// Emit event
		self.events
			.emit(Event::TaskStateChanged { task: id, to: new_state });
		Ok(())
	}

	/// Cancel Tasks. Skips missing/terminal, emits per cancel. Terminal — no `Result`, ends repeating chain.
	pub fn cancel_tasks(
		&self,
		ids: &[TaskId],
		now: Timestamp,
	) -> Result<(), StoreError> {
		// Take lock
		let conn = self.conn.lock().unwrap();
		let mut cancelled = Vec::new();
		// Cancel each
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
		// Emit events
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

	/// Read `TaskState` by id. Fast path for cancellation check. Returns `None` if missing.
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

	/// Return first `Pending` Task whose `not_before` has passed. Picks by time only; blocking is via `await_result`.
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

	/// Return duration until earliest `Pending` Task is due. `None` if empty.
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

	pub fn list_tasks(
		&self,
		filter: ListFilter,
	) -> Result<Vec<TaskSummary>, StoreError> {
		// Take lock
		let conn = self.conn.lock().unwrap();

		// Build query
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

		// Query tasks
		let mut stmt = conn.prepare(&sql).store()?;
		let mut rows = stmt
			.query(rusqlite::params_from_iter(
				params.iter().map(|p| p.as_ref()),
			))
			.store()?;

		// Collect summaries
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

	/// List Tasks for Run ordered by id. For one-shot output.
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

	/// List every Task across Runs ordered by id. For semantic search.
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
		// Take lock and mint id
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

		// Insert session
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

		// Insert messages
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

		// Emit event
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

	/// Append `Message` to Session transcript. Returns new index. Emits `MessageAppended`.
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

	/// Read all `Message`s for Session ordered by idx.
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

	/// Count `Message`s for Session. For interrupt threshold.
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

	/// Read latest `Reflection` for Session, if any. For interrupt baseline.
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

	/// Find Session that ran Task, if any. Bridges `search_tasks` hit to conversation.
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

	/// Enqueue `Incoming` in Comms mailbox. Emits `MailReceived`.
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

	/// Drain unread `Incoming` from mailbox and mark read atomically. Mid-turn post waits.
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

	/// Queue a new `LlmCall` as `Queued`. Emits `CallQueued`. Waiting visible as working.
	pub fn queue_call(
		&self,
		new: NewCall,
		now: Timestamp,
	) -> Result<CallId, StoreError> {
		// Mint id
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;
		let id = CallId(crate::db::counters::take(&tx, CallId::COUNTER)?);

		let status = CallStatus::Queued;
		let status_row = crate::db::rows::call_status_to_row(&status)?;
		let request_json = serde_json::to_string(&new.request).store()?;

		// Insert call
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

		// Emit event
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
		let usage = match &status {
			CallStatus::Done { usage, .. } => Some(usage),
			_ => None,
		};
		let updated = conn
			.execute(
				"UPDATE calls SET status = ?1, status_json = ?2, cached = ?3,
				 prefill = ?4, produced = ?5, cost = ?6 WHERE id = ?7",
				rusqlite::params![
					row.tag,
					row.json,
					usage.map(|u| u.cached as i64),
					usage.map(|u| u.prefill as i64),
					usage.map(|u| u.produced as i64),
					usage.map(|u| u.cost.0),
					id.0
				],
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

	/// Sum `Spend` for Run from `Done` calls. Re-summed, not accumulated.
	///
	/// Tokens are the ones computed — `prefill + produced`. A cache hit costs
	/// nothing to process, so counting it here would flatter the total.
	pub fn spend(&self, run: RunId) -> Result<Spend, StoreError> {
		let conn = self.conn.lock().unwrap();
		let (calls, tokens, cost): (i64, Option<i64>, Option<i64>) = conn
			.query_row(
				"SELECT COUNT(*), SUM(prefill + produced), SUM(cost) FROM calls
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

	/// Check if any calls are `Queued` or `InFlight` for this Run. For wind-down.
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

	/// Open Channel and Comms Session atomically. Mints both ids in one transaction. Emits `SessionStarted` and `ChannelOpened`.
	pub fn open_comms(
		&self,
		kind: ChannelKind,
		messages: Vec<Message>,
		now: Timestamp,
	) -> Result<(SessionId, ChannelId), StoreError> {
		// Mint ids
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

		// Insert session
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

		// Insert messages
		for (idx, message) in messages.iter().enumerate() {
			let row = crate::db::rows::message_to_row(message)?;
			tx.execute(
				"INSERT INTO messages (session, idx, role, body_json)
				 VALUES (?1,?2,?3,?4)",
				rusqlite::params![session_id.0, idx as i64, row.tag, row.json],
			)
			.store()?;
		}

		// Insert channel
		tx.execute(
			"INSERT INTO channels (id, kind, session) VALUES (?1,?2,?3)",
			rusqlite::params![channel_id.0, <&str>::from(kind), session_id.0],
		)
		.store()?;

		tx.commit().store()?;

		// Emit events
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
		// Take lock
		let conn = self.conn.lock().unwrap();
		let mut heads = Vec::new();
		// Load heads
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
		// Load transcripts
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

	/// Read Channel with full transcript by id. For Watcher patch.
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

	/// Read Comms Session for Channel, if any.
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
		// Mint id
		let mut conn = self.conn.lock().unwrap();
		let tx = conn.transaction().store()?;
		let id = LessonId(crate::db::counters::take(&tx, LessonId::COUNTER)?);
		let day = Day::today(now);
		let about_row = crate::db::rows::lesson_subject_to_row(&new.about)?;

		// Insert lesson
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

		// Emit event
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

	/// List every `Lesson` across Runs. For memory search.
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

	/// Read cached embedding for `key`/`model`. Returns `None` if missing.
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

	/// Load full `Snapshot` for Watcher first frame. Reads run/tasks/sessions/calls/channels/lessons.
	pub fn snapshot(&self) -> Result<Snapshot, StoreError> {
		let mut conn = self.conn.lock().unwrap();

		// Load run
		let run = read_required(
			&conn,
			"SELECT * FROM runs WHERE id = ?1",
			[self.run.0],
			crate::db::rows::read_run,
		)?;

		// Load tasks
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

		// Load sessions
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

		// Load calls
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

		// Load channels
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

		// Load lessons
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

	/// Copy database file via `VACUUM INTO`. For bench artifacts.
	pub fn save_copy(&self, to: &std::path::Path) -> Result<(), StoreError> {
		let conn = self.conn.lock().unwrap();
		crate::db::save_copy(&conn, to)?;
		Ok(())
	}
}
