//! One process lifetime, as the database records it.
//!
//! Construct at [`crate::store::Store::open`] — mints [`RunId`] via
//! `counters::take` in the same transaction that inserts `runs`, emits
//! `RunStarted`, then `recover` cancels stale `running` Tasks, open Sessions
//! and `queued`/`in_flight` Calls. Close at [`crate::store::Store::end_run`]
//! which stamps `ended_at` and emits `RunEnded`. Decoded by
//! [`crate::db::rows::read_run`] from `SELECT * FROM runs`.
//!
//! Use as the cost boundary: [`crate::store::Store::spend`], `tasks_of_run`
//! and `Snapshot.run` are scoped by `run`; everything else that benefits
//! from history (Lessons, `all_tasks`, `memory`) is not. Pending Tasks
//! survive `recover` as the queue for the next Run.
//!
//! Consumers:
//!
//! | Consumer | Reads | Writes | Emits |
//! | --- | --- | --- | --- |
//! | `Store::open` | — | `runs` row | `RunStarted` |
//! | `Store::end_run` | `runs` row | `ended_at` | `RunEnded` |
//! | `Store::spend` / `tasks_of_run` | `calls` / `tasks WHERE run` | — | — |
//! | `db::rows::read_run` | `runs` row → `Run` | — | — |
//! | Watchers / `Snapshot` | `Run` | — | first frame |
//!
//! Rules / asymmetry:
//!
//! - **Spend is scoped to a Run; Lessons and past Tasks are not.** Cross-Run
//!   search is what makes `memory` useful in the first minutes after start.
//! - **Pending Tasks survive a Run boundary; Running ones do not.** `recover`
//!   leaves `pending` for the next Run and cancels `running`.
//! - **`ended_at` is `Some` only on clean shutdown.** Live and killed Runs
//!   both read as `None` afterwards — indistinguishable, accepted.
//! - **One `Run` per `Store`.** `Store` holds `run: RunId`; every later insert
//!   stamps that id, so no row is unscoped.
//!
//! Defines: [`Run`]

use super::ids::RunId;
use super::time::Timestamp;

/// One process lifetime, as stored in `runs`.
///
/// Minted by [`crate::store::Store::open`], closed by
/// [`crate::store::Store::end_run`]. `ended_at` is `None` while running.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Run {
	pub id: RunId,
	pub started_at: Timestamp,
	/// Set on clean shutdown. `None` while running and after a killed process
	/// — indistinguishable, accepted.
	pub ended_at: Option<Timestamp>,
	/// Model this Run used. Lets comparisons across Runs name what they compare.
	pub model: String,
}
