//! The Channel: one two-way line to a human, as the Store holds it.
//!
//! Construct: `Store::open_comms(kind, messages, now)` mints `ChannelId` +
//! `SessionId` atomically; `Store::open_channel(kind, session)` mints alone.
//! Both ids from `counters` inside the transaction that inserts them.
//! Use: `Store::say(channel, utterance)` appends one row; `Store::transcript(id)`
//! reads it; `Store::channels` / `Store::channel` hydrate `ChannelRecord` with
//! its transcript. `Store::channel_session` resolves the standing Comms Session.
//! Consumers: `Store` owns rows; `Harness` owns adapters and drives Comms;
//! `comms::{receive, respond, say}` are transport-agnostic and never import
//! `channels`; `channels::stdio` / `channels::web` implement `Channel::send`;
//! `web::wire` and `log` render the transcript.
//! Seam: `ChannelKind` ↔ adapter behaviour:
//!
//! | Kind | `Channel::send` | Inbound path |
//! |---|---|---|
//! | `Stdio` | cyan to stdout | stdin loop → `Harness::receive` |
//! | `Web` | no-op (Store `Said` already holds it) | browser → `Harness::receive` |
//! | `Matrix` | sends into the direct room | homeserver sync → `Harness::receive` |
//! | `Scripted` | captured for artifacts | bench script → `Harness::receive` |
//!
//! Rules: **one Comms Session per Channel, for its life.** **Transcript is what the human saw — no system prompt, tool calls, or unseen swarm post.** **More than one Channel may be open; they share nothing.** **One-way sources (RSS/mail) out of scope — outside work enters as Task via control socket.**
//!
//! Defines: [`ChannelRecord`], [`ChannelKind`], [`Utterance`], [`Who`].

use super::ids::{ChannelId, SessionId};
use super::time::Timestamp;

/// One open Channel, as the Store holds it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChannelRecord {
	pub id: ChannelId,
	/// Transport this Channel uses, shown in the UI.
	pub kind: ChannelKind,
	/// The single Comms Session standing on this Channel.
	pub session: SessionId,
	/// What the human saw and said, without system prompt or tool calls.
	pub transcript: Vec<Utterance>,
}

/// How a Channel reaches its human.
#[derive(
	Debug,
	Clone,
	Copy,
	PartialEq,
	Eq,
	serde::Serialize,
	serde::Deserialize,
	strum::Display,
	strum::EnumString,
	strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ChannelKind {
	/// The terminal Sandman was started in.
	Stdio,
	/// A browser on the Watcher UI.
	Web,
	/// A direct room with one human on a Matrix homeserver.
	Matrix,
	/// A bench script — named honestly, not a fake terminal.
	Scripted,
}

/// One turn on the transcript: what was said, by whom, and when.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Utterance {
	pub who: Who,
	pub text: String,
	pub at: Timestamp,
}

/// Which side of a Channel spoke.
#[derive(
	Debug,
	Clone,
	Copy,
	PartialEq,
	Eq,
	serde::Serialize,
	serde::Deserialize,
	strum::Display,
	strum::EnumString,
	strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Who {
	Human,
	Sandman,
}
