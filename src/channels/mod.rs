//! Channel adapters — one live human connection each.
//!
//! Each adapter bridges transport traffic ↔ Comms Session input without the
//! Session knowing its transport. **Adding a transport must not change `comms.rs`.**
//!
//! Construct: [`stdio::attach`] / [`web::attach`] register `Arc<dyn Channel>`
//! via `Harness::attach`; `Store` mints `ChannelId` and the adapter holds it
//! in its `OnceLock`.
//! Use: inbound `Harness::receive(id, text, from)` → `Store::receive_mail`
//! (Human also `Store::say` to the transcript); outbound `Channel::send` from
//! `Harness::forward_said` on `Event::Said`.
//! Consumers: `Harness` owns and drives adapters; `comms::respond` is
//! transport-agnostic and never imports `channels`.
//!
//! | Adapter | `Channel::send` | Inbound path |
//! |---|---|---|
//! | [`stdio`] | prints cyan to stdout | blocking stdin loop; `/quit` or EOF → `Harness::stop` |
//! | [`web`] | no-op — text already in `Store`, push carries it | browser → `Harness::receive` |
//!
//! Rules: one Comms Session per `Channel`; `Channel::send` is fire-and-forget,
//! delivery is the `Store`.

pub mod stdio;
pub mod web;
