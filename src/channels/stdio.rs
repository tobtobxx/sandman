//! The terminal Channel.
//!
//! The human types, and what the swarm says comes back on stdout. Only the
//! conversation goes here — the trace goes to `sandman.log`, so the two never
//! interleave.
//!
//! Defines: [`Stdio`], [`attach`].

use std::sync::Arc;

use crate::domain::{ChannelId, ChannelKind};

/// The terminal, as a Channel.
pub struct Stdio {
    id: ChannelId,
}

impl crate::comms::Channel for Stdio {
    fn id(&self) -> ChannelId {
        unimplemented!()
    }

    fn kind(&self) -> ChannelKind {
        ChannelKind::Stdio
    }

    fn send(&self, _text: &str) {
        unimplemented!()
    }
}

/// Open the terminal Channel and start reading lines from it.
///
/// `/quit` leaves. Everything else is something the human said.
pub async fn attach(
    _harness: Arc<crate::harness::Harness>,
) -> Result<ChannelId, crate::store::StoreError> {
    unimplemented!()
}

/// Draw the prompt the human types at.
pub fn prompt() {
    unimplemented!()
}
