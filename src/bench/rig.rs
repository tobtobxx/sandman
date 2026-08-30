//! One Sandman under test, and everything needed to drive and read it.
//!
//! A Rig is a whole Harness: a private in-memory database, its own Event stream,
//! its own scheduler, its own log file in a temporary directory that is removed
//! with it. Nothing in it is process-global, which is why a case is a test rather
//! than a process.
//!
//! What a case chooses is how much of it is real: real prompts, real model, real
//! clock, unless it says otherwise in its own first lines. The tools are the
//! exception — they are replaced in every case, because a bench is one Session's
//! decisions and the interceptor is what keeps it to one.
//!
//! Waiting is [`Rig::until`]: it follows the Event stream, re-checks the
//! predicate whenever something changed, and evaluates every tripwire on the way
//! past. A tripped wire comes back as [`Trip`], which a test propagates with `?`.
//!
//! Winding down is not optional and not the caller's problem to remember.
//! [`Rig::wind_down`] cancels everything unfinished and waits for the last
//! in-flight call so its cost reaches the record; `Drop` aborts the driver tasks
//! as a backstop, so a case that panics cannot leave a Harness spending.
//!
//! Defines: [`Rig`], [`RigBuilder`], [`ModelChoice`], [`ClockChoice`], [`Watch`].

use std::sync::Arc;

use super::{CheckResult, Trip};
use crate::domain::{
    ChannelId, Duration, NewLesson, NewTask, SessionId, Spend, Task, TaskId, Timestamp, Utterance,
};
use crate::harness::{Drive, Harness};
use crate::store::Store;

/// Where the model's answers come from.
pub enum ModelChoice {
    /// The real model, over the real wire. What a bench case measures.
    Real,
    /// Replies written by the test, in order. For exercising the Harness itself
    /// — the turn loop, the scheduler, the review — without spending anything.
    Scripted(Vec<crate::domain::Completion>),
    /// Anything else the test wants to supply.
    Custom(Arc<dyn crate::model::Model>),
}

/// Where time comes from.
pub enum ClockChoice {
    /// The real clock. What a case asserting on the model's judgement of time
    /// must use: faking it would bench a Sandman that does not exist.
    Real,
    /// Stopped, so every timestamp in a run is the same and comparable.
    Fixed(Timestamp),
    /// Only moves when the test moves it. For a case that needs a scheduled Task
    /// to actually fire — which is a case about the Harness, not the model, and
    /// should say so.
    Manual(Arc<crate::domain::ManualClock>),
}

/// Builds a Rig. Everything defaults to real except the tools, which default to
/// [`super::ToolsChoice::Deny`]: a case pays for what it asks for.
pub struct RigBuilder {
    model: ModelChoice,
    tools: super::ToolsChoice,
    clock: ClockChoice,
    drive: Drive,
    channels: Vec<crate::domain::ChannelKind>,
    log: crate::log::Verbosity,
}

/// One Sandman under test.
pub struct Rig {
    pub harness: Arc<Harness>,
    pub store: Arc<Store>,
    pub interceptor: Arc<super::Interceptor>,
    /// The Channel a case's script speaks on, if it opened one.
    pub channel: Option<ChannelId>,
    events: tokio::sync::broadcast::Receiver<crate::event::Event>,
    tripwires: Vec<Tripwire>,
    started_at: Timestamp,
    deadline: Option<Timestamp>,
    dir: tempfile::TempDir,
    drivers: Vec<tokio::task::JoinHandle<()>>,
}

/// A condition evaluated continuously: "this must never happen".
struct Tripwire {
    name: String,
    pred: Box<dyn Fn(&Watch) -> CheckResult + Send + Sync>,
}

/// What a tripwire may look at.
///
/// The Store alone is not enough for a unit bench: a case that answers
/// `create_task` itself leaves no row behind, so "a second Task spawning" can
/// only be seen in the calls.
pub struct Watch<'a> {
    pub store: &'a Store,
    pub calls: &'a [super::RecordedToolCall],
}

impl RigBuilder {
    pub fn model(self, _choice: ModelChoice) -> Self {
        unimplemented!()
    }

    pub fn tools(self, _choice: super::ToolsChoice) -> Self {
        unimplemented!()
    }

    pub fn clock(self, _choice: ClockChoice) -> Self {
        unimplemented!()
    }

    /// How much the Harness starts by itself. A case wants the least that gets
    /// its one Session running: [`Drive::CommsOnly`] for a Comms Session, and
    /// [`Drive::Full`] for a seeded Task, whose children the interceptor answers
    /// rather than lets run.
    pub fn drive(self, _drive: Drive) -> Self {
        unimplemented!()
    }

    /// Open a Channel a script can speak on.
    pub fn channel(self, _kind: crate::domain::ChannelKind) -> Self {
        unimplemented!()
    }

    /// The whole case must finish inside this, or it trips.
    pub fn timeout(self, _within: Duration) -> Self {
        unimplemented!()
    }

    pub async fn build(self) -> Result<Rig, Trip> {
        unimplemented!()
    }
}

impl Rig {
    pub fn builder() -> RigBuilder {
        unimplemented!()
    }

    // --- Filling the state --------------------------------------------------

    /// Put a Task on the queue as though the command line had.
    ///
    /// An ordinary Store write through the ordinary path: nothing here reaches
    /// around the Harness to plant a row.
    pub fn seed_task(&self, _new: NewTask) -> Result<TaskId, Trip> {
        unimplemented!()
    }

    /// Write a lesson, so a case can test what the `memory` Role finds without
    /// first making a swarm earn one.
    pub fn seed_lesson(&self, _new: NewLesson) -> Result<crate::domain::LessonId, Trip> {
        unimplemented!()
    }

    /// What the human on the bench Channel says.
    pub async fn send(&self, _text: &str) {
        unimplemented!()
    }

    // --- Driving ------------------------------------------------------------

    /// Add a condition that must hold for the whole run.
    pub fn tripwire(
        &mut self,
        _name: &str,
        _pred: impl Fn(&Watch) -> CheckResult + Send + Sync + 'static,
    ) {
        unimplemented!()
    }

    /// Wait until something is true.
    ///
    /// Follows the Event stream: the predicate is re-checked when something
    /// changed, and every tripwire with it. No polling, and no clock involved
    /// except the case's own bound.
    pub async fn until(
        &mut self,
        _what: &str,
        _pred: impl Fn(&Store) -> bool,
    ) -> Result<(), Trip> {
        unimplemented!()
    }

    /// Wait for a fixed span, still watching tripwires. For the rare case that
    /// has to let a Session keep going for a while.
    pub async fn idle_for(&mut self, _span: Duration) -> Result<(), Trip> {
        unimplemented!()
    }

    /// True when a Comms Session has finished answering: mail read, idle, and at
    /// least one model call made since `since_calls`.
    ///
    /// Take the count *before* sending, or it is true before the run has done
    /// anything.
    pub fn comms_idle(&self, _since_calls: usize) -> Result<bool, Trip> {
        unimplemented!()
    }

    // --- Reading ------------------------------------------------------------

    /// Every tool call any Session made, in order, with what it answered. The
    /// unit bench's whole point.
    pub fn tool_calls(&self) -> Vec<super::RecordedToolCall> {
        unimplemented!()
    }

    /// What the human on the bench Channel has seen and said.
    pub fn transcript(&self) -> Result<Vec<Utterance>, Trip> {
        unimplemented!()
    }

    pub fn tasks(&self) -> Result<Vec<Task>, Trip> {
        unimplemented!()
    }

    pub fn spend(&self) -> Result<Spend, Trip> {
        unimplemented!()
    }

    /// Model calls that never came back, with the wire error — how an expired key
    /// or a dead endpoint becomes visible without opening the log. The checks
    /// alone would only say "no reply".
    pub fn failed_calls(&self) -> Result<Vec<(crate::domain::CallId, SessionId, String)>, Trip> {
        unimplemented!()
    }

    // --- Ending -------------------------------------------------------------

    /// Stop everything and make sure nothing can still spend.
    ///
    /// Cancels every unfinished Task, then waits for the last in-flight call to
    /// land so the cost record stays honest.
    pub async fn wind_down(&mut self) {
        unimplemented!()
    }

    /// Write the artifacts: `result.json`, `sandman.log`, and `store.sqlite` —
    /// the whole database of the run, with every Session's transcript and every
    /// model call's request and reply, queryable afterwards with `sqlite3`.
    ///
    /// Takes the directory rather than assuming the working one, which is what
    /// lets several cases write artifacts from one process.
    pub fn save_to(&self, _dir: &std::path::Path) -> std::io::Result<()> {
        unimplemented!()
    }
}

impl Drop for Rig {
    /// The backstop. A case that panicked never reached `wind_down`, and a
    /// driver task left running would keep spending against a Harness nothing
    /// is watching.
    fn drop(&mut self) {
        unimplemented!()
    }
}
