//! Rough smoke tests for the Store, against a fresh in-memory database: Runs,
//! Comms Sessions, messages, mail, Channels and Lessons. A rough "is this
//! broken" check, not a regression suite.
//!
//! Two need a file instead: a lock and a restart are each two Stores over one
//! database, and a fresh in-memory one has nothing to be left behind in.

use std::sync::Arc;

use sandman::db::{Backing, DbError, Lock};
use sandman::domain::{
	Brief, CallRequest, CallStatus, ChannelId, ChannelKind, Creator, Incoming,
	IncomingFrom, LessonSubject, Message, NewCall, NewLesson, NewSession,
	NewTask, Schedule, SessionKind, SessionStatus, Spend, TaskPriority,
	TaskState, Timestamp, Title, Utterance, Who,
};
use sandman::event::Events;
use sandman::roles::RoleName;
use sandman::scheduler::Tier;
use sandman::store::{Store, StoreError};

fn open() -> Store {
	Store::open(
		Backing::Memory,
		Arc::new(Events::new(16)),
		"test-model",
		Timestamp(0),
	)
	.expect("open a fresh in-memory store")
}

#[test]
fn run_lifecycle() {
	let store = open();
	let run = store.run();

	assert_eq!(store.spend(run).unwrap(), Spend::default());
	assert!(!store.calls_outstanding().unwrap());

	store.end_run(Timestamp(1000)).unwrap();

	let snapshot = store.snapshot().unwrap();
	assert_eq!(snapshot.run.id, run);
	assert_eq!(snapshot.run.ended_at, Some(Timestamp(1000)));
	assert!(snapshot.tasks.is_empty());
	assert!(snapshot.sessions.is_empty());
}

#[test]
fn comms_session_messages_and_mail() {
	let store = open();
	let kind =
		SessionKind::Comms { channel: ChannelId(1), mailbox: Vec::new() };
	let id = store
		.start_session(
			NewSession {
				kind,
				status: SessionStatus::Idle,
				messages: vec![Message::System {
					content: "you are a Comms Session".to_string(),
				}],
			},
			Timestamp(0),
		)
		.unwrap();

	assert_eq!(store.message_count(id).unwrap(), 1);
	let index = store
		.append_message(id, Message::User { content: "hello".to_string() })
		.unwrap();
	assert_eq!(index, 1);
	assert_eq!(store.messages(id).unwrap().len(), 2);

	assert!(!store.has_mail(id).unwrap());
	store
		.receive_mail(
			id,
			Incoming {
				from: IncomingFrom::Human,
				text: "hi there".to_string(),
				at: Timestamp(5),
			},
		)
		.unwrap();
	assert!(store.has_mail(id).unwrap());

	let taken = store.take_mail(id).unwrap();
	assert_eq!(taken.len(), 1);
	assert_eq!(taken[0].text, "hi there");
	assert!(!store.has_mail(id).unwrap());

	let session = store.session(id).unwrap().unwrap();
	assert_eq!(session.messages.len(), 2);
	match session.kind {
		SessionKind::Comms { mailbox, .. } => assert!(mailbox.is_empty()),
		SessionKind::Worker { .. } => panic!("expected a Comms Session"),
	}
}

#[test]
fn channel_open_say_and_transcript() {
	let store = open();
	let session = store
		.start_session(
			NewSession {
				kind: SessionKind::Comms {
					channel: ChannelId(1),
					mailbox: Vec::new(),
				},
				status: SessionStatus::Idle,
				messages: Vec::new(),
			},
			Timestamp(0),
		)
		.unwrap();

	let channel = store.open_channel(ChannelKind::Scripted, session).unwrap();
	assert_eq!(store.channel_session(channel).unwrap(), Some(session));

	store
		.say(
			channel,
			Utterance {
				who: Who::Human,
				text: "hello".to_string(),
				at: Timestamp(1),
			},
		)
		.unwrap();
	store
		.say(
			channel,
			Utterance {
				who: Who::Sandman,
				text: "hi".to_string(),
				at: Timestamp(2),
			},
		)
		.unwrap();

	let transcript = store.transcript(channel).unwrap();
	assert_eq!(transcript.len(), 2);
	assert_eq!(transcript[0].text, "hello");
	assert_eq!(transcript[1].text, "hi");

	let channels = store.channels().unwrap();
	assert_eq!(channels.len(), 1);
	assert_eq!(channels[0].transcript.len(), 2);
}

/// Who gets the answer is read off who asked. A Comms Session subscribes the
/// Channel it stands on without asking for it; nobody else subscribes at all.
#[test]
fn a_comms_session_subscribes_the_task_it_creates() {
	let store = open();
	let (comms, channel) = store
		.open_comms(ChannelKind::Scripted, Vec::new(), Timestamp(0))
		.unwrap();
	let task = |by: Creator| NewTask {
		title: Title::try_from("find something out".to_string()).unwrap(),
		brief: Brief::try_from("the whole of it".to_string()).unwrap(),
		role: RoleName::Research,
		schedule: Schedule::Now,
		priority: TaskPriority::default(),
		created_by: by,
	};
	let subscriber_of = |by: Creator| {
		let id = store.create_task(task(by), Timestamp(0)).unwrap();
		store.task(id).unwrap().unwrap().subscriber
	};

	assert_eq!(subscriber_of(Creator::Session(comms)), Some(channel));
	assert_eq!(subscriber_of(Creator::Cli), None);
	assert_eq!(subscriber_of(Creator::Control), None);

	// A Worker stands on a Task rather than a Channel, so it resolves to
	// nobody. It waits for a child with `await_result` instead.
	let parent = store.create_task(task(Creator::Cli), Timestamp(0)).unwrap();
	let worker = store
		.start_session(
			NewSession {
				kind: SessionKind::Worker {
					task: parent,
					role: RoleName::Planning,
				},
				status: SessionStatus::Thinking,
				messages: Vec::new(),
			},
			Timestamp(0),
		)
		.unwrap();
	assert_eq!(subscriber_of(Creator::Session(worker)), None);
}

#[test]
fn lessons_and_vectors() {
	let store = open();
	let session = store
		.start_session(
			NewSession {
				kind: SessionKind::Comms {
					channel: ChannelId(1),
					mailbox: Vec::new(),
				},
				status: SessionStatus::Idle,
				messages: Vec::new(),
			},
			Timestamp(0),
		)
		.unwrap();

	let lesson_id = store
		.keep_lesson(
			NewLesson {
				text: "ask before assuming".to_string(),
				session,
				about: LessonSubject::Conversation { channel: ChannelId(1) },
			},
			Timestamp(10),
		)
		.unwrap();

	let lessons = store.all_lessons().unwrap();
	assert_eq!(lessons.len(), 1);
	assert_eq!(lessons[0].id, lesson_id);
	assert_eq!(lessons[0].text, "ask before assuming");

	assert_eq!(store.vector("lesson/l-01", "test-embed").unwrap(), None);
	store
		.put_vector("lesson/l-01", "test-embed", &[1.0, 2.0, 3.0])
		.unwrap();
	assert_eq!(
		store.vector("lesson/l-01", "test-embed").unwrap(),
		Some(vec![1.0, 2.0, 3.0])
	);
}

/// One Sandman per database. A second Store on a file a live one holds is
/// refused — this is what lets `recover` end everything it finds open without
/// asking whose it is.
#[test]
fn a_second_store_on_one_database_is_refused() {
	let path = std::env::temp_dir()
		.join(format!("sandman-lock-{}.sqlite", std::process::id()));
	let clean = || {
		for suffix in ["", "-wal", "-shm", ".lock"] {
			let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
		}
	};
	clean();
	let open = || {
		Store::open(
			Backing::File(path.clone()),
			Arc::new(Events::new(64)),
			"test-model",
			Timestamp(0),
		)
	};

	let first = open().expect("the first Store takes the lock");
	assert!(
		matches!(open(), Err(StoreError::Db(DbError::Locked { .. }))),
		"a second Store on a held database must be refused"
	);

	// Dropping releases it. A restart is exactly this: one process, then the
	// next.
	drop(first);
	let second = open().expect("the lock goes with the Store that held it");

	// What `--break-lock` does, and why it is a last resort: the lock is gone
	// whether or not anything was still using it.
	Lock::clear(&path).expect("breaking a lock");
	let third = open().expect("a broken lock lets the next start in");

	drop(second);
	drop(third);
	clean();
}

/// A Run that died leaves rows mid-flight. The next start ends every one of
/// them, and leaves the queue alone.
#[test]
fn a_restart_ends_what_a_dead_run_left_open() {
	let path = std::env::temp_dir()
		.join(format!("sandman-restart-{}.sqlite", std::process::id()));
	for suffix in ["", "-wal", "-shm", ".lock"] {
		let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
	}
	let open = |now| {
		Store::open(
			Backing::File(path.clone()),
			Arc::new(Events::new(64)),
			"test-model",
			now,
		)
		.expect("open a file-backed store")
	};
	let task = |title: &str| NewTask {
		title: Title::try_from(title.to_string()).unwrap(),
		brief: Brief::try_from("find something out".to_string()).unwrap(),
		role: RoleName::Research,
		schedule: Schedule::Now,
		priority: TaskPriority::default(),
		created_by: Creator::Cli,
	};

	// A Run that got as far as starting one Task and dying.
	let (pending, running, session, call) = {
		let store = open(Timestamp(0));
		let pending = store
			.create_task(task("waits its turn"), Timestamp(0))
			.unwrap();
		let running = store
			.create_task(task("was interrupted"), Timestamp(0))
			.unwrap();
		let session = store
			.start_session(
				NewSession {
					kind: SessionKind::Worker {
						task: running,
						role: RoleName::Research,
					},
					status: SessionStatus::Thinking,
					messages: Vec::new(),
				},
				Timestamp(1),
			)
			.unwrap();
		store.start_task(running, session, Timestamp(1)).unwrap();
		let call = store
			.queue_call(
				NewCall {
					session,
					tier: Tier::TaskNormal,
					model: "test-model".to_string(),
					request: CallRequest {
						messages: Vec::new(),
						tools: Vec::new(),
					},
				},
				Timestamp(1),
			)
			.unwrap();
		assert!(store.calls_outstanding().unwrap());
		(pending, running, session, call)
	};

	let store = open(Timestamp(100));

	assert_eq!(
		store.task(running).unwrap().unwrap().state,
		TaskState::Cancelled { at: Timestamp(100) }
	);
	let session = store.session(session).unwrap().unwrap();
	assert_eq!(session.status, SessionStatus::Cancelled);
	assert_eq!(session.ended_at, Some(Timestamp(100)));
	assert_eq!(
		store.call(call).unwrap().unwrap().status,
		CallStatus::Dropped { at: Timestamp(100) }
	);

	// The queue outlives the process it was written in.
	assert_eq!(
		store.task(pending).unwrap().unwrap().state,
		TaskState::Pending
	);
	assert_eq!(
		store.next_pending(Timestamp(100)).unwrap().unwrap().id,
		pending
	);

	drop(store);
	for suffix in ["", "-wal", "-shm", ".lock"] {
		let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
	}
}
