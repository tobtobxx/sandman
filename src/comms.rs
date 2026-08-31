//! The Comms Session: one standing Session per Channel, the only voice a human hears.
//!
//! Construct: `open(ctx, kind)` mints Channel + Session via `Store::open_comms`; every entry takes `SessionCtx` built by `Harness::ctx(id)`.
//! Use: `receive(ctx, text, from)` enqueues mail; `respond(ctx)` drains mail → `tell` each → `turn(ctx, Tier::Comms)` → `say` if `Text` without `NO_RESPONSE` → `Idle`.
//! Consumers: `Harness::drive_comms` loops `respond` (one at a time per Channel); `Channel` adapters (`channels/stdio`, `channels/web`, `bench::BenchChannel`) implement `send`; `Harness::receive` enqueues swarm side.
//! Seam: `Turn` reported by `session::turn`, decided by policy here vs `worker.rs`:
//!
//! | `Turn` | `comms.rs` | `worker.rs` |
//! | --- | --- | --- |
//! | `Text` | `say` to human | `reflect` → `Done` or `Continue` |
//! | `Silent` | legitimate end → `Idle` | `reflect`; fallback `Continue` |
//! | `Unreachable` | `Idle`, nothing said | `Failed` without review |
//! | `Cancelled` | unreachable (no Task) | `Aborted`, no Result |
//!
//! Rules: **a Turn decides nothing — policy lives in `comms.rs` / `worker.rs`, never `session.rs`.** **Comms never completes a Task and is never reviewed.** **One Comms Session per Channel, standing, never ends.** **`NO_RESPONSE` is silence; models are bad at empty replies.** **Comms knows no transport; `Channel` adapters own it.** **Interrupted like any Session, inside `turn`.**
//!
//! Defines: [`Channel`], [`NO_RESPONSE`], [`open`], [`receive`], [`respond`].

use crate::domain::{
	ChannelId, ChannelKind, Incoming, IncomingFrom, Message, SessionId,
	SessionStatus, Utterance, Who,
};
use crate::scheduler::Tier;
use crate::session::SessionCtx;

/// Marker for silence.
///
/// `respond` suppresses any `Text` containing this; nothing reaches the human.
pub const NO_RESPONSE: &str = "<no-response />";

/// Transport as the Comms Session sees it.
///
/// Implemented by `channels/*`; `send` delivers text already stored in the transcript.
pub trait Channel: Send + Sync {
	fn id(&self) -> ChannelId;
	fn kind(&self) -> ChannelKind;
	fn send(&self, text: &str);
}

/// Open a Channel and its standing Comms Session.
///
/// Mints both ids atomically. One Session per Channel for the Channel's life.
pub async fn open(
	ctx: &SessionCtx,
	kind: ChannelKind,
) -> Result<(ChannelId, SessionId), crate::store::StoreError> {
	let now = ctx.clock.now();
	let messages = vec![Message::System {
		content: crate::prompts::COMMS_SESSION.to_string(),
	}];
	let (session, channel) = ctx.store.open_comms(kind, messages, now)?;
	Ok((channel, session))
}

/// Enqueue mail for this Comms Session.
///
/// Human input also appends to the Channel transcript; swarm mail does not.
pub async fn receive(ctx: &SessionCtx, text: &str, from: IncomingFrom) {
	let now = ctx.clock.now();

	// Append to transcript if human
	if from == IncomingFrom::Human {
		if let Some(channel) = ctx
			.store
			.session(ctx.id)
			.ok()
			.flatten()
			.and_then(|s| s.kind.channel())
		{
			let _ = ctx.store.say(
				channel,
				Utterance { who: Who::Human, text: text.to_string(), at: now },
			);
		}
	}

	// Enqueue mail
	let _ = ctx.store.receive_mail(
		ctx.id,
		Incoming { from, text: text.to_string(), at: now },
	);
}

/// Drain the mailbox, run one turn, and speak the result.
///
/// Takes all unread mail at once; runs at `Tier::Comms`. Says `Text` unless it
/// contains `NO_RESPONSE`; otherwise goes `Idle`.
pub async fn respond(ctx: &SessionCtx) {
	// Take mail
	let Ok(mail) = ctx.store.take_mail(ctx.id) else {
		return;
	};
	if mail.is_empty() {
		return;
	}

	// Enqueue in context
	for item in &mail {
		let who = match item.from {
			IncomingFrom::Human => "human",
			IncomingFrom::Swarm => "swarm",
		};
		let content = format!(
			"[{who}, {}] {}",
			crate::domain::time::stamp(item.at),
			item.text
		);
		crate::session::tell(ctx, &content).await;
	}

	// Run turn at top tier
	if let crate::session::Turn::Text(text) =
		crate::session::turn(ctx, Tier::Comms).await
	{
		// Say if not silent
		if !text.contains(NO_RESPONSE) {
			say(ctx, &text).await;
		}
	}

	// Go idle
	let _ = ctx.store.set_status(ctx.id, SessionStatus::Idle);
}

/// Append to the transcript and send on the transport.
///
/// No-op if the Session has no Channel.
async fn say(ctx: &SessionCtx, text: &str) {
	let Some(channel) = ctx
		.store
		.session(ctx.id)
		.ok()
		.flatten()
		.and_then(|s| s.kind.channel())
	else {
		return;
	};
	let now = ctx.clock.now();
	let _ = ctx.store.say(
		channel,
		Utterance { who: Who::Sandman, text: text.to_string(), at: now },
	);
}
