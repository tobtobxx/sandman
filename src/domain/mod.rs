//! Vocabulary for Sandman — every definition, no logic.
//!
//! Names match `CONTEXT.md`; a concept not in the glossary is worth stopping
//! over. Sum types make invalid states unrepresentable — a `Completed` Task
//! always has a `Result`, a `Queued` call never has `cost`, `AssistantBody`
//! and `Reply` separate preamble from ending — so the rule is the type, not a
//! comment about which `Option`s are real.
//!
//! Construct: `Store` mints each id inside the inserting transaction via
//! `db::counters::take` (`Display`/`FromStr` as `<prefix>-nn`); `Title`/`Brief`/
//! `Day` via `TryFrom<String>` at the edge; `Timestamp`/`Cost` as epoch millis /
//! nano-USD; `NewTask`/`NewSession`/`NewCall`/`NewLesson` carry inserts — no id
//! without a row, no second way in.
//! Use: plain data threaded through `Store` and `SessionCtx` into
//! `session::turn(ctx, tier) → Turn`, `worker`/`comms`, `scheduler`, `model`,
//! `reflect` and `channels`; nothing here touches SQLite, holds an `Event`, or
//! decides a lifecycle — a Turn reports, its caller decides.
//! Consumers and how they match the same types differently:
//!
//! | Type | Store (only writer) | Harness / worker / comms | Scheduler / Model | memory / reflect |
//! | --- | --- | --- | --- | --- |
//! | `TaskState` | persists + mints | `Pending→Running→Completed\|Cancelled` drives Turns | — | — |
//! | `CallStatus` | persists `Queued→InFlight→Done\|Failed\|Dropped` | — | one `InFlight` at a time, `Tier` orders waiting | — |
//! | `SessionKind`/`Status` | persists transcript + reflections | `Worker` vs `Comms` policy never references the other | — | records against judged `Session` |
//! | `LessonSubject` | persists across Runs | — | — | searched by meaning across every `Run` |
//! | `AssistantBody`/`Reply` | — | `Text` triggers review, `Calls` loops | frozen `CallRequest` at queue time | `Outcome`/`Nudge` never mixed |
//!
//! Seam: domain is the data seam — traits (`Model`, `ToolRunner`, `Clock`,
//! `Embedder`) sit above it and exchange these types; every real/bench adapter
//! pairs below shares them.
//!
//! Rules: **no logic here — `store.rs` is the only writer.** **no `Option` where
//! a variant belongs — add a variant instead.** **checked once: `Title`/`Brief`/
//! `Day` never re-validated downstream.** **distinct id types — cross-entity
//! misuse does not compile.** **Spend is scoped to `Run`; lessons and past
//! `Task`s are searched across every `Run`.** **one `Task` concept — human
//! request, investigation and delegated work are the same type.**

pub mod call;
pub mod channel;
pub mod ids;
pub mod lesson;
pub mod message;
pub mod run;
pub mod session;
pub mod task;
pub mod text;
pub mod time;

pub use call::{CallRequest, CallStatus, LlmCall, NewCall, Usage};
pub use channel::{ChannelKind, ChannelRecord, Utterance, Who};
pub use ids::{CallId, ChannelId, IdError, LessonId, RunId, SessionId, TaskId};
pub use lesson::{Hit, Lesson, LessonSubject, NewLesson};
pub use message::{
	AssistantBody, Completion, Message, NonEmpty, Reply, ToolCall, ToolSchema,
};
pub use run::Run;
pub use session::{
	Incoming, IncomingFrom, NewSession, Nudge, Outcome, Reflection,
	ReflectionKind, ReflectionResult, Session, SessionKind, SessionStatus,
};
pub use task::{
	Creator, CronExpr, NewTask, Schedule, ScheduleError, Task, TaskPriority,
	TaskResult, TaskState, TaskStateName, TaskSummary,
};
pub use text::{Brief, Day, TextError, Title};
pub use time::{
	Clock, Cost, Duration, FixedClock, ManualClock, Spend, SystemClock,
	Timestamp,
};
