//! The browser Channel.
//!
//! The one thing a Watcher may write: a message on its own Channel. Everything
//! else the browser does is reading.
//!
//! Sending has nothing to do. The Comms Session has already put the text in the
//! Channel's transcript, the transcript is in the Store, and the same push that
//! carries every other change carries that too — delivery is what the Store
//! does.
//!
//! Defines: [`Web`], [`attach`].

use std::sync::{Arc, OnceLock};

use crate::domain::{ChannelId, ChannelKind};

/// The browser, as a Channel.
pub struct Web {
	/// Set once, right after the Store mints it — `attach` cannot know it any
	/// earlier, since the Store is what mints it.
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

	/// Nothing to do: the transcript is already in the Store, and the push
	/// carries it.
	fn send(&self, _text: &str) {}
}

/// Open the browser Channel.
pub async fn attach(
	harness: Arc<crate::harness::Harness>,
) -> Result<ChannelId, crate::store::StoreError> {
	let web = Arc::new(Web { id: OnceLock::new() });
	let id = harness.attach(web.clone()).await?;
	web.id.set(id).expect("attach runs once per Web");
	Ok(id)
}
