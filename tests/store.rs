//! Rough smoke tests for the Store, against a fresh in-memory database.
//!
//! Task and call round trips need `RoleName`/`Tier` (`roles.rs`,
//! `scheduler.rs`), still `unimplemented!()` this early in the build order —
//! see TASKS.md. These cover what does not: Runs, Comms Sessions, messages,
//! mail, Channels and Lessons. A rough "is this broken" check, not a
//! regression suite.

use std::sync::Arc;

use sandman::db::Backing;
use sandman::domain::{
	ChannelId, ChannelKind, Incoming, IncomingFrom, LessonSubject, Message,
	NewLesson, NewSession, SessionKind, SessionStatus, Spend, Timestamp,
	Utterance, Who,
};
use sandman::event::Events;
use sandman::store::Store;

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
