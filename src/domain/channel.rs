//! The Channel: a two-way connection to a human.
//!
//! More than one may be open at once, and each has its own Comms Session, so the
//! swarm may be talking to several humans who share nothing. One-way sources
//! such as RSS or mail are out of scope — anything outside issues a Task through
//! the control socket instead.
//!
//! The Transcript is narrower than the Comms Session's own history: it is what
//! the human actually saw and what they said, without the system prompt, the
//! tool calls, or the post from the swarm the human was never shown.
//!
//! Defines: [`ChannelRecord`], [`ChannelKind`], [`Utterance`], [`Who`].

use super::ids::{ChannelId, SessionId};
use super::time::Timestamp;

/// One open connection to a human, as the Store holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRecord {
	pub id: ChannelId,
	/// How this Channel reaches its human, for a person reading the UI.
	pub kind: ChannelKind,
	/// The Comms Session standing on it. Exactly one, for the Channel's life.
	pub session: SessionId,
	/// What the human has seen, and what they said.
	pub transcript: Vec<Utterance>,
}

/// What kind of transport a Channel sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
	/// The terminal Sandman was started in.
	Stdio,
	/// A browser on the Watcher UI.
	Web,
	/// A bench case's script. Named honestly, so an artifact does not claim a
	/// terminal that was never there.
	Scripted,
}

/// One thing said on a Channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utterance {
	pub who: Who,
	pub text: String,
	pub at: Timestamp,
}

/// Which side of a Channel spoke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
	Human,
	Sandman,
}

impl ChannelKind {
	pub fn discriminant(&self) -> &'static str {
		unimplemented!()
	}
}
