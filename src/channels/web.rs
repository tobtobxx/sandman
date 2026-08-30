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

use std::sync::Arc;

use crate::domain::{ChannelId, ChannelKind};

/// The browser, as a Channel.
pub struct Web {
	id: ChannelId,
}

impl crate::comms::Channel for Web {
	fn id(&self) -> ChannelId {
		unimplemented!()
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
	_harness: Arc<crate::harness::Harness>,
) -> Result<ChannelId, crate::store::StoreError> {
	unimplemented!()
}
