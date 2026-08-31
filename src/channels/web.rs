//! The browser Channel — the Watcher's one write path.
//!
//! The Watcher UI is otherwise read-only. Outbound delivery is already done:
//! the Comms Session wrote `Store::say` to the Channel transcript and the
//! `Event::Said` push carries it, so `Web::send` has nothing to do.
//!
//! Construct: `web::attach(harness)` builds `Web(OnceLock<ChannelId>)` and
//! registers `Arc<dyn Channel>` via `Harness::attach`; the Store mints the id
//! and `attach` stores it.
//! Use: inbound `ClientMessage::Say` → `Harness::receive(id, text, Human)` →
//! `Store::say` + `Store::receive_mail` → `comms::respond` at `Tier::Comms`;
//! outbound `Harness::forward_said` on `Event::Said` → `Channel::send`.
//! Consumers: `Harness` owns the `Web`; `web::server` drives inbound; `comms`
//! never imports `channels`.
//!
//! | Adapter | `Channel::send` | Inbound |
//! |---|---|---|
//! | `stdio::Stdio` | prints cyan to stdout | blocking stdin loop → `Harness::receive` |
//! | `web::Web` | **no-op** — Store + push already delivered | WS `Say` → `Harness::receive` |
//!
//! Rules: **a new transport must not change `comms.rs`**; one Comms Session per
//! Channel; `Channel::send` is fire-and-forget, delivery is the Store.
//!
//! Defines: [`Web`], [`attach`].

use std::sync::{Arc, OnceLock};

use crate::domain::{ChannelId, ChannelKind};

/// The browser as a [`crate::comms::Channel`].
///
/// Holds the id minted by the Store on `attach`.
pub struct Web {
	/// Minted by the Store on `attach`; `OnceLock` because the id does not exist beforehand.
	id: OnceLock<ChannelId>,
}

impl crate::comms::Channel for Web {
	fn id(&self) -> ChannelId {
		*self
			.id
			.get()
			.expect("id is set before this Channel is used")
	}

	fn kind(&self) -> ChannelKind {
		ChannelKind::Web
	}

	/// Deliver text to the browser.
	///
	/// No-op — `Store::say` already wrote the transcript and `Event::Said` pushes it.
	fn send(&self, _text: &str) {}
}

/// Open the browser Channel and return its id.
///
/// Registers `Web` via `Harness::attach` and stores the minted id.
/// Fails only on `StoreError`.
pub async fn attach(
	harness: Arc<crate::harness::Harness>,
) -> Result<ChannelId, crate::store::StoreError> {
	let web = Arc::new(Web { id: OnceLock::new() });
	let id = harness.attach(web.clone()).await?;
	web.id.set(id).expect("attach runs once per Web");
	Ok(id)
}
