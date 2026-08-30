//! Time as the domain uses it, and money.
//!
//! One instant type, epoch milliseconds, so a Timestamp means the same thing in
//! the database, on the wire and in a Brief. Wall-clock reading itself is behind
//! the [`Clock`] seam: production reads the system clock, and a bench can hold
//! time still while a real model call decides what to do with it.
//!
//! [`Cost`] is an integer of nano-USD rather than a float. A run's Spend is a
//! sum of several hundred fractions of a cent, and an integer sum is exact where
//! a float sum drifts.
//!
//! Defines: [`Timestamp`], [`Duration`], [`Cost`], [`Spend`], [`Clock`],
//! [`SystemClock`], [`FixedClock`], [`ManualClock`], [`stamp`].

use std::fmt;

/// An instant, as epoch milliseconds.
#[derive(
	Debug,
	Clone,
	Copy,
	PartialEq,
	Eq,
	PartialOrd,
	Ord,
	Hash,
	serde::Serialize,
	serde::Deserialize,
)]
pub struct Timestamp(pub i64);

/// A span of time, in milliseconds. Always positive where the domain uses it.
#[derive(
	Debug,
	Clone,
	Copy,
	PartialEq,
	Eq,
	PartialOrd,
	Ord,
	Hash,
	serde::Serialize,
	serde::Deserialize,
)]
pub struct Duration(pub i64);

/// Money, in nano-USD. Sums exactly; prints as six decimal places of a dollar —
/// the last three digits are lost on the way out, so nothing prints a Cost and
/// reads it back.
#[derive(
	Debug,
	Clone,
	Copy,
	Default,
	PartialEq,
	Eq,
	PartialOrd,
	Ord,
	serde::Serialize,
	serde::Deserialize,
)]
pub struct Cost(pub i64);

/// What a Run has cost so far.
///
/// Always derived by summing the model calls that finished, never accumulated in
/// a counter, so it cannot drift from the calls it came from.
#[derive(
	Debug,
	Clone,
	Copy,
	Default,
	PartialEq,
	Eq,
	serde::Serialize,
	serde::Deserialize,
)]
pub struct Spend {
	pub calls: u32,
	pub tokens: u64,
	pub cost: Cost,
}

impl Timestamp {
	/// This instant plus a span.
	pub fn plus(self, d: Duration) -> Timestamp {
		Timestamp(self.0 + d.0)
	}

	/// How long from this instant to a later one. Saturates at zero.
	pub fn until(self, later: Timestamp) -> Duration {
		Duration((later.0 - self.0).max(0))
	}
}

impl Duration {
	pub const fn from_secs(s: i64) -> Duration {
		Duration(s * 1_000)
	}

	pub const fn from_millis(ms: i64) -> Duration {
		Duration(ms)
	}
}

/// Where wall-clock time comes from.
///
/// The one seam that lets a bench assert on scheduled work without waiting for
/// it. Production is [`SystemClock`] and nothing else; a case that swaps it is
/// testing the Harness rather than the model, and should say so.
pub trait Clock: Send + Sync {
	fn now(&self) -> Timestamp;
}

/// The real clock. What every Sandman run outside a test uses.
#[derive(Debug, Default)]
pub struct SystemClock;

/// A clock stopped at one instant. Every read returns the same time.
#[derive(Debug)]
pub struct FixedClock(pub Timestamp);

/// A clock that only moves when a test moves it.
#[derive(Debug)]
pub struct ManualClock {
	now: std::sync::Mutex<Timestamp>,
}

impl Clock for SystemClock {
	fn now(&self) -> Timestamp {
		Timestamp(chrono::Utc::now().timestamp_millis())
	}
}

impl Clock for FixedClock {
	fn now(&self) -> Timestamp {
		self.0
	}
}

impl Clock for ManualClock {
	fn now(&self) -> Timestamp {
		*self.now.lock().unwrap()
	}
}

impl ManualClock {
	pub fn starting_at(t: Timestamp) -> Self {
		ManualClock { now: std::sync::Mutex::new(t) }
	}

	/// Move time forward. Anything waiting on the clock sees the new instant on
	/// its next read.
	pub fn advance(&self, by: Duration) {
		let mut now = self.now.lock().unwrap();
		*now = now.plus(by);
	}
}

/// One instant, written for a model to read: weekday, date and time of day.
///
/// Time enters a Session as text riding on whatever arrived — a Brief, a piece
/// of mail — rather than as a tool. A Session that runs for a while needs "just
/// now" and "earlier" to mean something.
pub fn stamp(at: Timestamp) -> String {
	use chrono::TimeZone;
	let local = chrono::Local.timestamp_millis_opt(at.0).unwrap();
	local.format("%A, %Y-%m-%d %H:%M").to_string()
}

impl fmt::Display for Cost {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let dollars = self.0 / 1_000_000_000;
		let micros = (self.0 % 1_000_000_000) / 1_000;
		write!(f, "${}.{:06}", dollars, micros)
	}
}

impl std::ops::Add for Cost {
	type Output = Cost;
	fn add(self, rhs: Cost) -> Cost {
		Cost(self.0 + rhs.0)
	}
}
