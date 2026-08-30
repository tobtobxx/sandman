//! The Turn: model calls and tool calls, until the model replies with plain text.
//!
//! Both shapes of Session run this one loop. In the prototype a Session was one
//! class holding both its data and its loop; here the data is in the Store —
//! because its whole life has to be watchable while it happens, and a loop that
//! awaits cannot hold it — and the loop is a function over [`SessionCtx`].
//!
//! **A turn decides nothing.** It reports how it ended — text, silence, an
//! unreachable model, or a Task that was cancelled underneath it — and the caller
//! says what that means. This is the seam worth protecting: the two shapes of
//! Session differ by almost nothing else, and they once ran as two copies of one
//! loop until they quietly drifted apart. Ending policy belongs in `worker.rs`
//! or `comms.rs`, never here.
//!
//! The single exception is the metacognitive interrupt, which fires between two
//! model calls in this loop. It has to: a caller only ever sees turns that
//! finished, and a Worker grinding on tool calls never finishes one — which is
//! exactly the failure the interrupt exists to catch. The top of the loop is
//! where it goes, because there every tool call already has its result and a
//! pushed message cannot split the two.
//!
//! Defines: [`SessionCtx`], [`Turn`], [`turn`].

use std::sync::Arc;

use crate::domain::{Clock, SessionId};
use crate::event::Events;
use crate::harness::Harness;
use crate::scheduler::{Scheduler, Tier};
use crate::store::Store;
use crate::tools::ToolRunner;

/// What a running Session and its tools need to reach.
///
/// Everything here is an [`Arc`], and none of it is the Session's own state: a
/// Session owns nothing. The Harness is here because tools reach it — creating a
/// Task, waiting on one, messaging a human — and the reference is safe because
/// the Harness holds Session *ids*, never Sessions, so nothing is cyclic.
#[derive(Clone)]
pub struct SessionCtx {
    pub id: SessionId,
    pub store: Arc<Store>,
    pub events: Arc<Events>,
    pub scheduler: Arc<Scheduler>,
    pub tools: Arc<dyn ToolRunner>,
    pub clock: Arc<dyn Clock>,
    pub harness: Arc<Harness>,
}

/// How a turn ended.
///
/// None of these is a success or a failure on its own; reading that is the
/// caller's job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Turn {
    /// The model replied with plain text and called no tool.
    Text(String),
    /// The model replied with nothing at all.
    Silent,
    /// The model could not be reached.
    Unreachable(String),
    /// The Task this Session was working on was cancelled. The turn ends with no
    /// Result and nothing is reviewed.
    Cancelled,
}

/// One turn.
///
/// The tier is the caller's, because this one loop drives both shapes: a Worker
/// passes its Task's tier, a Comms Session passes [`Tier::Comms`]. Metacognition
/// runs its own calls through the scheduler directly and never has to ask.
pub async fn turn(_ctx: &SessionCtx, _tier: Tier) -> Turn {
    unimplemented!()
}

/// Put something in the context for the next turn to see.
///
/// The only way anything reaches a Session from outside: mail, a child's answer,
/// and the feedback metacognition wrote all arrive as one of these.
pub async fn tell(_ctx: &SessionCtx, _content: &str) {
    unimplemented!()
}

/// The interrupt, fired from the top of the loop.
///
/// Records what it found either way. An interrupt that found nothing wrong is
/// the normal outcome — and a run where none ever fired and a run where they all
/// passed would otherwise look identical from outside.
async fn check_in(_ctx: &SessionCtx) {
    unimplemented!()
}
