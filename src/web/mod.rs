//! Watcher UI: read-only live view of the swarm, kept in step by the Event stream.
//!
//! A Watcher never decides and never writes — the swarm behaves the same
//! whether one is attached or not — except for two deliberate writes: a chat
//! message on the browser's own Channel and a Lessons search that costs an
//! embedding call.
//!
//! Construct: `AppState { harness: Arc<Harness>, channel: Option<ChannelId> }`;
//! `server::serve(state, addr, port)` binds axum, serves embedded `web/` and upgrades
//! `/ws`. Use: browser loads `/` or `/chat` (same `index.html`), opens `/ws`,
//! gets one `Frame::Init` (every entity from `Store::snapshot` plus `Spend` and
//! `Run`) then one `Frame` per [`crate::event::Event`] via `wire::patch_for`;
//! writes are `Say`/`Find`/`Cancel` JSON (`t: "say"|"find"|"cancel"`) handled by
//! `on_message`/`on_search`/`on_cancel`. Consumers: browser JS is the only external consumer;
//! internally `Harness` supplies `Events` + `Store`, `memory::rank` supplies
//! Lesson scores so the browser sees the same ranking a `memory` Worker would.
//! Seam: `Event` → `Frame` translation lives only in [`wire`]; [`server`] owns
//! transport, broadcast handling, and the two writes.
//!
//! | `Event` | `wire::patch_for` | `server::watch` on `Lagged` |
//! | --- | --- | --- |
//! | `Task`/`Session`/`Call`/`Channel`/`Lesson`/`MessageAppended` | `Patch`/`Appended` — whole entity re-read from `Store`, browser replaces outright | — |
//! | `RunStarted`/`RunEnded`/`MailReceived`/`ToolCalled`/`ToolReturned` | `None` — nothing a Watcher shows | — |
//! | broadcast `Lagged` | — | fresh `Init` — same as reconnect, no replay |
//!
//! Call trace: `serve → ws_handler → watch → init(snapshot) → send(Init)` then
//! `select! { events.recv → patch_for → send(Patch/Appended); rx.next → Say→receive | Find→rank→send(Ranked) }`
//! — subscribed before the snapshot so a gap Event is sent twice, never lost.
//! Rules: **one snapshot then patches; patch carries whole entity, never a delta.**
//! **broadcast is lossy — slow Watcher loses Events, never slows the swarm.**
//! **read-only except `Say` on own Channel and `Find` via same `memory::rank`.**
//! **reconnect gets fresh `Init`, never a replay.**
//! **nothing recomputed here — every wire field comes off the Store value.**
//!
//! Files: [`server`] sockets, static files, and the two writes; [`wire`] `Event`→`Frame` and `Bucket`.

pub mod server;
pub mod wire;
