//! Checked-once text — `Title`, `Brief`, and `Day` validate once at the edge.
//!
//! A `Title` that wraps no longer scans and a `Brief` that assumes context
//! leaves a Worker stranded; the prototype re-checked both at every tool.
//! Here the check is `TryFrom<String>` at the edge and `#[serde(try_from =
//! "String")]` on DB read — no second way in, no downstream re-validation.
//! A newtype serialises as the bare string it wraps.
//!
//! Construct: `TryFrom<String>` + `TextError` at the model/control/bin edge;
//! `Display`/`Serialize` as bare `String`; `Deserialize` via `try_from`;
//! `Day::today` from `Timestamp` via `chrono::Local`.
//! Use: carried as typed fields in `Task { title, brief }` and `Lesson
//! { day, title in subject }`; read as `&str` via `as_str`/`Display` after.
//! Consumers: every `create_task` path validates, `store`/`db::rows` re-checks
//! on read, `task`/`lesson`/`memory`/`recall` carry without re-checking.
//!
//! | Type | Rule | Transform | Stored as | Carries |
//! | --- | --- | --- | --- | --- |
//! | `Title` | non-empty | `split_whitespace` collapsed to single spaces | bare `String` | queue scan label |
//! | `Brief` | non-empty after `trim` | none — interior whitespace preserved | bare `String` | Worker's sole context |
//! | `Day` | `YYYY-MM-DD` via `NaiveDate` | none | bare `String` | lesson search hit date |
//!
//! Seam: domain is the data seam — `Model`/`ToolRunner`/`Embedder` exchange
//! these types untouched; `TextError` wording is model-facing tool output,
//! `DbError::Corrupt` is the store's corruption signal on re-check failure.
//!
//! Rules: **checked once — no downstream re-validation.** **one way in —
//! `TryFrom` or `try_from` serde, no direct `Title(String)`.** **`Title`
//! collapses, `Brief` does not.** **`Day` is local, not UTC.** **`TextError`
//! is for the model; `Corrupt` is for the log.**
//!
//! Defines: [`Title`], [`Brief`], [`Day`], [`TextError`].

use std::fmt;

/// One-line queue label a human scans.
///
/// Non-empty; contiguous whitespace collapsed to single spaces on `TryFrom`.
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

/// Sole instructions a Worker receives.
///
/// Must stand alone — the only context the Session gets.
/// Non-empty after `trim`; interior whitespace preserved.
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

/// Local calendar day `YYYY-MM-DD` as a human reads it on a lesson hit.
///
/// Validated by `NaiveDate`; local, not UTC.
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

/// Why text could not become its domain type.
///
/// Wording is model-facing — returned as tool output on `create_task`.
#[derive(Debug, thiserror::Error)]
pub enum TextError {
	#[error("a {what} cannot be empty")]
	Empty { what: &'static str },
	#[error("`{text}` is not a date of the form YYYY-MM-DD")]
	NotADay { text: String },
}

impl Title {
	/// Borrowed view of the title string.
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl Brief {
	/// Borrowed view of the brief string.
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl Day {
	/// Today in the local timezone from epoch millis.
	pub fn today(now: super::time::Timestamp) -> Self {
		use chrono::TimeZone;
		let local = chrono::Local.timestamp_millis_opt(now.0).unwrap();
		Day(local.format("%Y-%m-%d").to_string())
	}

	/// Borrowed view of the day string `YYYY-MM-DD`.
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
