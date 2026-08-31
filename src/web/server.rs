//! Transport for the Watcher UI: static files and one WebSocket per browser.
//!
//! Construct: `AppState { harness: Arc<Harness>, channel: Option<ChannelId> }` where
//! `None` means `[channels].web` is off — UI still watches, chat has nowhere to send;
//! `serve(state, addr, port)` binds axum, serves `web/` and upgrades `/ws`.
//! Use: browser loads `/` or `/chat` (same `index.html`) then `ws_handler → watch`
//! sends one `Frame::Init` then one `Frame` per [`crate::event::Event`]; inbound
//! `ClientMessage::Say | Find` are the only writes (`on_message → Harness::receive`,
//! `on_search → memory::rank`).
//! Consumers: browser JS is the only external consumer; internally `Harness` supplies
//! `Events` + `Store::snapshot`, `wire::{init_frame,patch_for}` supplies `Frame`s.
//! Seam: `wire` owns `Event` → `Frame`; `server` owns sockets, broadcast handling,
//! and the two writes — never ranking or patch translation.
//!
//! | `ClientMessage` | handler | effect |
//! | --- | --- | --- |
//! | `Say` | `on_message` | `Harness::receive` on own `Channel` — dropped if `None` |
//! | `Find` | `on_search` | `memory::rank` with same `Embedder` as `memory` tool |
//!
//! Call trace: `serve → ws_handler → watch → init → send(Init)` then
//! `select! { events.recv → patch_for → send; rx.next → Say | Find → send(Ranked) }`
//! — subscribed before snapshot so gap is duplicated, never lost.
//! Rules: **one `Init` then `Patch`/`Appended`; `Init` is fresh on every connect or `Lagged`.**
//! **broadcast is lossy — Watcher that falls behind loses `Event`s, never slows the swarm.**
//! **read-only except `Say` on own `Channel` and `Find` via `memory::rank`.**
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

/// State threaded into every handler.
///
/// Carries the `Harness` for `Store` + `Events` and the browser's own `Channel`.
/// `None` when `[channels].web` is off — UI still watches, chat is disabled.
#[derive(Clone)]
pub struct AppState {
	pub harness: Arc<Harness>,
	pub channel: Option<crate::domain::ChannelId>,
}

/// How many hits a Lessons search returns.
const FIND_LIMIT: usize = 10;

/// One write from a browser.
///
/// `Say` is the human talking on own `Channel`; `Find` is the Lessons search box.
/// No other browser write exists.
#[derive(serde::Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum ClientMessage {
	Say { text: String },
	Find { query: String },
}

/// Bind the Watcher UI.
///
/// Serves `web/` as static files and upgrades `/ws` to Watcher sockets.
/// `/chat` serves the same `index.html` as `/` so a chat window has its own link.
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

/// Upgrade a request to a Watcher socket.
///
/// Hands the socket to `watch` after the handshake.
async fn ws_handler(
	ws: WebSocketUpgrade,
	State(state): State<AppState>,
) -> impl IntoResponse {
	ws.on_upgrade(move |socket| watch(state, socket))
}

/// Drive one browser connection.
///
/// Sends `Init` then follows `Events` with `Patch`/`Appended` and answers `Say`/`Find`.
/// Returns when the socket closes or `Events` is closed; `Lagged` re-sends `Init`.
async fn watch(state: AppState, socket: WebSocket) {
	// Subscribe before snapshot
	let mut events = state.harness.events.subscribe();
	let (mut tx, mut rx) = socket.split();

	// Send snapshot
	let Some(first) = init(&state) else { return };
	if send(&mut tx, &first).await.is_err() {
		return;
	}

	// Follow events and input
	loop {
		tokio::select! {
			event = events.recv() => {
				// Map event to frame
				let frame = match event {
					// Event — patch for watcher
					Ok(event) => super::wire::patch_for(&state.harness.store, &event),
					// Lagged — resend fresh init
					Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
						init(&state)
					},
					// Closed — end watch
					Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
				};
				// Send frame
				if let Some(frame) = frame {
					if send(&mut tx, &frame).await.is_err() {
						return;
					}
				}
			},
			incoming = rx.next() => {
				// Read inbound text
				let text = match incoming {
					// Text — browser talking
					Some(Ok(Message::Text(text))) => text,
					// Non-text — ignore
					Some(Ok(_)) => continue,
					// Closed or broken — end watch
					Some(Err(_)) | None => return,
				};
				// Parse client message
				let Ok(msg) = serde_json::from_str::<ClientMessage>(&text) else {
					continue;
				};
				// Dispatch write
				match msg {
					// Say — enqueue on channel
					ClientMessage::Say { text } => on_message(&state, &text).await,
					// Find — rank and reply
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

/// Build the opening `Init` frame.
///
/// Reads `Store::snapshot` and `Harness::spend`. Returns `None` if the `Store`
/// cannot be read.
fn init(state: &AppState) -> Option<Frame> {
	let snapshot = state.harness.store.snapshot().ok()?;
	let spend = state.harness.spend().unwrap_or_default();
	Some(super::wire::init_frame(&snapshot, spend))
}

/// Send one `Frame` as a text message.
///
/// Serializes the `Frame` and writes it to the socket. Returns `Err` if the
/// send fails.
async fn send(
	tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
	frame: &Frame,
) -> Result<(), axum::Error> {
	let text = serde_json::to_string(frame).expect("a Frame always serializes");
	tx.send(Message::Text(text)).await
}

/// Enqueue a human message from the browser.
///
/// Drops the text when no `Channel` is configured. Records via `Harness::receive`.
async fn on_message(state: &AppState, text: &str) {
	let Some(channel) = state.channel else {
		return;
	};
	state
		.harness
		.receive(channel, text, IncomingFrom::Human)
		.await;
}

/// Rank `Lesson`s for a search query.
///
/// Uses the same `memory::rank` call as the `memory` tool so scores match.
/// Returns a `Ranked` frame with at most `FIND_LIMIT` hits.
async fn on_search(state: &AppState, query: &str) -> Frame {
	// Load lessons
	let hits = match state.harness.store.all_lessons() {
		Ok(lessons) => {
			// Rank by meaning
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

	// Build ranked frame
	Frame::Ranked {
		query: query.to_string(),
		hits: hits
			.into_iter()
			.map(|h| (h.item.id.to_string(), h.score))
			.collect(),
	}
}
