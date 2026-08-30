//! `await_result`: how one Session holds for another's answer.
//!
//! A Worker that wants a child's answer does not park and get re-created. It
//! blocks inside the tool call, and the answer comes back as that call's result,
//! in the same turn. The link is held by the suspended call — here — rather than
//! by a field on the Task.
//!
//! Waking rather than re-running is what keeps continuity: the Session that
//! asked remembers why it asked. It also means a Session can hold several times,
//! once per answer it is owed.
//!
//! This is the subtlest concurrency in Sandman, which is why it is four methods
//! in a file of its own rather than three fields on the Harness. [`wait`]
//! registers under the lock, drops the guard, and only then awaits — a receiver
//! is never awaited while anything is held.
//!
//! A Task's outcome, once resolved, is kept rather than forgotten: that is what
//! lets a Task already finished resolve at once. A caller that reads "pending"
//! and a completion that writes "resolved" would otherwise race — whichever
//! ordering the caller tried, the other side could always slip in between. Doing
//! the read and the registration under the same lock is the only ordering that
//! has no gap for that to happen in.
//!
//! Defines: [`Waiters`].

use std::collections::HashMap;

use crate::domain::{SessionId, TaskId};

/// Who is blocked on what, and what has already been decided.
#[derive(Default)]
pub struct Waiters {
	inner: std::sync::Mutex<Registry>,
}

/// Private: nothing outside resolves a waiter directly.
#[derive(Default)]
struct Registry {
	by_task: HashMap<TaskId, Status>,
}

/// One Task's outcome, as far as `Waiters` has heard.
enum Status {
	/// Not finished. Whoever is blocked on it, waiting to be told.
	Pending(Vec<Entry>),
	/// Finished. Kept forever rather than delivered once, so a Task already
	/// finished by the time something asks still resolves at once.
	Resolved(String),
}

/// One Session, blocked in [`Waiters::wait`], and the channel that wakes it.
struct Entry {
	caller: SessionId,
	tx: tokio::sync::oneshot::Sender<String>,
}

impl Waiters {
	pub fn new() -> Self {
		Waiters::default()
	}

	/// Block until a Task completes or is cancelled, then return its answer as
	/// text.
	///
	/// The caller is recorded so a cancellation of the caller's *own* Task can
	/// release it — otherwise a Worker whose Task was cancelled while it waited
	/// would never reach the check at the top of its turn.
	pub async fn wait(&self, caller: SessionId, task: TaskId) -> String {
		enum Next {
			Ready(String),
			Registered(tokio::sync::oneshot::Receiver<String>),
		}

		let next = {
			let mut reg = self.inner.lock().unwrap();
			match reg
				.by_task
				.entry(task)
				.or_insert_with(|| Status::Pending(Vec::new()))
			{
				Status::Resolved(text) => Next::Ready(text.clone()),
				Status::Pending(entries) => {
					let (tx, rx) = tokio::sync::oneshot::channel();
					entries.push(Entry { caller, tx });
					Next::Registered(rx)
				},
			}
		};

		match next {
			Next::Ready(text) => text,
			Next::Registered(rx) => rx.await.unwrap_or_default(),
		}
	}

	/// Give every waiter on a Task the same text, and keep it for whoever asks
	/// later.
	///
	/// Called when a Task completes, with its answer, and when one is cancelled,
	/// with the notice that stands in for a Result.
	pub fn resolve(&self, task: TaskId, text: &str) {
		let entries = {
			let mut reg = self.inner.lock().unwrap();
			let previous =
				reg.by_task.insert(task, Status::Resolved(text.to_string()));
			match previous {
				Some(Status::Pending(entries)) => entries,
				_ => Vec::new(),
			}
		};
		for entry in entries {
			let _ = entry.tx.send(text.to_string());
		}
	}

	/// Release every wait held by one Session, whatever it was waiting on.
	///
	/// The self-unblock: a Worker whose own Task was cancelled while it sat
	/// blocked here never reaches the loop-top cancellation check. Its tool call
	/// returns, the loop resumes, reads the cancelled state, and ends without a
	/// Result.
	///
	/// This only ever releases the caller's own entries; the Tasks it was
	/// waiting on stay exactly as pending or resolved as they were.
	pub fn resolve_held_by(&self, session: SessionId, text: &str) {
		let mut released = Vec::new();
		{
			let mut reg = self.inner.lock().unwrap();
			for status in reg.by_task.values_mut() {
				let Status::Pending(entries) = status else {
					continue;
				};
				let mut i = 0;
				while i < entries.len() {
					if entries[i].caller == session {
						released.push(entries.remove(i));
					} else {
						i += 1;
					}
				}
			}
		}
		for entry in released {
			let _ = entry.tx.send(text.to_string());
		}
	}

	/// Whether anyone is waiting at all. A wind-down wants to know.
	pub fn any(&self) -> bool {
		let reg = self.inner.lock().unwrap();
		reg.by_task.values().any(
			|status| matches!(status, Status::Pending(entries) if !entries.is_empty()),
		)
	}
}
