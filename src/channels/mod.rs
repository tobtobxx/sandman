//! Channel adapters: one connection to a human each.
//!
//! An adapter converts inbound traffic into input for a Comms Session and sends
//! that Session's output back. The Comms Session does not know which transport
//! it sits on, and adding a transport must not change it.
//!
//! Files: [`stdio`] the terminal; [`web`] the browser.

pub mod stdio;
pub mod web;
