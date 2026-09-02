//! Sandman: one queue, one database, one Event stream, two Session shapes.
//!
//! The bet: **nothing waits on a queue.** One Task kind, one Worker kind. A
//! Worker needing another's answer holds inside `await_result`; the answer
//! returns as that call's result, same Turn. A Comms Session never re-runs —
//! it subscribes and gets mail. Workers start fresh and see only the Brief, so
//! every piece of context must be written down by whoever created the Task.
//!
//! Vocabulary in `CONTEXT.md` is exact; `ARCHITECTURE.md` is the shape.
//!
//! Construct: `Config::load(path)` once at start (`default-config.toml` is the
//! doc); `Store::open(Backing, Events, model, now)` mints `Run` and recovers
//! leftovers; `Harness::new(store, events, scheduler, tools, clock, embedder,
//! config)` → `Arc<Harness>`; `harness.attach(channel)` opens `Channel` +
//! standing Comms Session; `harness.ctx(SessionId)` builds `SessionCtx` passed
//! down every layer into `session::turn` and every tool.
//! Use: `Harness::run(drive)` / `run_until_idle` → `step` → `drive_comms` |
//! `drive_worker` → `session::turn(ctx, tier) → Turn` → `worker`/`comms`
//! decide; `store.create_task(NewTask, now)` enqueues; `reflect` interrupts
//! inside the turn, fail-open.
//! Consumers — **a Turn decides nothing**, policy lives above it:
//!
//! | `Turn` | `worker::work_turn` | `comms::respond` |
//! | --- | --- | --- |
//! | `Text` | Review writes Result | said to human |
//! | `Silent` | Review; nothing to say loops | legitimate end |
//! | `Unreachable` | Task failed without Review | idle |
//! | `Cancelled` | ends, no Result | unreachable: no Task |
//!
//! Call trace:
//! ```text
//! Harness::run → step
//!   ├ drive_comms(channel) → comms::respond → session::turn(ctx, Comms)
//!   │     └ session::tell (mail → next turn)
//!   └ drive_worker(session) → worker::work_turn → session::turn(ctx, Tier)
//!         └ reflect::interrupt (top of loop) / reflect::reflect (Worker end)
//! ```
//!
//! Seams — each real + bench adapter, `Model` sits under `Scheduler` so bench
//! still exercises queue, Tier ordering and one-call-at-a-time:
//!
//! | Trait | Real | Bench | Note |
//! | --- | --- | --- |
//! | `Model` | OpenRouter | scripted replies | one `Models` adapter per spec |
//! | `ToolRunner` | registry | recorder + closure | `web_search`/`fetch` need no HTTP seam |
//! | `Clock` | system | fixed/manual | |
//! | `Embedder` | service | stub | lazy index, first search batches |
//!
//! Rules: **nothing but `store.rs` touches the database — no method mutates
//! without emitting an `Event`.** **worker and comms never reference each
//! other.** **one model call in flight; `Tier` orders waiting, never aborts
//! in-flight.** **one Task concept; one Comms Session per Channel; Brief is
//! the parent/child boundary.** **Spend re-summed on read, never
//! accumulated.**

// `matrix-sdk`'s encrypted sync is a future deep enough that proving it `Send`
// runs past the default limit.
#![recursion_limit = "256"]

pub mod bench;
pub mod channels;
pub mod comms;
pub mod config;
pub mod control;
pub mod db;
pub mod domain;
pub mod event;
pub mod harness;
pub mod log;
pub mod memory;
pub mod model;
pub mod prompts;
pub mod reflect;
pub mod roles;
pub mod scheduler;
pub mod session;
pub mod store;
pub mod tools;
pub mod waiters;
pub mod web;
pub mod worker;
