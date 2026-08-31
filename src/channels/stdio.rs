//! Terminal Channel — stdin/stdout transport.
//!
//! What the human types on stdin arrives as `IncomingFrom::Human` mail; what
//! the swarm says comes back on stdout in cyan. The trace stays in
//! `sandman.log`, so the two never interleave.
//!
//! Construct: [`attach`] creates `Stdio` with an empty `OnceLock`, registers
//! it via `Harness::attach` (`Store::open_comms` mints `ChannelId`), then
//! spawns the blocking stdin loop.
//! Use: inbound `prompt` → `read_line` → `Harness::receive`; outbound
//! `Channel::send` → cyan `println!` via `Harness::forward_said` on `Event::Said`.
//! Consumers: `Harness` owns the transport; `comms::respond` never imports `channels`.
//!
//! | `Channel` | `Stdio` (this file) | `Web` (`channels::web`) |
//! |---|---|---|
//! | `send` | prints cyan to stdout | no-op — push carries transcript |
//! | inbound | blocking stdin loop | browser → `Harness::receive` |
//! | stop | `/quit` or EOF → `Harness::stop` | browser close |
//!
//! Rules: one Comms Session per `Channel`; **`Channel::send` is fire-and-forget**, delivery is the `Store`.
//!
//! Defines: [`Stdio`], [`attach`], [`prompt`].

use std::io::Write;
use std::sync::{Arc, OnceLock};

use crate::domain::{ChannelId, ChannelKind, IncomingFrom};

/// Terminal transport as a `Channel`.
///
/// Holds the `ChannelId` minted by the `Store`; `send` prints to stdout in cyan.
pub struct Stdio {
	/// Minted `ChannelId`, set once by `attach` after `Harness::attach`.
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

/// Open the terminal Channel and drive stdin.
///
/// Spawns a blocking loop that forwards lines to the `Harness`.
/// Returns the minted `ChannelId`; `/quit` or EOF stops the `Harness`.
pub async fn attach(
	harness: Arc<crate::harness::Harness>,
) -> Result<ChannelId, crate::store::StoreError> {
	// Register channel
	let stdio = Arc::new(Stdio { id: OnceLock::new() });
	let id = harness.attach(stdio.clone()).await?;
	stdio.id.set(id).expect("attach runs once per Stdio");

	// Spawn blocking reader
	let runtime = tokio::runtime::Handle::current();
	tokio::task::spawn_blocking(move || {
		let stdin = std::io::stdin();
		let mut line = String::new();
		// Read input loop
		loop {
			// Prompt and read line
			prompt();
			line.clear();
			match stdin.read_line(&mut line) {
				// EOF or error - stop harness
				Ok(0) | Err(_) => {
					harness.stop();
					break;
				},
				// Line ready - continue
				Ok(_) => {},
			}
			let text = line.trim_end_matches(['\r', '\n']);
			// Check quit command
			if text == "/quit" {
				harness.stop();
				break;
			}
			// Forward to harness
			runtime.block_on(harness.receive(id, text, IncomingFrom::Human));
		}
	});

	Ok(id)
}

/// Draw the input prompt.
///
/// Writes `> ` to stdout and flushes.
pub fn prompt() {
	print!("> ");
	let _ = std::io::stdout().flush();
}
