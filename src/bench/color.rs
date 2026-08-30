//! A little color for the bench driver's terminal output.
//!
//! Off whenever stdout is not a real terminal, or `NO_COLOR` is set — a
//! redirected log or a CI runner should never see raw escape codes.

use std::io::IsTerminal;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";

/// Whether to paint at all. Checked once per call rather than cached: a bench
/// run is a handful of prints, not a hot loop.
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
