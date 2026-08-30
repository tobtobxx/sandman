//! The terminal Channel.
//!
//! The human types, and what the swarm says comes back on stdout. Only the
//! conversation goes here — the trace goes to `sandman.log`, so the two never
//! interleave.
//!
//! Defines: [`Stdio`], [`attach`].

use std::io::Write;
use std::sync::{Arc, OnceLock};

use crate::domain::{ChannelId, ChannelKind, IncomingFrom};

/// The terminal, as a Channel.
pub struct Stdio {
	/// Set once, right after the Store mints it — `attach` cannot know it any
	/// earlier, since the Store is what mints it.
	id: OnceLock<ChannelId>,
}

impl crate::comms::Channel for Stdio {
	fn id(&self) -> ChannelId {
		*self
			.id
			.get()
			.expect("id is set before this Channel is used")
	}

	fn kind(&self) -> ChannelKind {
		ChannelKind::Stdio
	}

	fn send(&self, text: &str) {
		println!("{CYAN}{text}{RESET}");
	}
}

/// Sandman's own voice, set apart from what the human typed.
const CYAN: &str = "\x1b[36m";
const RESET: &str = "\x1b[0m";

/// Open the terminal Channel and start reading lines from it.
///
/// `/quit` leaves. Everything else is something the human said.
pub async fn attach(
	harness: Arc<crate::harness::Harness>,
) -> Result<ChannelId, crate::store::StoreError> {
	let stdio = Arc::new(Stdio { id: OnceLock::new() });
	let id = harness.attach(stdio.clone()).await?;
	stdio.id.set(id).expect("attach runs once per Stdio");

	let runtime = tokio::runtime::Handle::current();
	tokio::task::spawn_blocking(move || {
		let stdin = std::io::stdin();
		let mut line = String::new();
		loop {
			prompt();
			line.clear();
			match stdin.read_line(&mut line) {
				// EOF (Ctrl-D) or a broken stdin both mean the terminal is
				// gone, same as typing `/quit`: without this the harness
				// would sit in its run loop forever with nothing left to
				// stop it.
				Ok(0) | Err(_) => {
					harness.stop();
					break;
				},
				Ok(_) => {},
			}
			let text = line.trim_end_matches(['\r', '\n']);
			if text == "/quit" {
				harness.stop();
				break;
			}
			runtime.block_on(harness.receive(id, text, IncomingFrom::Human));
		}
	});

	Ok(id)
}

/// Draw the prompt the human types at.
pub fn prompt() {
	print!("> ");
	let _ = std::io::stdout().flush();
}
