//! Time and money as domain primitives — no logic.
//!
//! One [`Timestamp`] (epoch millis) means the same in SQLite, on the wire and
//! in a [`Brief`](super::text::Brief). [`Duration`] is millis, [`Cost`] is
//! integer nano-USD so a [`Spend`] sums exactly where floats drift. Reading
//! the wall clock is behind the [`Clock`] seam.
//!
//! Construct: `Timestamp(i64)` / `Duration::from_secs|millis` and `Cost(i64)`
//! directly; `Spend` is never constructed — `Store::spend` re-sums `Done`
//! calls; `Clock` via [`SystemClock`] (prod) or [`FixedClock`] /
//! [`ManualClock`] (bench); [`stamp`] formats local weekday+date+time for
//! model context.
//! Use: `Clock::now() -> Timestamp` threaded as `Arc<dyn Clock>` in
//! `SessionCtx`, `Harness` and `Scheduler`; `Timestamp::plus` / `until` for
//! scheduling and interrupt counting; `Cost` / `Spend` summed from
//! `LlmCall::Usage`.
//! Consumers and how they match the seam:
//!
//! | Type | Store (only writer) | Scheduler / Harness / Session | Bench |
//! | --- | --- | --- | --- |
//! | `Timestamp` / `Duration` | persists `not_before`, `queued_at`, `started_at` | `next_pending(now)`, `next_due_in`, interrupt `msgs - last >= interval` | `FixedClock(at)` frozen, `ManualClock::advance` by hand |
//! | `Clock` | — | `SystemClock` via `chrono::Utc` | `FixedClock` stopped, `ManualClock` moved by test |
//! | `Cost` / `Spend` | re-sums `Done` calls' `Usage` on every `spend()` | `Model` returns `Usage{cost}`; `Spend` not accumulated | whatever cost a test wants |
//! | `stamp` | — | injects time as text on `Brief` / mail, never as tool | — |
//!
//! Seam: `Clock` is the time seam — `Model`, `ToolRunner`, `Embedder` share
//! it; bench swaps it to assert on scheduling without waiting. `Cost` / `Spend`
//! have no seam — one integer type, re-summed.
//!
//! Rules: **one instant type — epoch millis everywhere.** **`Spend` re-summed, never accumulated — cannot drift.** **`Cost` integer nano-USD — exact sum, lossy `Display` to 6dp.** **`ManualClock` only moves via `advance`.** **`stamp` is local, not UTC; time enters Session as text.**
//!
//! Defines: [`Timestamp`], [`Duration`], [`Cost`], [`Spend`], [`Clock`],
//! [`SystemClock`], [`FixedClock`], [`ManualClock`], [`stamp`].

use std::fmt;

/// Instant as epoch milliseconds.
/// Single type for DB, wire and Brief.
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

/// Span of time in milliseconds.
/// Always positive where the domain uses it.
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

/// Cost in nano-USD.
/// Sums exactly; `Display` rounds to six decimals and is lossy.
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

/// Cost of a Run so far.
/// Re-summed from `Done` calls on every read, never accumulated.
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
	/// Tokens computed: `prefill + produced` over the Run. Cache hits are not
	/// counted — see [`crate::domain::Usage`].
	pub tokens: u64,
	pub cost: Cost,
}

impl Timestamp {
	/// Add a span to this instant.
	/// Returns the instant `d` after self.
	pub fn plus(self, d: Duration) -> Timestamp {
		Timestamp(self.0 + d.0)
	}

	/// Duration from self to `later`.
	/// Saturates at zero if `later` is earlier.
	pub fn until(self, later: Timestamp) -> Duration {
		Duration((later.0 - self.0).max(0))
	}
}

impl Duration {
	/// Create from seconds.
	pub const fn from_secs(s: i64) -> Duration {
		Duration(s * 1_000)
	}

	/// Create from milliseconds.
	pub const fn from_millis(ms: i64) -> Duration {
		Duration(ms)
	}
}

/// Source of wall-clock time.
/// Implementations decide whether time flows, stands still, or moves by hand.
pub trait Clock: Send + Sync {
	fn now(&self) -> Timestamp;
}

/// System clock via `chrono::Utc`.
/// Used by every production run.
#[derive(Debug, Default)]
pub struct SystemClock;

/// Clock frozen at one instant.
/// Every `now()` returns the same `Timestamp`.
#[derive(Debug)]
pub struct FixedClock(pub Timestamp);

/// Clock that only advances on `advance()`.
/// Tests move time explicitly between scheduler checks.
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
	/// Create a manual clock at `t`.
	pub fn starting_at(t: Timestamp) -> Self {
		ManualClock { now: std::sync::Mutex::new(t) }
	}

	/// Advance time by `by`.
	/// Next `now()` reflects the new instant.
	pub fn advance(&self, by: Duration) {
		let mut now = self.now.lock().unwrap();
		*now = now.plus(by);
	}
}

/// Format `at` for model context.
/// Returns local weekday, date and time of day.
pub fn stamp(at: Timestamp) -> String {
	use chrono::TimeZone;
	// Resolve local time
	let local = chrono::Local.timestamp_millis_opt(at.0).unwrap();
	// Format for model
	local.format("%a %Y-%m-%d %H:%M").to_string()
}

impl fmt::Display for Cost {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		// Split cost
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
