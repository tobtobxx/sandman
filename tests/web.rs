//! The Watcher UI end to end: connect, get an `init`, say something, and see
//! the Events that follow arrive as frames.
//!
//! Regression coverage for a real bug: a browser's periodic WebSocket ping was
//! read by `watch` as "not text, give up", which silently ended the connection
//! after the first frame. A client that never writes anything (as this test's
//! listener does not) exercises the same path a real browser's pings do.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use sandman::bench::script::ScriptedModel;
use sandman::channels;
use sandman::config::Config;
use sandman::db::Backing;
use sandman::domain::{Clock, SystemClock};
use sandman::event::Events;
use sandman::harness::{Drive, Harness};
use sandman::memory::{Embedder, OpenRouterEmbedder};
use sandman::model::{Model, Models};
use sandman::scheduler::Scheduler;
use sandman::store::Store;
use sandman::tools::Registry;
use sandman::web::server::{AppState, serve};

const PORT: u16 = 18_080;

/// The shipped default, against a stubbed environment: this test needs a
/// configuration of the real shape and has no opinion about what is in it, and
/// reading the machine's would make it depend on whoever runs it.
fn config() -> Arc<Config> {
	Arc::new(
		Config::parse_with(sandman::config::DEFAULT, &|_| {
			Some("/nonexistent".to_string())
		})
		.expect("the shipped default parses"),
	)
}

#[tokio::test]
async fn watcher_streams_patches_after_say() {
	let config = config();
	let clock: Arc<dyn Clock> = Arc::new(SystemClock);
	let now = clock.now();
	let events = Arc::new(Events::new(1024));
	let store = Arc::new(
		Store::open(Backing::Memory, events.clone(), "scripted", now).unwrap(),
	);
	let model: Arc<dyn Model> =
		Arc::new(ScriptedModel::new(vec![ScriptedModel::saying("ok")]));
	let scheduler = Arc::new(Scheduler::new(
		Models::uniform(model),
		store.clone(),
		clock.clone(),
	));
	let tools = Arc::new(Registry::all(events.clone()));
	let embedder: Arc<dyn Embedder> =
		Arc::new(OpenRouterEmbedder::from_spec(&config.embedding));
	let harness = Harness::new(
		store.clone(),
		events.clone(),
		scheduler,
		tools,
		clock,
		embedder,
		config,
	);

	tokio::spawn({
		let harness = harness.clone();
		async move {
			let _ = harness.run(Drive::Full).await;
		}
	});

	let channel = channels::web::attach(harness.clone()).await.unwrap();
	let state = AppState { harness: harness.clone(), channel: Some(channel) };
	tokio::spawn(async move {
		let _ =
			serve(state, std::net::IpAddr::from([127, 0, 0, 1]), PORT).await;
	});
	// `serve` binds inside the spawned task; give it a moment before dialing.
	tokio::time::sleep(Duration::from_millis(200)).await;

	let (mut ws, _) =
		tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{PORT}/ws"))
			.await
			.expect("the Watcher UI is listening");

	let init = next_text(&mut ws).await;
	assert!(
		init.contains("\"type\":\"init\""),
		"first frame was: {init}"
	);

	// Exactly what killed `watch` before the fix: a non-Text frame on the
	// incoming side, same as any WebSocket client's periodic keepalive ping.
	ws.send(WsMessage::Ping(Vec::new())).await.unwrap();

	ws.send(WsMessage::Text(
		serde_json::json!({"t": "say", "text": "hello"}).to_string(),
	))
	.await
	.unwrap();

	let mut saw_patch = false;
	let mut saw_appended = false;
	for _ in 0..20 {
		let Some(text) = try_next_text(&mut ws).await else {
			break;
		};
		saw_patch |= text.contains("\"type\":\"patch\"");
		saw_appended |= text.contains("\"type\":\"appended\"");
		if saw_patch && saw_appended {
			break;
		}
	}

	assert!(saw_patch, "expected a Patch frame after saying something");
	assert!(
		saw_appended,
		"expected an Appended frame for the new message"
	);
}

async fn next_text(
	ws: &mut tokio_tungstenite::WebSocketStream<
		tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
	>,
) -> String {
	try_next_text(ws).await.expect("the socket stayed open")
}

async fn try_next_text(
	ws: &mut tokio_tungstenite::WebSocketStream<
		tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
	>,
) -> Option<String> {
	loop {
		let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
			.await
			.ok()??;
		match msg.ok()? {
			WsMessage::Text(text) => return Some(text),
			_ => continue,
		}
	}
}
