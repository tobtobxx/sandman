//! Task wait registry — Sessions hold a Turn for another Task's answer.
//!
//! Construct: `Harness::new` builds `Arc<Waiters>` as `harness.waiters`; threaded via `SessionCtx`.
//! Use: `wait(caller, task) -> String` blocks until resolved (returns at once if already `Resolved`);
//! `resolve(task, text)` wakes all on that Task and keeps the text; `resolve_held_by(session, text)`
//! wakes whatever that Session holds; `any() -> bool` for wind-down.
//! Consumers: `await_result::AwaitResult::call` via `wait`; `harness::complete_task` via `resolve`;
//! `harness::cancel_task` via `resolve` per chained Task + `resolve_held_by` for self-unblock.
//! Call trace: `turn → tools.run(await_result) → waiters.wait(caller, task) → harness.complete_task → waiters.resolve(task, answer) → oneshot → String → loop continues`.
//! Rules: **Resolved kept forever — already finished resolves at once.**
//! **read-and-register under one lock — no gap between pending check and registration.**
//! **receiver never awaited while lock held — register, drop guard, then await.**
//! **resolve_held_by is self-unblock — cancelled Worker's own wait released so the loop-top check runs; Tasks untouched.**
//! **one Task → many waiters; one Session → many waits.**
//! **cancel delivers its notice — no waiter left pending.**
//!
//! Defines: [`Waiters`].

use std::collections::HashMap;

use crate::domain::{SessionId, TaskId};

/// Registry of Tasks with blocked Sessions or kept outcomes.
#[derive(Default)]
pub struct Waiters {
	inner: std::sync::Mutex<Registry>,
}

/// Private registry; nothing outside resolves a waiter directly.
#[derive(Default)]
struct Registry {
	by_task: HashMap<TaskId, Status>,
}

/// One Task's outcome as seen by waiters.
enum Status {
	/// Not finished; waiters to notify on resolution.
	Pending(Vec<Entry>),
	/// Finished; kept so later callers resolve at once.
	Resolved(String),
}

/// One blocked Session and the channel that wakes it.
struct Entry {
	caller: SessionId,
	tx: tokio::sync::oneshot::Sender<String>,
}

impl Waiters {
	pub fn new() -> Self {
		Waiters::default()
	}

	/// Block until a Task resolves, then return its text.
	///
	/// Returns at once if already `Resolved`; otherwise registers and awaits.
	/// Caller recorded so cancel of the caller's own Task can self-unblock.
	pub async fn wait(&self, caller: SessionId, task: TaskId) -> String {
		enum Next {
			Ready(String),
			Registered(tokio::sync::oneshot::Receiver<String>),
		}

		// Register or return immediately
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

		// Await wakeup
		match next {
			Next::Ready(text) => text,
			Next::Registered(rx) => rx.await.unwrap_or_default(),
		}
	}

	/// Wake every waiter on a Task and keep the text for later callers.
	///
	/// Called on completion (answer) and on cancellation (notice).
	pub fn resolve(&self, task: TaskId, text: &str) {
		// Swap to resolved
		let entries = {
			let mut reg = self.inner.lock().unwrap();
			let previous =
				reg.by_task.insert(task, Status::Resolved(text.to_string()));
			match previous {
				Some(Status::Pending(entries)) => entries,
				_ => Vec::new(),
			}
		};
		// Wake waiters
		for entry in entries {
			let _ = entry.tx.send(text.to_string());
		}
	}

	/// Wake every wait held by one Session, whatever Task it waited on.
	///
	/// Self-unblock for a Worker cancelled while waiting; Tasks unchanged.
	pub fn resolve_held_by(&self, session: SessionId, text: &str) {
		// Collect held waits
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
		// Wake collected waiters
		for entry in released {
			let _ = entry.tx.send(text.to_string());
		}
	}

	/// Whether any waiter is still pending.
	pub fn any(&self) -> bool {
		// Check any pending
		let reg = self.inner.lock().unwrap();
		reg.by_task.values().any(
			|status| matches!(status, Status::Pending(entries) if !entries.is_empty()),
		)
	}
}
