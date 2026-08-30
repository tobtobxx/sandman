//! The Comms Session: standing, one per Channel, and the only part of Sandman a
//! human ever sees.
//!
//! It is not a Worker. It is never created from a Task, it never completes, and
//! it is never reviewed — it owes nobody a Result. It is interrupted like any
//! other Session, which is the only metacognition it ever sees.
//!
//! It is a Session plus one policy — what the text a turn produces means. Here it
//! means something to say to the human, and then the Session goes idle to wait
//! for more. Silence is a legitimate ending.
//!
//! Two things arrive: what the human says, and what the swarm sends — an answer
//! it subscribed to, or a `message_human` from a planning Worker. Both land in
//! the mailbox, and this Session decides how to pass them on. It owns the
//! human-facing voice: content may go on word for word, or be reworded and given
//! context. That is its decision.
//!
//! The Comms Session does not know which transport it sits on. That is the
//! Channel adapter's job — see `channels/`.
//!
//! Defines: [`Channel`], [`open`], [`receive`], [`respond`], [`NO_RESPONSE`].

use crate::domain::{
	ChannelId, ChannelKind, Incoming, IncomingFrom, Message, SessionId,
	SessionStatus, Utterance, Who,
};
use crate::scheduler::Tier;
use crate::session::SessionCtx;

/// How a turn says nothing.
///
/// Models are bad at producing an empty reply, so a Comms Session writes this
/// marker instead and nothing reaches the human.
pub const NO_RESPONSE: &str = "<no-response />";

/// One transport, as the Comms Session sees it: somewhere to send text.
///
/// The browser adapter has nothing to do on send — the text is already in the
/// Channel's transcript, and the transcript is in the Store, so the same push
/// that carries everything else carries that too.
pub trait Channel: Send + Sync {
	fn id(&self) -> ChannelId;
	fn kind(&self) -> ChannelKind;
	fn send(&self, text: &str);
}

/// Open a Channel and stand a Comms Session on it.
///
/// One Comms Session per Channel, for the Channel's life. Several Channels may
/// be open at once and they share nothing, so as far as the swarm is concerned
/// there are several humans.
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

/// Put something in this Session's mailbox.
///
/// What the human says also goes into the Channel's transcript; what the swarm
/// sends does not, because the human has not seen it yet — the Session decides
/// whether and how to pass it on.
pub async fn receive(ctx: &SessionCtx, text: &str, from: IncomingFrom) {
	let now = ctx.clock.now();

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

	let _ = ctx.store.receive_mail(
		ctx.id,
		Incoming { from, text: text.to_string(), at: now },
	);
}

/// Read the mailbox, take one turn, and say whatever came of it.
///
/// Everything unread is taken at once and put into the context stamped with when
/// it arrived, so "this morning" and "did it finish yet?" mean something. Post
/// that lands mid-turn waits for the next respond — nothing arrives in the
/// middle of this Session's thinking.
///
/// The turn runs at [`crate::scheduler::Tier::Comms`], the top tier: its calls
/// jump the queue in front of every Task, which is how a human is never left
/// waiting behind the swarm's work.
pub async fn respond(ctx: &SessionCtx) {
	let Ok(mail) = ctx.store.take_mail(ctx.id) else {
		return;
	};
	if mail.is_empty() {
		return;
	}

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

	if let crate::session::Turn::Text(text) =
		crate::session::turn(ctx, Tier::Comms).await
	{
		if !text.contains(NO_RESPONSE) {
			say(ctx, &text).await;
		}
	}

	let _ = ctx.store.set_status(ctx.id, SessionStatus::Idle);
}

/// Say something to the human: into the transcript, then out on the transport.
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
