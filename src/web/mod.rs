//! The Watcher UI: serving it, and keeping it in step.
//!
//! A Watcher reads the Harness's state as it changes without taking part. It
//! never decides anything and never writes, so a swarm behaves the same whether
//! one is attached or not. The exceptions are exactly two: a message on the
//! browser's own Channel, and a search over the Lessons — a read, but one that
//! costs an embedding call.
//!
//! A browser gets one snapshot carrying everything, then a patch per
//! [`crate::event::Event`]. In the prototype the Watcher was kept in step by
//! stringifying every entity twice a second and comparing against a shadow;
//! with one ordered stream there is nothing to compare, and a change reaches
//! the browser when it happens rather than up to half a second later.
//!
//! Files: [`server`] the sockets and the static files; [`wire`] an Event as a
//! browser reads it.

pub mod server;
pub mod wire;

/// Where the Watcher listens.
pub const PORT: u16 = 8080;
