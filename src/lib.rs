//! Sandman: an agent swarm.
//!
//! Agents talk to a human, and complete or investigate work. They coordinate
//! through a shared queue rather than through a command hierarchy.
//!
//! The vocabulary is in `CONTEXT.md` and the shape of the thing is in
//! `ARCHITECTURE.md`. Read the first before the code: the words there are exact,
//! and this crate uses them.
//!
//! The bet: **nothing waits on a queue.** There is one kind of work — a Task —
//! and one kind of Worker. A Worker that needs an answer does not park and get
//! rebuilt; it holds inside a tool call and carries on where it stopped. Because
//! a Worker starts fresh and sees only its Brief, every piece of context has to
//! be written down by whoever created the Task.
//!
//! Where to start reading:
//!
//! - [`domain`] — every definition, and no logic. The states this system cannot
//!   be in are the ones you cannot write down.
//! - [`store`] — all the state, behind one domain-shaped vocabulary. Emits
//!   [`event::Event`], which is the only trace there is.
//! - [`harness`] — the Task lifecycle, and the loops that start work.
//! - [`session`] — the Turn, which both shapes of Session run and which decides
//!   nothing.
//! - [`worker`] and [`comms`] — one policy each, on top of that Turn.
//! - [`reflect`] — metacognition, which is harness machinery and not an agent.
//! - [`bench`] — a Sandman under test, with four seams to make unreal.
//!
//! Four traits are the seams worth knowing: [`model::Model`],
//! [`tools::ToolRunner`], [`domain::Clock`] and [`memory::Embedder`]. Each has a
//! real adapter and a bench adapter, which is what makes them seams rather than
//! hypothetical ones.

pub mod bench;
pub mod channels;
pub mod comms;
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
