//! Short readable ids, one newtype per entity.
//!
//! A log full of `t-07` reads far better than one full of uuids, and this
//! system is meant to be read. Each id is a `u32` behind a distinct type, so
//! handing a `SessionId` to something that wants a `TaskId` does not compile.
//!
//! Ids are minted by the Store, inside the same transaction as the insert that
//! uses them (see `db::counters`). They are therefore unique across restarts,
//! and a Store opened on a fresh database counts from one — which is what lets
//! two Harnesses share a process without sharing an id space.
//!
//! Defines: [`RunId`], [`TaskId`], [`SessionId`], [`ChannelId`], [`CallId`],
//! [`LessonId`], and the `id_type!` macro behind them.

use std::fmt;
use std::str::FromStr;

/// Builds one id newtype: a `u32` that prints as `<prefix>-<n>` and parses back.
///
/// The `Display` form is what reaches the log, the wire and a human's eye. The
/// `FromStr` form is what reads it back off a database row or a tool argument.
macro_rules! id_type {
	($name:ident, $prefix:literal, $doc:literal) => {
		#[doc = $doc]
		#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
		pub struct $name(pub u32);

		impl $name {
			/// The counter name this id is minted from, in the `counters` table.
			pub const COUNTER: &'static str = $prefix;

			/// The textual prefix, as it appears in `t-07`.
			pub const PREFIX: &'static str = $prefix;
		}

		impl fmt::Display for $name {
			fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "{}-{:02}", $prefix, self.0)
			}
		}

		impl FromStr for $name {
			type Err = IdError;

			fn from_str(s: &str) -> Result<Self, Self::Err> {
				s.strip_prefix($prefix)
					.and_then(|rest| rest.strip_prefix('-'))
					.and_then(|n| n.parse::<u32>().ok())
					.map($name)
					.ok_or_else(|| IdError {
						text: s.to_string(),
						expected: $prefix,
					})
			}
		}

		impl serde::Serialize for $name {
			fn serialize<S: serde::Serializer>(
				&self,
				s: S,
			) -> Result<S::Ok, S::Error> {
				s.collect_str(self)
			}
		}

		impl<'de> serde::Deserialize<'de> for $name {
			fn deserialize<D: serde::Deserializer<'de>>(
				d: D,
			) -> Result<Self, D::Error> {
				let s = <String as serde::Deserialize>::deserialize(d)?;
				s.parse().map_err(serde::de::Error::custom)
			}
		}
	};
}

id_type!(
	RunId,
	"run",
	"One run of Sandman: a process lifetime, in the database."
);
id_type!(TaskId, "t", "One Task — the single unit of work.");
id_type!(SessionId, "s", "One Session — a live agent context.");
id_type!(
	ChannelId,
	"ch",
	"One Channel — a two-way connection to a human."
);
id_type!(CallId, "call", "One model call.");
id_type!(LessonId, "l", "One lesson kept by metacognition.");

/// An id that could not be read back from its textual form.
#[derive(Debug, thiserror::Error)]
#[error("`{text}` is not a {expected} id")]
pub struct IdError {
	pub text: String,
	pub expected: &'static str,
}
