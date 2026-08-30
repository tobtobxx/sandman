//! Text that has been checked once, so nothing downstream checks it again.
//!
//! A Title and a Brief both have a rule the whole system depends on — a Title is
//! one line a human can scan, a Brief must stand alone because it is everything
//! a Worker gets — and in the prototype those rules were re-tested at every tool
//! that built one. Here they are `TryFrom<String>`, so the check happens at the
//! edge where a model's argument becomes a domain value, and every reader after
//! that point is spared it.
//!
//! Reading one back off a database row goes through the same check:
//! `#[serde(try_from = "String")]`, so there is no second way in. Writing one out
//! is the bare string — a newtype serialises as what it wraps.
//!
//! Defines: [`Title`], [`Brief`], [`Day`], [`TextError`].

use std::fmt;

/// A Task's one-line description. It exists so a human can scan the queue; no
/// Session depends on it, and the Brief still has to stand alone.
///
/// Non-empty, and newlines are collapsed on the way in — a Title that wraps is
/// a Title that no longer scans.
#[derive(
	Debug,
	Clone,
	PartialEq,
	Eq,
	PartialOrd,
	Ord,
	Hash,
	serde::Serialize,
	serde::Deserialize,
)]
#[serde(try_from = "String")]
pub struct Title(String);

/// The instructions a Task carries.
///
/// A Session starts fresh and sees nothing of the work that led to it, so this
/// must stand alone: it is the only thing the Worker gets. Non-empty.
#[derive(
	Debug,
	Clone,
	PartialEq,
	Eq,
	PartialOrd,
	Ord,
	Hash,
	serde::Serialize,
	serde::Deserialize,
)]
#[serde(try_from = "String")]
pub struct Brief(String);

/// A local calendar day, `YYYY-MM-DD`.
///
/// Local rather than UTC, because this is a date a human reads off a search hit
/// for a lesson.
#[derive(
	Debug,
	Clone,
	PartialEq,
	Eq,
	PartialOrd,
	Ord,
	Hash,
	serde::Serialize,
	serde::Deserialize,
)]
#[serde(try_from = "String")]
pub struct Day(String);

/// Why a piece of text could not become the domain value it was offered as.
///
/// This is worded for a model to read: it comes back as a tool result when a
/// Worker calls `create_task` with an empty Brief.
#[derive(Debug, thiserror::Error)]
pub enum TextError {
	#[error("a {what} cannot be empty")]
	Empty { what: &'static str },
	#[error("`{text}` is not a date of the form YYYY-MM-DD")]
	NotADay { text: String },
}

impl Title {
	/// The Title as written. Borrowed, because most readers only print it.
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl Brief {
	/// The Brief as written.
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl Day {
	/// Today, in the local zone.
	pub fn today(now: super::time::Timestamp) -> Self {
		use chrono::TimeZone;
		let local = chrono::Local.timestamp_millis_opt(now.0).unwrap();
		Day(local.format("%Y-%m-%d").to_string())
	}

	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl TryFrom<String> for Title {
	type Error = TextError;
	fn try_from(s: String) -> Result<Self, Self::Error> {
		let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
		if collapsed.is_empty() {
			return Err(TextError::Empty { what: "title" });
		}
		Ok(Title(collapsed))
	}
}

impl TryFrom<String> for Brief {
	type Error = TextError;
	fn try_from(s: String) -> Result<Self, Self::Error> {
		if s.trim().is_empty() {
			return Err(TextError::Empty { what: "brief" });
		}
		Ok(Brief(s))
	}
}

impl TryFrom<String> for Day {
	type Error = TextError;
	fn try_from(s: String) -> Result<Self, Self::Error> {
		chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d")
			.map_err(|_| TextError::NotADay { text: s.clone() })?;
		Ok(Day(s))
	}
}

impl fmt::Display for Title {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

impl fmt::Display for Brief {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

impl fmt::Display for Day {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}
