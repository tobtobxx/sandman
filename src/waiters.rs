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
//! Defines: [`Waiters`].

use crate::domain::{SessionId, TaskId};

/// Who is blocked on what.
#[derive(Default)]
pub struct Waiters {
    inner: std::sync::Mutex<Registry>,
}

/// Private: nothing outside resolves a waiter directly.
#[derive(Default)]
struct Registry {
    _private: (),
}

impl Waiters {
    pub fn new() -> Self {
        unimplemented!()
    }

    /// Block until a Task completes or is cancelled, then return its answer as
    /// text.
    ///
    /// The caller is recorded so a cancellation of the caller's *own* Task can
    /// release it — otherwise a Worker whose Task was cancelled while it waited
    /// would never reach the check at the top of its turn.
    pub async fn wait(&self, _caller: SessionId, _task: TaskId) -> String {
        unimplemented!()
    }

    /// Give every waiter on a Task the same text, and forget them.
    ///
    /// Called when a Task completes, with its answer, and when one is cancelled,
    /// with the notice that stands in for a Result.
    pub fn resolve(&self, _task: TaskId, _text: &str) {
        unimplemented!()
    }

    /// Release every wait held by one Session, whatever it was waiting on.
    ///
    /// The self-unblock: a Worker whose own Task was cancelled while it sat
    /// blocked here never reaches the loop-top cancellation check. Its tool call
    /// returns, the loop resumes, reads the cancelled state, and ends without a
    /// Result.
    pub fn resolve_held_by(&self, _session: SessionId, _text: &str) {
        unimplemented!()
    }

    /// Whether anyone is waiting at all. A wind-down wants to know.
    pub fn any(&self) -> bool {
        unimplemented!()
    }
}
