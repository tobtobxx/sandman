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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(pub i64);

/// A span of time, in milliseconds. Always positive where the domain uses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration(pub i64);

/// Money, in nano-USD. Sums exactly; prints as six decimal places of a dollar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cost(pub i64);

/// What a Run has cost so far.
///
/// Always derived by summing the model calls that finished, never accumulated in
/// a counter, so it cannot drift from the calls it came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Spend {
    pub calls: u32,
    pub tokens: u64,
    pub cost: Cost,
}

impl Timestamp {
    /// This instant plus a span.
    pub fn plus(self, _d: Duration) -> Timestamp {
        unimplemented!()
    }

    /// How long from this instant to a later one. Saturates at zero.
    pub fn until(self, _later: Timestamp) -> Duration {
        unimplemented!()
    }
}

impl Duration {
    pub const fn from_secs(_s: i64) -> Duration {
        unimplemented!()
    }

    pub const fn from_millis(_ms: i64) -> Duration {
        unimplemented!()
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
        unimplemented!()
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        unimplemented!()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        unimplemented!()
    }
}

impl ManualClock {
    pub fn starting_at(_t: Timestamp) -> Self {
        unimplemented!()
    }

    /// Move time forward. Anything waiting on the clock sees the new instant on
    /// its next read.
    pub fn advance(&self, _by: Duration) {
        unimplemented!()
    }
}

/// One instant, written for a model to read: weekday, date and time of day.
///
/// Time enters a Session as text riding on whatever arrived — a Brief, a piece
/// of mail — rather than as a tool. A Session that runs for a while needs "just
/// now" and "earlier" to mean something.
pub fn stamp(_at: Timestamp) -> String {
    unimplemented!()
}

impl fmt::Display for Cost {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        unimplemented!()
    }
}

impl std::ops::Add for Cost {
    type Output = Cost;
    fn add(self, _rhs: Cost) -> Cost {
        unimplemented!()
    }
}
