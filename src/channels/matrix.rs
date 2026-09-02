//! The Matrix Channel — one direct room, one human, end-to-end encrypted.
//!
//! Sandman logs in as its own Matrix account, finds (or opens) the direct room
//! it shares with `authorized_user`, and answers there. **Every other sender is
//! ignored**, because a room is not a permission. The device and its keys live
//! in `store_path`, so a restart comes back as the same device rather than as a
//! stranger the human's client refuses to trust.
//!
//! Construct: [`attach`] logs in or restores the saved session, syncs once so
//! the room list is there, sets up cross-signing ([`verify`]), then registers
//! `Arc<dyn Channel>` via `Harness::attach` and spawns three loops.
//! Use: inbound homeserver sync → [`receive`] → `Harness::receive(id, text,
//! Human)` → read receipt; outbound `Channel::send` puts text on an unbounded
//! queue that the send loop delivers into the direct room; [`show_typing`]
//! watches `Events` and turns the typing indicator on while the Comms Session
//! thinks.
//! Consumers: `bin/sandman::serve` attaches it when `[channels.matrix] enable`
//! is set; `comms::respond` is transport-agnostic and never imports `channels`.
//!
//! | Adapter | `Channel::send` | Inbound path |
//! |---|---|---|
//! | [`stdio`](super::stdio) | prints cyan to stdout | blocking stdin loop |
//! | [`web`](super::web) | no-op — push carries the transcript | browser → `Harness::receive` |
//! | `matrix` (this file) | queued, sent into the direct room | sync → `Harness::receive` |
//!
//! Verification, decided by `recovery_passphrase`:
//!
//! | Passphrase | Fresh login | Restored session |
//! |---|---|---|
//! | set | recovers from Secret Storage, or bootstraps and uploads to it | recovers if the local keys went missing |
//! | absent | bootstraps cross-signing locally; other clients show the device unverified | nothing to do |
//!
//! Rules: **only `authorized_user` is heard.** **`Channel::send` is
//! fire-and-forget** — the queue never blocks a Turn, and the Store is
//! delivery. **The sync loop never ends**; a failed sync waits [`RETRY`] and
//! starts again, because a homeserver is allowed to be away. **Transport
//! trouble goes to the `Logger`, never to stdout**, which belongs to the
//! terminal Channel.
//!
//! Defines: [`Matrix`], [`MatrixError`], [`attach`].

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use matrix_sdk::{
	authentication::matrix::MatrixSession,
	config::SyncSettings,
	encryption::recovery::RecoveryError,
	ruma::{
		api::client::{
			receipt::create_receipt,
			uiaa::{AuthData, Password, UserIdentifier},
		},
		events::{
			receipt::ReceiptThread,
			room::message::{
				MessageType, OriginalSyncRoomMessageEvent, Relation,
				RoomMessageEventContent,
			},
			AnySyncMessageLikeEvent, AnySyncTimelineEvent,
			OriginalSyncMessageLikeEvent, SyncMessageLikeEvent,
		},
		EventId, OwnedUserId, UserId,
	},
	store::RoomLoadSettings,
	Client, Room, RoomMemberships, RoomState,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use crate::domain::{ChannelId, ChannelKind, IncomingFrom, SessionStatus};
use crate::event::Event;
use crate::harness::Harness;
use crate::log::Logger;

/// How long a failed sync waits before it tries again.
const RETRY: Duration = Duration::from_secs(10);

/// How often a standing typing indicator is renewed. The server forgets one
/// after ten seconds.
const REFRESH: Duration = Duration::from_secs(8);

/// A Matrix direct room as a `Channel`.
///
/// Holds the `ChannelId` minted by the Store and the queue the send loop
/// drains; `send` only enqueues, so no Turn ever waits on a homeserver.
pub struct Matrix {
	/// Minted `ChannelId`, set once by `attach` after `Harness::attach`.
	id: OnceLock<ChannelId>,
	/// Outbound text, drained by the send loop.
	out: UnboundedSender<String>,
}

impl crate::comms::Channel for Matrix {
	fn id(&self) -> ChannelId {
		*self
			.id
			.get()
			.expect("id is set before this Channel is used")
	}

	fn kind(&self) -> ChannelKind {
		ChannelKind::Matrix
	}

	fn send(&self, text: &str) {
		let _ = self.out.send(text.to_string());
	}
}

/// Failure to open the Matrix Channel.
///
/// Only `attach` returns these; once the loops run, trouble is logged and
/// retried instead.
#[derive(Debug, thiserror::Error)]
pub enum MatrixError {
	#[error("`{what}` is not a Matrix user id: {source}")]
	Id {
		what: &'static str,
		source: matrix_sdk::IdParseError,
	},
	#[error("could not use {}: {source}", .path.display())]
	Store { path: PathBuf, source: std::io::Error },
	#[error("could not write the session: {0}")]
	Session(serde_json::Error),
	#[error("could not reach {0}")]
	Homeserver(#[from] matrix_sdk::ClientBuildError),
	#[error("{0}")]
	Matrix(#[from] matrix_sdk::Error),
	#[error("could not set up encryption: {0}")]
	Encryption(#[from] RecoveryError),
	#[error("{0}")]
	Swarm(#[from] crate::store::StoreError),
}

/// Open the Matrix Channel and drive it.
///
/// Logs in (or restores the saved device), sets up cross-signing, registers
/// the Channel and spawns the sync, send and typing loops. Returns the minted
/// `ChannelId`; the loops run for the life of the process.
pub async fn attach(
	harness: Arc<Harness>,
	config: &crate::config::Matrix,
	logger: Arc<Logger>,
) -> Result<ChannelId, MatrixError> {
	// Read the two identities this Channel is between
	let user = UserId::parse(&config.user)
		.map_err(|source| MatrixError::Id { what: "user", source })?;
	let authorized =
		UserId::parse(&config.authorized_user).map_err(|source| {
			MatrixError::Id { what: "authorized_user", source }
		})?;

	// Log in, or come back as the device we were last time
	let (client, fresh) = open(config, &user, &logger).await?;
	let client = Arc::new(client);
	// The room list has to be there before the first send can find the room.
	client.sync_once(SyncSettings::default()).await?;
	verify(&client, &user, config, fresh, &logger).await?;

	// Register the Channel; `send` from here on only fills the queue
	let (out, mut outbox) = unbounded_channel::<String>();
	let matrix = Arc::new(Matrix { id: OnceLock::new(), out });
	let id = harness.attach(matrix.clone()).await?;
	matrix.id.set(id).expect("attach runs once per Matrix");
	logger.note(
		"matrix",
		&format!("listening as {user}, answering {authorized}"),
	);

	// Inbound: the sync stream, until the process ends
	tokio::spawn({
		let client = client.clone();
		let logger = logger.clone();
		let handler = (harness.clone(), logger.clone(), authorized.clone());
		async move {
			let (harness, notes, authorized) = handler;
			client.add_event_handler(
				move |event: OriginalSyncRoomMessageEvent, room: Room| {
					receive(event, room, id, authorized, harness, notes)
				},
			);
			loop {
				match client.sync(SyncSettings::default()).await {
					Ok(()) => logger.note(
						"matrix",
						"sync stopped by itself — starting it again",
					),
					Err(e) => logger
						.note("matrix", &format!("sync failed: {e} — again")),
				}
				tokio::time::sleep(RETRY).await;
			}
		}
	});

	// Outbound: whatever the Comms Session said
	let known: Known = Arc::new(tokio::sync::OnceCell::new());
	tokio::spawn({
		let known = known.clone();
		let client = client.clone();
		let user = user.to_owned();
		let authorized = authorized.to_owned();
		let logger = logger.clone();
		async move {
			while let Some(text) = outbox.recv().await {
				match direct_room(&known, &client, &user, &authorized, &logger)
					.await
				{
					Ok(room) => {
						let content =
							RoomMessageEventContent::text_plain(&text);
						if let Err(e) = room.send(content).await {
							logger.note(
								"matrix",
								&format!("could not say it: {e}"),
							);
						}
					},
					Err(e) => logger
						.note("matrix", &format!("no room to say it in: {e}")),
				}
			}
		}
	});

	// The human watches the swarm think
	tokio::spawn(show_typing(
		known,
		client,
		user.to_owned(),
		authorized.to_owned(),
		harness,
		id,
		logger,
	));

	Ok(id)
}

/// Build the client and get a session for it.
///
/// Restores `session.json` from `store_path` when it is there, so the same
/// Matrix device comes back; otherwise logs in and saves the new one. Returns
/// the client and whether this was a fresh login.
async fn open(
	config: &crate::config::Matrix,
	user: &UserId,
	logger: &Logger,
) -> Result<(Client, bool), MatrixError> {
	// Make room for the crypto store
	let store = config.store_path.clone();
	tokio::fs::create_dir_all(&store)
		.await
		.map_err(|source| MatrixError::Store { path: store.clone(), source })?;
	let client = Client::builder()
		.server_name_or_homeserver_url(&config.homeserver)
		.sqlite_store(&store, None)
		.build()
		.await?;

	// Restore the saved device, if there is one to restore
	let session_file = store.join("session.json");
	let saved = match tokio::fs::read(&session_file).await {
		Ok(data) => match serde_json::from_slice::<MatrixSession>(&data) {
			Ok(session) => Some(session),
			// A session we cannot read is one we do not have.
			Err(e) => {
				logger.note(
					"matrix",
					&format!(
						"the saved session is unreadable ({e}), \
					          logging in again"
					),
				);
				None
			},
		},
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
		Err(source) => {
			return Err(MatrixError::Store { path: session_file, source });
		},
	};
	if let Some(session) = saved {
		logger.note(
			"matrix",
			&format!("restoring device {}", session.meta.device_id),
		);
		client
			.matrix_auth()
			.restore_session(session, RoomLoadSettings::default())
			.await?;
		return Ok((client, false));
	}

	// Log in as a new device
	let name =
		format!("sandman-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
	let response = client
		.matrix_auth()
		.login_username(user, &config.password)
		.initial_device_display_name(&name)
		.send()
		.await?;
	let session = MatrixSession::from(&response);
	logger.note(
		"matrix",
		&format!("logged in as device {} ({name})", session.meta.device_id),
	);
	let data = serde_json::to_vec(&session).map_err(MatrixError::Session)?;
	tokio::fs::write(&session_file, data)
		.await
		.map_err(|source| MatrixError::Store {
			path: session_file.clone(),
			source,
		})?;
	// The file holds an access token, so nobody else may read it.
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		let mode = std::fs::Permissions::from_mode(0o600);
		std::fs::set_permissions(&session_file, mode).map_err(|source| {
			MatrixError::Store { path: session_file, source }
		})?;
	}

	Ok((client, true))
}

/// Give the device an identity other Matrix clients can trust.
///
/// With a passphrase the private cross-signing keys go to (or come from) the
/// homeserver's Secret Storage, which is what makes the device show as
/// verified and lets a second machine become the same identity. Without one
/// the keys stay local and the device stays unverified — fine for a trial run.
async fn verify(
	client: &Client,
	user: &UserId,
	config: &crate::config::Matrix,
	fresh: bool,
	logger: &Logger,
) -> Result<(), MatrixError> {
	let encryption = client.encryption();
	let recovery = encryption.recovery();

	// No passphrase: a local identity is all we can have
	let Some(passphrase) = config.recovery_passphrase.as_deref() else {
		if fresh {
			logger.note(
				"matrix",
				"bootstrapping cross-signing without Secret Storage — other \
				 clients will show this device as unverified",
			);
			encryption
				.bootstrap_cross_signing(Some(password(user, config)))
				.await?;
			if !encryption.backups().are_enabled().await {
				encryption.backups().create().await?;
			}
		}
		return Ok(());
	};

	// A restored session only needs help when its keys went missing
	if !fresh {
		let complete = encryption
			.cross_signing_status()
			.await
			.is_some_and(|status| status.has_self_signing);
		if complete {
			return Ok(());
		}
		if encryption.secret_storage().is_enabled().await? {
			logger.note(
				"matrix",
				"the local cross-signing keys are gone — recovering them",
			);
			recovery.recover(passphrase).await?;
		} else {
			logger.note(
				"matrix",
				"the local cross-signing keys are gone and the server has no \
				 Secret Storage — this device is unverified",
			);
		}
		return Ok(());
	}

	// A fresh login either joins an identity that exists or makes it
	if encryption.secret_storage().is_enabled().await? {
		logger.note(
			"matrix",
			"recovering the cross-signing keys from Secret Storage",
		);
		recovery.recover(passphrase).await?;
		logger.note("matrix", "this device signed itself");
	} else {
		logger.note("matrix", "bootstrapping cross-signing");
		encryption
			.bootstrap_cross_signing(Some(password(user, config)))
			.await?;
		let key = recovery.enable().with_passphrase(passphrase).await?;
		logger.note(
			"matrix",
			&format!(
				"Secret Storage is set up and this device signed itself. Its \
				 recovery key, an alternative to the passphrase, is {key}"
			),
		);
	}
	Ok(())
}

/// The account's password, as the interactive-auth stage wants it.
fn password(user: &UserId, config: &crate::config::Matrix) -> AuthData {
	AuthData::Password(Password::new(
		UserIdentifier::from(user.to_owned()),
		config.password.clone(),
	))
}

/// Turn one Matrix message into mail for the Comms Session.
///
/// Ignores every sender but `authorized`, quotes what the human replied to,
/// and marks the message read once the swarm holds it.
async fn receive(
	event: OriginalSyncRoomMessageEvent,
	room: Room,
	id: ChannelId,
	authorized: OwnedUserId,
	harness: Arc<Harness>,
	logger: Arc<Logger>,
) {
	// A room we are only invited to is not a room we talk in
	if room.state() != RoomState::Joined {
		return;
	}
	// The one check that matters: a room is not a permission
	if event.sender != authorized {
		logger.note(
			"matrix",
			&format!("ignored a message from {}", event.sender),
		);
		return;
	}

	// Say what came in, even when it was not text
	let body = match &event.content.msgtype {
		MessageType::Text(text) => text.body.trim().to_owned(),
		other => format!(
			"[the human sent a {} message: \"{}\"]",
			other.msgtype(),
			other.body()
		),
	};
	// A reply without its subject reads as a non-sequitur
	let text = match &event.content.relates_to {
		Some(Relation::Reply(reply)) => {
			match quote(&room, &reply.in_reply_to.event_id).await {
				Some(quoted) => format!("{quoted}\n---\n{body}"),
				None => body,
			}
		},
		_ => body,
	};

	// Hand it over, then tell the human it arrived
	harness.receive(id, &text, IncomingFrom::Human).await;
	let receipt = room
		.send_single_receipt(
			create_receipt::v3::ReceiptType::Read,
			ReceiptThread::Unthreaded,
			event.event_id,
		)
		.await;
	if let Err(e) = receipt {
		logger.note("matrix", &format!("could not mark it read: {e}"));
	}
}

/// Render the message a reply points at as a quote block.
///
/// `None` when the event cannot be fetched or was not text — a missing quote
/// is better than a wrong one.
async fn quote(room: &Room, event: &EventId) -> Option<String> {
	let found = room.event(event, None).await.ok()?;
	let (sender, at, body) = match found.raw().deserialize().ok()? {
		AnySyncTimelineEvent::MessageLike(
			AnySyncMessageLikeEvent::RoomMessage(
				SyncMessageLikeEvent::Original(OriginalSyncMessageLikeEvent {
					sender,
					origin_server_ts,
					content,
					..
				}),
			),
		) => {
			let body = match &content.msgtype {
				MessageType::Text(text) => text.body.trim().to_owned(),
				_ => String::from("[not a text message]"),
			};
			(sender, u64::from(origin_server_ts.get()), body)
		},
		_ => return None,
	};

	let when = chrono::DateTime::from_timestamp_millis(at as i64)?
		.format("%Y-%m-%d %H:%M UTC");
	let quoted = body
		.lines()
		.map(|line| format!("> {line}"))
		.collect::<Vec<_>>()
		.join("\n");
	Some(format!("On {when}, {sender} wrote:\n{quoted}"))
}

/// Show a typing indicator while this Channel's Comms Session works.
///
/// Follows `Events` rather than the transport, so the indicator says what the
/// swarm is doing. Renewed every [`REFRESH`] because the server forgets it.
async fn show_typing(
	known: Known,
	client: Arc<Client>,
	user: OwnedUserId,
	authorized: OwnedUserId,
	harness: Arc<Harness>,
	id: ChannelId,
	logger: Arc<Logger>,
) {
	let Ok(Some(session)) = harness.store.channel_session(id) else {
		return;
	};
	let mut events = harness.events.subscribe();
	let mut typing = false;
	let mut renew = tokio::time::Instant::now();

	loop {
		// Follow the Session, or renew what is already shown
		let show = tokio::select! {
			event = events.recv() => match event {
				Ok(Event::SessionStatusChanged { session: which, to })
					if which == session =>
				{
					matches!(
						to,
						SessionStatus::Thinking
							| SessionStatus::Tools
							| SessionStatus::Reflecting
					)
				},
				// The bus is gone, so the swarm is
				Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
				_ => continue,
			},
			_ = tokio::time::sleep_until(renew), if typing => true,
		};
		if !show && !typing {
			continue;
		}

		// Tell the room
		typing = show;
		renew = tokio::time::Instant::now() + REFRESH;
		match direct_room(&known, &client, &user, &authorized, &logger).await {
			Ok(room) => {
				if let Err(e) = room.typing_notice(typing).await {
					logger
						.note("matrix", &format!("could not show typing: {e}"));
				}
			},
			Err(e) => {
				logger.note("matrix", &format!("could not show typing: {e}"))
			},
		}
	}
}

/// The direct room, looked up once and then remembered.
///
/// Both the send loop and the typing indicator want it, and finding it asks
/// the homeserver for the members of every joined room.
type Known = Arc<tokio::sync::OnceCell<Room>>;

/// The direct room, from the cache or from the homeserver.
async fn direct_room(
	known: &Known,
	client: &Client,
	user: &UserId,
	authorized: &UserId,
	logger: &Logger,
) -> Result<Room, matrix_sdk::Error> {
	known
		.get_or_try_init(|| find_direct_room(client, user, authorized, logger))
		.await
		.cloned()
}

/// The room shared by exactly these two, opened if it is not there yet.
///
/// A room with anyone else in it is not the direct room, however it is named.
async fn find_direct_room(
	client: &Client,
	user: &UserId,
	authorized: &UserId,
	logger: &Logger,
) -> Result<Room, matrix_sdk::Error> {
	// Look for the room the two of us are alone in
	let mut wanted = vec![user, authorized];
	wanted.sort();
	for room in client.joined_rooms() {
		let joined = room.members(RoomMemberships::JOIN).await?;
		let mut members: Vec<&UserId> =
			joined.iter().map(|member| member.user_id()).collect();
		members.sort();
		if members == wanted {
			return Ok(room);
		}
	}

	// There is none, so open one
	logger.note(
		"matrix",
		&format!("no direct room with {authorized} yet, opening one"),
	);
	client.create_dm(authorized).await
}
