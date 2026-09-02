//! Append-only line trace of `Event` order — what a snapshot cannot show.
//!
//! Construct: `Logger::create(path, verbosity, echo)` truncates `path`; `path`
//! is per-Harness (config `log_path` or bench temp dir) so parallel Harnesses
//! never collide and no `cwd` move is needed.
//! Use: `Arc<Logger>::follow(&Events)` spawned once at startup — `subscribe`s,
//! `recv`s and `write`s one `timestamp category detail` line per `Event` under
//! a single `Mutex<File>`; `note` writes a non-Event line (banner, warnings);
//! `render` + `body` build the detail, `banner` builds the opening line.
//! Watch: `history` reads the file back for whoever joins late, `subscribe`
//! hands out every line written from then on — the Watcher UI's Logs tab is
//! the two put together.
//!
//! Consumers:
//! | Caller | Builds | Echo |
//! | --- | --- | --- |
//! | `bin/sandman::assemble` | `Logger::create(paths.log, Verbosity, Echo)` | `Quiet` if `stdio` channel owns terminal, `Stdout` otherwise |
//! | `bench/rig::build` | `Logger::create(temp/sandman.log, Verbosity, Quiet)` | always `Quiet` |
//!
//! `Event` → detail (`render`):
//! | `Event` | Detail after `timestamp category` |
//! | --- | --- |
//! | `RunStarted`/`RunEnded` | `run id` + model |
//! | `TaskCreated`/`TaskStateChanged`/`TaskReArmed` | `task id` + title/role/brief, new state or next occurrence |
//! | `SessionStarted`/`SessionStatusChanged`/`MessageAppended`/`ReflectionRecorded`/`MailReceived` | `session id` + kind/status/index/body |
//! | `CallQueued`/`CallStatusChanged` | `call id` + model/tier or new status |
//! | `ChannelOpened`/`Said` | `channel id` + session/utterance |
//! | `LessonKept` | `lesson id` + subject/body |
//! | `ToolCalled`/`ToolReturned` | `session id` + tool name/args/output |
//!
//! Seam `Verbosity`/`Echo`:
//! | Variant | `Terse` / `Quiet` | `Verbose` / `Stdout` |
//! | --- | --- | --- |
//! | `Verbosity` | `elide` to 60-char snippet + len | `format!("{text:?}")` whole |
//! | `Echo` | file only | also `println!` under same lock |
//!
//! Rules:
//! - **One line per `Event`, nothing else logs.** Nothing has to remember to log.
//! - **Log is index, database is content.** Bodies are lengths/counts in `Terse`, quoted whole in `Verbose`.
//! - **Terminal owns conversation.** `Quiet` keeps trace in file alone; `Stdout` only when no Channel owns terminal.
//! - **One `Mutex<File>` guards both file and stdout.** Prevents interleaved lines.
//! - **Broadcast is lossy, not blocking.** `Lagged(n)` is noted, never stalls the swarm.
//! - **The file is the history, the line stream is only the tail.** Nothing is kept
//!   in memory for a reader that is not there yet — `history` re-reads the file.
//!
//! Defines: [`Logger`], [`Verbosity`], [`Echo`], [`banner`].

use std::io::Write;
use std::path::{Path, PathBuf};

use strum::IntoDiscriminant;

use crate::domain::{AssistantBody, Message, ReflectionResult};
use crate::event::Event;

/// How much body text a log line carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Verbosity {
	/// One line per Event, bodies elided to length + snippet.
	#[default]
	Terse,
	/// Bodies written whole.
	Verbose,
}

/// Whether trace also goes to terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Echo {
	/// File alone; terminal shows conversation.
	#[default]
	Quiet,
	/// Also stdout; terminal has no Channel.
	Stdout,
}

/// Append-only trace writer.
///
/// Holds file, its path, verbosity, echo and the line stream late readers follow.
pub struct Logger {
	file: std::sync::Mutex<std::fs::File>,
	path: PathBuf,
	verbosity: Verbosity,
	echo: Echo,
	lines: tokio::sync::broadcast::Sender<String>,
}

/// How many lines a reader may fall behind before it loses some.
const BACKLOG: usize = 256;

impl Logger {
	/// Create log file at `path`, truncating existing content.
	///
	/// Creates file if missing. Fails if parent does not exist.
	pub fn create(
		path: &Path,
		verbosity: Verbosity,
		echo: Echo,
	) -> std::io::Result<Self> {
		let file = std::fs::OpenOptions::new()
			.create(true)
			.write(true)
			.truncate(true)
			.open(path)?;
		Ok(Logger {
			file: std::sync::Mutex::new(file),
			path: path.to_path_buf(),
			verbosity,
			echo,
			lines: tokio::sync::broadcast::channel(BACKLOG).0,
		})
	}

	/// Every line written so far, oldest first.
	///
	/// Reads the file back under the write lock, so no line is caught half
	/// written. Empty when the file cannot be read — a reader that joins late
	/// misses history, it does not fail.
	pub fn history(&self) -> Vec<String> {
		let _writing = self.file.lock().unwrap();
		std::fs::read_to_string(&self.path)
			.unwrap_or_default()
			.lines()
			.map(str::to_string)
			.collect()
	}

	/// Follow lines written from now on.
	///
	/// Lossy: a reader that falls behind by more than `BACKLOG` lines is told
	/// how many it lost, and nothing waits for it.
	pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<String> {
		self.lines.subscribe()
	}

	/// Follow Event stream until closed.
	///
	/// Subscribes and writes each Event; notes lag. Spawned once at startup.
	pub async fn follow(
		self: std::sync::Arc<Self>,
		events: &crate::event::Events,
	) {
		// Subscribe to stream
		use tokio::sync::broadcast::error::RecvError;
		let mut rx = events.subscribe();
		loop {
			match rx.recv().await {
				// Write event
				Ok(event) => self.write(&event),
				// Handle lag
				Err(RecvError::Lagged(n)) => {
					self.note(
						"log",
						&format!("fell behind, dropped {n} event(s)"),
					);
				},
				// Handle close
				Err(RecvError::Closed) => return,
			}
		}
	}

	/// Write one Event as timestamped line.
	///
	/// Formats `timestamp category detail` and appends.
	pub fn write(&self, event: &Event) {
		let line = format!(
			"{} {:<7} {}",
			timestamp(),
			event.category(),
			self.render(event)
		);
		self.append(&line);
	}

	/// Write non-Event line.
	///
	/// Formats `timestamp category text` and appends. Used for banners and warnings.
	pub fn note(&self, category: &str, text: &str) {
		let line = format!("{} {:<7} {}", timestamp(), category, text);
		self.append(&line);
	}

	fn append(&self, line: &str) {
		// Append to file
		let mut file = self.file.lock().unwrap();
		let _ = writeln!(file, "{line}");
		if self.echo == Echo::Stdout {
			// Mirror to stdout
			println!("{line}");
		}
		// Offer to followers
		let _ = self.lines.send(line.to_string());
	}

	/// Render detail after timestamp and category.
	///
	/// Maps each `Event` variant to id plus summary.
	fn render(&self, event: &Event) -> String {
		match event {
			// Render run
			Event::RunStarted(run) => {
				format!("{} started, model {}", run.id, run.model)
			},
			Event::RunEnded(run) => format!("{} ended", run.id),

			// Render task
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
			Event::TaskReArmed { task, to } => {
				format!("{task} re-armed, next {:?}", to.not_before())
			},

			// Render session
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

			// Render call
			Event::CallQueued(call) => {
				format!(
					"{} queued, model {}, tier {:?}",
					call.id, call.model, call.tier
				)
			},
			Event::CallStatusChanged { call, to } => {
				format!("{call} -> {to}")
			},

			// Render channel
			Event::ChannelOpened { channel, session } => {
				format!("{channel} opened for {session}")
			},
			Event::Said { channel, utterance } => format!(
				"{channel} {:?} {}",
				utterance.who,
				self.body(&utterance.text)
			),

			// Render lesson
			Event::LessonKept(lesson) => format!(
				"{} kept, about {} {}",
				lesson.id,
				lesson.about.describe(),
				self.body(&lesson.text)
			),

			// Render tool
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

	/// Format body per verbosity.
	///
	/// `Terse` elides to snippet, `Verbose` quotes whole.
	fn body(&self, text: &str) -> String {
		match self.verbosity {
			Verbosity::Terse => elide(text),
			Verbosity::Verbose => format!("{text:?}"),
		}
	}
}

/// Extract free text from a `Message`.
///
/// Returns system/user content, assistant text or preamble, or tool content.
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

/// Extract free text from a `ReflectionResult`.
///
/// Returns content on `Ran`, error on `FailedOpen`.
fn reflection_text(result: &ReflectionResult) -> &str {
	match result {
		ReflectionResult::Ran { content, .. } => content,
		ReflectionResult::FailedOpen { error } => error,
	}
}

/// Current wall-clock time to millisecond.
///
/// Formats as `HH:MM:SS.mmm`.
fn timestamp() -> String {
	chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

/// Elide text to snippet plus length.
///
/// Takes 60 chars then appends total char count.
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

/// Build opening banner line.
///
/// Prepends local date-time to `what`.
pub fn banner(what: &str) -> String {
	format!(
		"{} {what}",
		chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
	)
}
