//! ANSI helpers for the bench driver's terminal output.
//!
//! `enabled()` is the gate — `NO_COLOR` unset and stdout is a TTY — checked
//! on every call; callers pass that `on` to `bold`/`dim`/`red`/`green`/…
//! which wrap via private `paint` only when on. No cache — a bench prints a
//! handful of lines.
//!
//! Consumed only by `report::{print_run,print_summary,ratio}` for verdicts,
//! ratios and dimmed metadata. No state, no config, no seam.
//!
//! Rules:
//! - **Off when stdout is not a TTY or `NO_COLOR` is set** — logs and CI never see codes.
//! - Callers decide once per run — `enabled()` then every painter with `on`.

use std::io::IsTerminal;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";

/// Whether ANSI output is allowed.
///
/// Returns true only on a TTY without `NO_COLOR`.
pub fn enabled() -> bool {
	std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

fn paint(on: bool, code: &str, text: &str) -> String {
	if on {
		format!("{code}{text}{RESET}")
	} else {
		text.to_string()
	}
}

pub fn bold(on: bool, text: &str) -> String {
	paint(on, BOLD, text)
}

pub fn dim(on: bool, text: &str) -> String {
	paint(on, DIM, text)
}

pub fn red(on: bool, text: &str) -> String {
	paint(on, RED, text)
}

pub fn green(on: bool, text: &str) -> String {
	paint(on, GREEN, text)
}

pub fn yellow(on: bool, text: &str) -> String {
	paint(on, YELLOW, text)
}

pub fn cyan(on: bool, text: &str) -> String {
	paint(on, CYAN, text)
}
