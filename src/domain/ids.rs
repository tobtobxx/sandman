//! Typed ids for every entity.
//!
//! One `u32` per entity behind distinct types, so a `SessionId` cannot be
//! passed where a `TaskId` is expected. Printed as `<prefix>-nn` for logs,
//! wire, and tool args; parsed back via `FromStr`.
//!
//! Construct: `Store` mints inside the same transaction as the insert via
//! `db::counters::take(tx, T::COUNTER)` — unique across restarts, never leaked
//! on rollback, fresh DB starts at 1. Two Harnesses share a process without
//! sharing an id space because the counter lives in SQLite.
//!
//! Use: `Display` / `FromStr` / `serde` as text; `IdError` on mismatch.
//! `db::rows` decodes rows into the same types; tools parse args via `FromStr`.
//!
//! Consumers: `Task`, `Session`, `LlmCall`, `Channel`, `Lesson`, `Run` carry
//! them; `Store` mints them; `db::{counters,rows}` owns the mapping.
//!
//! | Id | Prefix | Entity | Counter row |
//! | --- | --- | --- | --- |
//! | `RunId` | `run` | `Run` | `counters.next` where `name = "run"` |
//! | `TaskId` | `t` | `Task` | `counters.next` where `name = "t"` |
//! | `SessionId` | `s` | `Session` | `counters.next` where `name = "s"` |
//! | `ChannelId` | `ch` | `Channel` | `counters.next` where `name = "ch"` |
//! | `CallId` | `call` | `LlmCall` | `counters.next` where `name = "call"` |
//! | `LessonId` | `l` | `Lesson` | `counters.next` where `name = "l"` |
//!
//! Rules: **one counter per type, keyed by `PREFIX`** — adding an entity means
//! adding a variant here. **mint and insert are one transaction** — no id
//! without a row. **text form is canonical** — logs, DB text fields, and tool
//! args share `Display`/`FromStr`. **distinct types are the seam** — cross-entity
//! misuse does not compile.
//!
//! Defines: [`RunId`], [`TaskId`], [`SessionId`], [`ChannelId`], [`CallId`],
//! [`LessonId`], `id_type!`, [`IdError`]

use std::fmt;
use std::str::FromStr;

/// Define one typed id backed by `u32`.
///
/// Prints as `<prefix>-nn` and parses back. Serializes as that string.
macro_rules! id_type {
	($name:ident, $prefix:literal, $doc:literal) => {
		#[doc = $doc]
		#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
		pub struct $name(pub u32);

		impl $name {
			/// Counter row this id is minted from.
			pub const COUNTER: &'static str = $prefix;

			/// Text prefix as it appears in `t-07`.
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

/// Id parse failure — text did not match `<prefix>-<n>`.
#[derive(Debug, thiserror::Error)]
#[error("`{text}` is not a {expected} id")]
pub struct IdError {
	pub text: String,
	pub expected: &'static str,
}
