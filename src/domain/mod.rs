//! Every definition in Sandman. No logic lives here.
//!
//! The vocabulary is defined in `CONTEXT.md`, and these names match it. If you
//! need a concept that is not in the glossary, that is worth stopping over.
//!
//! The types here do one job beyond naming things: they make the states the
//! system cannot be in impossible to write down. Where the prototype carried a
//! record with several optional fields and a rule in a comment about which
//! combinations were real — a Task with a `result` only once `state` said
//! `completed`, a call with a `cost` only once it was `done` — this crate
//! carries a sum type and the rule is the type.
//!
//! Modules:
//!
//! - [`ids`] — one newtype per entity, minted by the Store
//! - [`text`] — Title, Brief and Day, checked once at the edge
//! - [`time`] — Timestamp, Duration, Cost, and the `Clock` seam
//! - [`run`] — one lifetime of Sandman, as the database records it
//! - [`task`] — the single unit of work
//! - [`session`] — a live agent context, and metacognition's record of it
//! - [`call`] — one exchange with the model
//! - [`channel`] — a two-way connection to a human
//! - [`lesson`] — what metacognition kept
//! - [`message`] — the conversation, and what a model call gives back

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
pub use message::{AssistantBody, Completion, Message, NonEmpty, Reply, ToolCall, ToolSchema};
pub use run::Run;
pub use session::{
    Incoming, IncomingFrom, NewSession, Nudge, Outcome, Reflection, ReflectionKind,
    ReflectionResult, Session, SessionKind, SessionStatus,
};
pub use task::{
    Creator, NewTask, Schedule, Task, TaskPriority, TaskResult, TaskState, TaskSummary,
};
pub use text::{Brief, Day, TextError, Title};
pub use time::{Clock, Cost, Duration, FixedClock, ManualClock, Spend, SystemClock, Timestamp};
