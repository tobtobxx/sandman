//! `sandman.log`: the sequence, which a view of current state cannot show.
//!
//! Two Tasks that both ran are indistinguishable in a snapshot, and the log is
//! where their order lives. It reads the [`crate::event::Event`] stream and
//! writes one line per Event, so nothing anywhere has to remember to log
//! anything.
//!
//! **The log is the index; the database is the content.** A line names what
//! happened and the id to look it up under — it does not carry a model's whole
//! reply, a Brief, or a recorded request. Those are rows now, and a log that
//! reprinted them would bury the sequence it exists to show. `--verbose`
//! restores the bodies for a session where that is what is wanted.
//!
//! The terminal shows only the conversation, so the two never interleave.
//!
//! Defines: [`Logger`], [`Verbosity`], [`banner`].

use std::io::Write;
use std::path::Path;

use strum::IntoDiscriminant;

use crate::domain::{AssistantBody, Message, ReflectionResult};
use crate::event::Event;

/// How much of an Event's content reaches the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Verbosity {
	/// One line per Event, bodies elided to a length and a count.
	#[default]
	Terse,
	/// Bodies written out whole. For a run being read closely.
	Verbose,
}

/// Writes the trace.
///
/// Takes the log's path rather than writing to the working directory, so several
/// Harnesses in one process — a bench running its cases — never write over each
/// other and nothing has to move `cwd` to keep them apart.
pub struct Logger {
	file: std::sync::Mutex<std::fs::File>,
	verbosity: Verbosity,
}

impl Logger {
	/// Open a log, truncating whatever was there.
	pub fn create(path: &Path, verbosity: Verbosity) -> std::io::Result<Self> {
		let file = std::fs::OpenOptions::new()
			.create(true)
			.write(true)
			.truncate(true)
			.open(path)?;
		Ok(Logger { file: std::sync::Mutex::new(file), verbosity })
	}

	/// Follow an Event stream until it ends. Spawned once, at startup.
	pub async fn follow(
		self: std::sync::Arc<Self>,
		events: &crate::event::Events,
	) {
		use tokio::sync::broadcast::error::RecvError;
		let mut rx = events.subscribe();
		loop {
			match rx.recv().await {
				Ok(event) => self.write(&event),
				Err(RecvError::Lagged(n)) => {
					self.note(
						"log",
						&format!("fell behind, dropped {n} event(s)"),
					);
				},
				Err(RecvError::Closed) => return,
			}
		}
	}

	/// Write one Event.
	pub fn write(&self, event: &Event) {
		let line = format!(
			"{} {:<7} {}",
			timestamp(),
			event.category(),
			self.render(event)
		);
		self.append(&line);
	}

	/// A line that is not an Event: a startup banner, a warning from the
	/// Harness, a note from the bench driver.
	pub fn note(&self, category: &str, text: &str) {
		let line = format!("{} {:<7} {}", timestamp(), category, text);
		self.append(&line);
	}

	fn append(&self, line: &str) {
		let mut file = self.file.lock().unwrap();
		let _ = writeln!(file, "{line}");
	}

	/// One Event's detail, past the timestamp and category every line already
	/// carries.
	fn render(&self, event: &Event) -> String {
		match event {
			Event::RunStarted(run) => {
				format!("{} started, model {}", run.id, run.model)
			},
			Event::RunEnded(run) => format!("{} ended", run.id),

			Event::TaskCreated(task) => format!(
				"{} created \"{}\", role {}, brief {}",
				task.id,
				task.title,
				task.role,
				self.body(task.brief.as_str())
			),
			Event::TaskStateChanged { task, to } => {
				format!("{task} -> {}", to.discriminant())
			},

			Event::SessionStarted(session) => {
				format!("{} started, {}", session.id, session.kind)
			},
			Event::SessionStatusChanged { session, to } => {
				format!("{session} -> {to}")
			},
			Event::MessageAppended { session, index, message } => format!(
				"{session} #{index} {} {}",
				message.describe(),
				self.body(message_text(message))
			),
			Event::ReflectionRecorded { session, reflection } => format!(
				"{session} {:?} after #{} {}",
				reflection.kind,
				reflection.after_message,
				self.body(reflection_text(&reflection.result))
			),
			Event::MailReceived { session, incoming } => format!(
				"{session} mail from {:?} {}",
				incoming.from,
				self.body(&incoming.text)
			),

			Event::CallQueued(call) => {
				format!(
					"{} queued, model {}, tier {:?}",
					call.id, call.model, call.tier
				)
			},
			Event::CallStatusChanged { call, to } => {
				format!("{call} -> {to}")
			},

			Event::ChannelOpened { channel, session } => {
				format!("{channel} opened for {session}")
			},
			Event::Said { channel, utterance } => format!(
				"{channel} {:?} {}",
				utterance.who,
				self.body(&utterance.text)
			),

			Event::LessonKept(lesson) => format!(
				"{} kept, about {} {}",
				lesson.id,
				lesson.about.describe(),
				self.body(&lesson.text)
			),

			Event::ToolCalled { session, name, args } => {
				format!(
					"{session} called {name} {}",
					self.body(&args.to_string())
				)
			},
			Event::ToolReturned { session, name, output } => {
				format!("{session} {name} returned {}", self.body(output))
			},
		}
	}

	/// One body of free text: elided in `Terse`, whole in `Verbose`.
	fn body(&self, text: &str) -> String {
		match self.verbosity {
			Verbosity::Terse => elide(text),
			Verbosity::Verbose => format!("{text:?}"),
		}
	}
}

/// The free text carried by one Message, whichever variant it is.
fn message_text(message: &Message) -> &str {
	match message {
		Message::System { content } | Message::User { content } => content,
		Message::Assistant { body, .. } => match body {
			AssistantBody::Text(text) => text,
			AssistantBody::Calls { preamble, .. } => {
				preamble.as_deref().unwrap_or("")
			},
		},
		Message::Tool { content, .. } => content,
	}
}

/// The free text carried by one metacognitive result, whichever way it went.
fn reflection_text(result: &ReflectionResult) -> &str {
	match result {
		ReflectionResult::Ran { content, .. } => content,
		ReflectionResult::FailedOpen { error } => error,
	}
}

/// The wall-clock instant a line was written, to the millisecond.
fn timestamp() -> String {
	chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

/// Shorten a body for a terse line: a length, and enough of the text to
/// recognise it by.
fn elide(text: &str) -> String {
	const SNIPPET: usize = 60;
	let len = text.chars().count();
	if len <= SNIPPET {
		format!("{text:?} ({len} chars)")
	} else {
		let snippet: String = text.chars().take(SNIPPET).collect();
		format!("{snippet:?}… ({len} chars)")
	}
}

/// The header a run opens with: when, which model, which database.
pub fn banner(what: &str) -> String {
	format!(
		"{} {what}",
		chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
	)
}
