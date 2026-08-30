//! Serves the Watcher UI and holds its sockets open.
//!
//! Static files, one WebSocket per browser, and the Event stream turned into
//! frames. A Watcher that disconnects or falls behind costs the swarm nothing:
//! the stream is broadcast, and a browser that reconnects gets a fresh `init`.
//!
//! Two writes reach here, and both are deliberate exceptions to "a Watcher only
//! reads":
//!
//! - a message on the browser's own Channel, which is that human talking;
//! - a Lessons search, which is a read that costs an embedding call. It is
//!   ranked here rather than in the browser so that a score a human sees is the
//!   score a `memory` Worker would see — a ranking from a different embedding
//!   would not mean the same thing.
//!
//! Defines: [`serve`], [`AppState`].

use std::sync::Arc;

use crate::harness::Harness;

/// What every request handler reaches.
#[derive(Clone)]
pub struct AppState {
    pub harness: Arc<Harness>,
    pub embedder: Arc<dyn crate::memory::Embedder>,
    pub channel: crate::domain::ChannelId,
}

/// Start the Watcher UI on [`super::PORT`].
///
/// Serves `web/` as static files, and upgrades `/ws` to a socket that gets one
/// `init` and then a patch per Event.
pub async fn serve(_state: AppState, _port: u16) -> std::io::Result<()> {
    unimplemented!()
}

/// One browser: send the snapshot, then follow the stream.
async fn watch(_state: AppState, _socket: axum::extract::ws::WebSocket) {
    unimplemented!()
}

/// A message the human typed in the browser.
async fn on_message(_state: &AppState, _text: &str) {
    unimplemented!()
}

/// Rank the Lessons for the search box, with the same call the `memory` Role's
/// tools make.
async fn on_search(_state: &AppState, _query: &str) -> super::wire::Frame {
    unimplemented!()
}
