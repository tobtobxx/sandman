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

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};

use crate::domain::IncomingFrom;
use crate::harness::Harness;

use super::wire::Frame;

/// What every request handler reaches.
#[derive(Clone)]
pub struct AppState {
	pub harness: Arc<Harness>,
	/// The browser's own Channel, if it has one. `None` when
	/// `[channels].web` is off: the UI is still served and still watches
	/// everything, and only its chat window has nowhere to send.
	pub channel: Option<crate::domain::ChannelId>,
}

/// How many hits a Lessons search returns.
const FIND_LIMIT: usize = 10;

/// One request from a browser. `say` is the human talking; `find` is the
/// Lessons search box. There is nowhere else a browser writes.
#[derive(serde::Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum ClientMessage {
	Say { text: String },
	Find { query: String },
}

/// Start the Watcher UI where `[sandman]` says.
///
/// Serves `web/` as static files, and upgrades `/ws` to a socket that gets one
/// `init` and then a patch per Event. `/chat` is the same page as `/` — one
/// `index.html`, which picks its layout from the path — so a chat window can
/// be its own link without a second page to keep in step.
pub async fn serve(
	state: AppState,
	address: std::net::IpAddr,
	port: u16,
) -> std::io::Result<()> {
	let index = tower_http::services::ServeFile::new("web/index.html");
	let app = Router::new()
		.route("/ws", get(ws_handler))
		.route_service("/chat", index)
		.fallback_service(tower_http::services::ServeDir::new("web"))
		.with_state(state);

	let listener = tokio::net::TcpListener::bind((address, port)).await?;
	axum::serve(listener, app).await
}

async fn ws_handler(
	ws: WebSocketUpgrade,
	State(state): State<AppState>,
) -> impl IntoResponse {
	ws.on_upgrade(move |socket| watch(state, socket))
}

/// One browser: send the snapshot, then follow the stream.
///
/// Subscribed before the snapshot is taken, so an Event landing in the gap is
/// merely sent twice — once inside `init`, once as a Patch — rather than lost.
/// A consumer that falls behind loses Events per [`crate::event::Events`]; the
/// fix here is the same one a reconnect gets, a fresh `init`.
async fn watch(state: AppState, socket: WebSocket) {
	let mut events = state.harness.events.subscribe();
	let (mut tx, mut rx) = socket.split();

	let Some(first) = init(&state) else { return };
	if send(&mut tx, &first).await.is_err() {
		return;
	}

	loop {
		tokio::select! {
			event = events.recv() => {
				let frame = match event {
					Ok(event) => super::wire::patch_for(&state.harness.store, &event),
					Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
						init(&state)
					},
					Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
				};
				if let Some(frame) = frame {
					if send(&mut tx, &frame).await.is_err() {
						return;
					}
				}
			},
			// Axum answers a Ping with a Pong on its own, but still hands both
			// to the caller, along with any Binary or Close frame a client
			// sends — none of which is a browser talking, so only Text is
			// read, and only a closed or broken socket ends the watch.
			incoming = rx.next() => {
				let text = match incoming {
					Some(Ok(Message::Text(text))) => text,
					Some(Ok(_)) => continue,
					Some(Err(_)) | None => return,
				};
				let Ok(msg) = serde_json::from_str::<ClientMessage>(&text) else {
					continue;
				};
				match msg {
					ClientMessage::Say { text } => on_message(&state, &text).await,
					ClientMessage::Find { query } => {
						let frame = on_search(&state, &query).await;
						if send(&mut tx, &frame).await.is_err() {
							return;
						}
					},
				}
			},
		}
	}
}

/// The snapshot, as an `init` Frame. `None` only if the Store itself cannot
/// be read, which is not a state a Watcher can do anything about.
fn init(state: &AppState) -> Option<Frame> {
	let snapshot = state.harness.store.snapshot().ok()?;
	let spend = state.harness.spend().unwrap_or_default();
	Some(super::wire::init_frame(&snapshot, spend))
}

/// Put one Frame on the wire as a text message.
async fn send(
	tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
	frame: &Frame,
) -> Result<(), axum::Error> {
	let text = serde_json::to_string(frame).expect("a Frame always serializes");
	tx.send(Message::Text(text)).await
}

/// A message the human typed in the browser.
///
/// Dropped when there is no Channel to put it on. The browser already shows
/// that window as turned off, so this is the case of a socket that was open
/// before anyone read the configuration, not something a human is waiting on.
async fn on_message(state: &AppState, text: &str) {
	let Some(channel) = state.channel else {
		return;
	};
	state
		.harness
		.receive(channel, text, IncomingFrom::Human)
		.await;
}

/// Rank the Lessons for the search box, with the same call the `memory` Role's
/// tools make.
async fn on_search(state: &AppState, query: &str) -> Frame {
	let hits = match state.harness.store.all_lessons() {
		Ok(lessons) => {
			let corpus = crate::memory::lesson_corpus(lessons);
			crate::memory::rank(
				&state.harness.store,
				state.harness.embedder.as_ref(),
				query,
				&corpus,
				FIND_LIMIT,
			)
			.await
			.unwrap_or_default()
		},
		Err(_) => Vec::new(),
	};

	Frame::Ranked {
		query: query.to_string(),
		hits: hits
			.into_iter()
			.map(|h| (h.item.id.to_string(), h.score))
			.collect(),
	}
}
