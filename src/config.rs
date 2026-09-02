//! One file, one read: everything about the world Sandman runs in.
//!
//! Models, paths, channels, watcher — not policy. Read once at start, shared
//! as `Arc<Config>`; nothing else is configured anywhere.
//!
//! Construct: `Config::path(flag)` → path; `Config::load(path)` reads or writes
//! `DEFAULT` (`include_str!("default-config.toml")`) and returns `Written` on
//! first start; `read` does not write; `parse`/`parse_with(env)` expand then
//! validate. Use: `Harness { config: Arc<Config> }` — `for_all`/`for_role`/
//! `for_comms`/`for_metacognition`/`for_grader` resolve slug → `&ModelSpec`
//! (slugs checked at load, so `resolve` never fails). Consumers: `bin/sandman`
//! (`assemble` → Store, Events, Channels, web, Embedder), `model::Models::from_config`,
//! `memory::OpenRouterEmbedder::from_spec`, `session` (interrupt_interval), `bench::Rig`.
//!
//! Rules:
//! **No fallback in code.** Missing or unknown key is an error at load; `DEFAULT` is the only defaults, as text so file and code cannot drift.
//! **One fallback in file.** Optional `[models]` keys (`comms`, `planning`, …) fall back to `all`; absence means something, which is why it may mean it.
//! **Every string is expanded.** `$NAME` / `${NAME}` / `$$` on the parsed tree before it becomes `Config`; unset is an error, not empty; keys never expanded.
//! **Every named slug must exist.** `check_slugs` fails at start, not mid-run; a slug nothing names is kept without complaint.
//! **Hash by value.** `ModelSpec` hashed by all fields, so two slugs naming the same endpoint share one adapter in `Models`.
//!
//! ```text
//! path → load → read → parse → parse_with → expand_tree → expand → check_slugs → Config
//!         │       └─ Written if NotFound (writes DEFAULT mode 0o600)
//!         └─ never overwrites a file that fails to parse
//! ```
//! Defines: `Config`, `ModelSpec`, `ConfigError`, `DEFAULT`.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use crate::roles::RoleName;

/// Single defaults text, written on first start.
pub const DEFAULT: &str = include_str!("default-config.toml");

/// All world configuration from `config.toml`.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
	pub model: BTreeMap<String, ModelSpec>,
	pub models: ModelChoices,
	pub sandman: Sandman,
	pub metacognition: Metacognition,
	pub embedding: Embedding,
	pub channels: Channels,
	pub tools: Tools,
	pub bench: Bench,
}

/// One model and how to reach it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSpec {
	pub endpoint: String,
	pub api_key: String,
	pub model: String,
	#[serde(deserialize_with = "effort")]
	pub effort: Option<String>,
}

/// Which slug does which work.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelChoices {
	/// Fallback for every Session.
	pub all: String,
	pub comms: Option<String>,
	pub planning: Option<String>,
	pub research: Option<String>,
	pub memory: Option<String>,
	pub task_manager: Option<String>,
}

/// Paths and listen addresses.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sandman {
	pub sqlite_path: PathBuf,
	pub log_path: PathBuf,
	pub control_socket: PathBuf,
	pub webui_address: IpAddr,
	pub webui_port: u16,
}

/// Metacognition settings.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metacognition {
	pub interrupt_interval: usize,
	pub model: String,
}

/// Embedding model for `memory` search.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Embedding {
	pub endpoint: String,
	pub api_key: String,
	pub model: String,
}

/// Ways a human reaches the swarm.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Channels {
	pub stdio: bool,
	pub web: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
	pub searxng_endpoint: String,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bench {
	pub grader: String,
	/// Per-case bound in seconds. A case that runs past it trips.
	pub timeout: i64,
}

/// Failure to obtain a `Config`.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
	#[error(
		"no configuration found, so a default one was written to {}.\nRead it, \
		 then start Sandman again.",
		.0.display()
	)]
	Written(PathBuf),
	#[error("could not read {}: {source}", .path.display())]
	Read { path: PathBuf, source: std::io::Error },
	#[error("could not write {}: {source}", .path.display())]
	Write { path: PathBuf, source: std::io::Error },
	#[error("{}: {source}", .path.display())]
	Parse { path: PathBuf, source: toml::de::Error },
	#[error("{0}")]
	Malformed(toml::de::Error),
	#[error("the configuration names ${0}, which is not set")]
	UnsetVar(String),
	#[error("`{text}` {why}")]
	BadVar { text: String, why: &'static str },
	#[error(
		"`{key}` names the model `{slug}`, and no [model.{slug}] says what \
		 that is"
	)]
	NoSuchSlug { key: String, slug: String },
	#[error(
		"$XDG_CONFIG_HOME is not set. Name a configuration with --config."
	)]
	Nowhere,
}

impl Config {
	/// Resolve configuration path.
	///
	/// Uses `--config` if given, else `$XDG_CONFIG_HOME/sandman/config.toml`.
	/// Fails with `Nowhere` if the variable is unset.
	pub fn path(flag: Option<PathBuf>) -> Result<PathBuf, ConfigError> {
		if let Some(path) = flag {
			return Ok(path);
		}
		let dir = std::env::var("XDG_CONFIG_HOME")
			.map_err(|_| ConfigError::Nowhere)?;
		Ok(PathBuf::from(dir).join("sandman").join("config.toml"))
	}

	/// Read config, or write `DEFAULT` and stop.
	///
	/// Writes only when no file exists; parse errors are left untouched.
	/// Returns `Written` on first start.
	pub fn load(path: &Path) -> Result<Config, ConfigError> {
		match Config::read(path) {
			Err(ConfigError::Read { source, .. })
				if source.kind() == std::io::ErrorKind::NotFound =>
			{
				write_default(path)?;
				Err(ConfigError::Written(path.to_path_buf()))
			},
			other => other,
		}
	}

	/// Read config without writing.
	///
	/// Returns `Read` or `Parse` on failure.
	pub fn read(path: &Path) -> Result<Config, ConfigError> {
		let text = std::fs::read_to_string(path).map_err(|source| {
			ConfigError::Read { path: path.to_path_buf(), source }
		})?;
		Config::parse(&text).map_err(|e| match e {
			ConfigError::Malformed(source) => {
				ConfigError::Parse { path: path.to_path_buf(), source }
			},
			other => other,
		})
	}

	/// Parse config text with the real environment.
	///
	/// Expands vars, deserializes and checks slugs.
	pub fn parse(text: &str) -> Result<Config, ConfigError> {
		Config::parse_with(text, &|name| std::env::var(name).ok())
	}

	/// Parse config text against a supplied environment.
	///
	/// For tests or callers that must not read the process environment.
	pub fn parse_with(
		text: &str,
		env: &dyn Fn(&str) -> Option<String>,
	) -> Result<Config, ConfigError> {
		let mut tree: toml::Value =
			toml::from_str(text).map_err(ConfigError::Malformed)?;
		expand_tree(&mut tree, env)?;
		let config: Config = tree.try_into().map_err(ConfigError::Malformed)?;
		config.check_slugs()?;
		Ok(config)
	}

	/// Every slug this configuration names.
	fn named_slugs(&self) -> Vec<(String, &str)> {
		let mut named: Vec<(String, &str)> =
			vec![("models.all".to_string(), self.models.all.as_str())];
		for role in <RoleName as strum::VariantArray>::VARIANTS {
			if let Some(slug) = self.models.named_for(*role) {
				named.push((format!("models.{role}"), slug));
			}
		}
		if let Some(slug) = &self.models.comms {
			named.push(("models.comms".to_string(), slug.as_str()));
		}
		named.push((
			"metacognition.model".to_string(),
			self.metacognition.model.as_str(),
		));
		named.push(("bench.grader".to_string(), self.bench.grader.as_str()));
		named
	}

	/// Verify every named slug has a `[model.*]` table.
	fn check_slugs(&self) -> Result<(), ConfigError> {
		for (key, slug) in self.named_slugs() {
			if !self.model.contains_key(slug) {
				return Err(ConfigError::NoSuchSlug {
					key,
					slug: slug.to_string(),
				});
			}
		}
		Ok(())
	}

	/// Lookup spec by slug.
	pub fn spec(&self, slug: &str) -> Option<&ModelSpec> {
		self.model.get(slug)
	}

	/// Spec for `models.all`.
	pub fn for_all(&self) -> &ModelSpec {
		self.resolve(&self.models.all)
	}

	/// Spec for a Role's Workers.
	pub fn for_role(&self, role: RoleName) -> &ModelSpec {
		self.resolve(self.models.named_for(role).unwrap_or(&self.models.all))
	}

	/// Spec for the Comms Session.
	pub fn for_comms(&self) -> &ModelSpec {
		self.resolve(self.models.comms.as_deref().unwrap_or(&self.models.all))
	}

	/// Spec for review and interrupt.
	pub fn for_metacognition(&self) -> &ModelSpec {
		self.resolve(&self.metacognition.model)
	}

	/// Spec for the bench grader.
	pub fn for_grader(&self) -> &ModelSpec {
		self.resolve(&self.bench.grader)
	}

	/// Resolve slug, panicking if unchecked.
	fn resolve(&self, slug: &str) -> &ModelSpec {
		self.spec(slug).expect("slugs are checked at load")
	}
}

impl ModelChoices {
	/// Slug for this `Role`, if one is configured.
	fn named_for(&self, role: RoleName) -> Option<&str> {
		match role {
			RoleName::Research => self.research.as_deref(),
			RoleName::Planning => self.planning.as_deref(),
			RoleName::Memory => self.memory.as_deref(),
			RoleName::TaskManager => self.task_manager.as_deref(),
		}
	}
}

/// Write `DEFAULT` to `path` with parent dirs and `0o600`.
fn write_default(path: &Path) -> Result<(), ConfigError> {
	let failed = |source: std::io::Error| ConfigError::Write {
		path: path.to_path_buf(),
		source,
	};
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent).map_err(failed)?;
	}
	std::fs::write(path, DEFAULT).map_err(failed)?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
			.map_err(failed)?;
	}
	Ok(())
}

/// Expand env vars in every string of the parsed tree.
fn expand_tree(
	value: &mut toml::Value,
	env: &dyn Fn(&str) -> Option<String>,
) -> Result<(), ConfigError> {
	match value {
		toml::Value::String(text) => *text = expand(text, env)?,
		toml::Value::Array(items) => {
			for item in items {
				expand_tree(item, env)?;
			}
		},
		toml::Value::Table(table) => {
			for (_, item) in table.iter_mut() {
				expand_tree(item, env)?;
			}
		},
		_ => {},
	}
	Ok(())
}

/// Expand `$NAME` / `${NAME}` / `$$` in one string.
fn expand(
	text: &str,
	env: &dyn Fn(&str) -> Option<String>,
) -> Result<String, ConfigError> {
	// Init buffer
	let mut out = String::with_capacity(text.len());
	let mut rest = text;

	// Scan for next variable
	while let Some(at) = rest.find('$') {
		out.push_str(&rest[..at]);
		rest = &rest[at + 1..];

		// Handle escape
		if let Some(tail) = rest.strip_prefix('$') {
			out.push('$');
			rest = tail;
			continue;
		}

		// Extract name
		let (name, tail) = match rest.strip_prefix('{') {
			Some(braced) => match braced.find('}') {
				Some(end) => (&braced[..end], &braced[end + 1..]),
				None => {
					return Err(ConfigError::BadVar {
						text: text.to_string(),
						why: "opens ${ and never closes it",
					});
				},
			},
			None => {
				let end = rest
					.find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
					.unwrap_or(rest.len());
				(&rest[..end], &rest[end..])
			},
		};

		// Handle bare delimiter
		if name.is_empty() {
			// Bare `$` is literal, `${}` is error
			if rest.starts_with('{') {
				return Err(ConfigError::BadVar {
					text: text.to_string(),
					why: "names no variable between ${ and }",
				});
			}
			out.push('$');
			rest = tail;
			continue;
		}

		// Substitute variable
		match env(name) {
			Some(value) => out.push_str(&value),
			None => return Err(ConfigError::UnsetVar(name.to_string())),
		}
		rest = tail;
	}

	// Append remainder
	out.push_str(rest);
	Ok(out)
}

/// Deserialize `effort`: `false` → `None`, level string → `Some`, `true` rejected.
fn effort<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	use serde::Deserialize;

	#[derive(Deserialize)]
	#[serde(untagged)]
	enum Raw {
		Off(bool),
		Level(String),
	}

	match Raw::deserialize(deserializer)? {
		Raw::Off(false) => Ok(None),
		Raw::Off(true) => Err(serde::de::Error::custom(
			"`effort = true` says nothing. Write `false` for no reasoning, or \
			 a level like \"low\".",
		)),
		Raw::Level(level) => Ok(Some(level)),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn stub(name: &str) -> Option<String> {
		match name {
			"XDG_STATE_HOME" => Some("/home/someone/.local/state".to_string()),
			"XDG_RUNTIME_DIR" => Some("/run/user/1000".to_string()),
			_ => None,
		}
	}

	fn default_config() -> Config {
		Config::parse_with(DEFAULT, &stub).expect("the shipped default parses")
	}

	#[test]
	fn the_shipped_default_is_a_configuration() {
		let config = default_config();
		assert_eq!(
			config.sandman.sqlite_path,
			PathBuf::from("/home/someone/.local/state/sandman/sandman.sqlite")
		);
		assert_eq!(
			config.sandman.control_socket,
			PathBuf::from("/run/user/1000/sandman/sandman.sock")
		);
		assert_eq!(config.metacognition.interrupt_interval, 15);
		assert!(config.channels.stdio && config.channels.web);
	}

	#[test]
	fn every_role_resolves_to_a_model() {
		let config = default_config();
		for role in <RoleName as strum::VariantArray>::VARIANTS {
			assert_eq!(
				config.for_role(*role).model,
				"Qwen3.6-35B-A3B:MXFP4_MOE"
			);
		}
		assert_eq!(config.for_comms().model, "Qwen3.6-35B-A3B:MXFP4_MOE");
		assert_eq!(config.for_grader().model, "z-ai/glm-5.3-flash");
	}

	#[test]
	fn a_named_role_overrides_all() {
		let text = DEFAULT.replace(
			"# research = \"qwen36-remote\"",
			"research = \"glm-flash\"",
		);
		let config = Config::parse_with(&text, &stub).unwrap();
		assert_eq!(
			config.for_role(RoleName::Research).model,
			"z-ai/glm-5.3-flash"
		);
		assert_eq!(
			config.for_role(RoleName::Planning).model,
			"Qwen3.6-35B-A3B:MXFP4_MOE"
		);
	}

	#[test]
	fn effort_off_is_no_reasoning_and_a_level_is_itself() {
		let config = default_config();
		assert_eq!(config.spec("qwen36-local").unwrap().effort, None);
		assert_eq!(
			config.spec("glm-flash").unwrap().effort,
			Some("low".to_string())
		);
	}

	#[test]
	fn effort_true_is_refused() {
		let text = DEFAULT.replace("effort = false", "effort = true");
		assert!(Config::parse_with(&text, &stub).is_err());
	}

	#[test]
	fn a_key_nobody_knows_is_an_error() {
		let text = format!("{DEFAULT}\n[sandman]\nnonsense = 1\n");
		assert!(Config::parse_with(&text, &stub).is_err());
	}

	#[test]
	fn a_missing_key_is_an_error() {
		let text = DEFAULT.replace("webui_port = 8080", "");
		assert!(Config::parse_with(&text, &stub).is_err());
	}

	#[test]
	fn a_slug_nothing_defines_is_an_error() {
		let text = DEFAULT.replace("all = \"qwen36-local\"", "all = \"nope\"");
		match Config::parse_with(&text, &stub) {
			Err(ConfigError::NoSuchSlug { key, slug }) => {
				assert_eq!(key, "models.all");
				assert_eq!(slug, "nope");
			},
			other => panic!("expected an unknown slug, got {other:?}"),
		}
	}

	#[test]
	fn an_unset_variable_is_an_error() {
		let text = DEFAULT.replace("$XDG_STATE_HOME", "$NOT_SET_ANYWHERE");
		match Config::parse_with(&text, &stub) {
			Err(ConfigError::UnsetVar(name)) => {
				assert_eq!(name, "NOT_SET_ANYWHERE")
			},
			other => panic!("expected an unset variable, got {other:?}"),
		}
	}

	#[test]
	fn expansion_reads_both_forms_and_escapes() {
		assert_eq!(
			expand("$XDG_STATE_HOME/db", &stub).unwrap(),
			"/home/someone/.local/state/db"
		);
		assert_eq!(
			expand("${XDG_STATE_HOME}x", &stub).unwrap(),
			"/home/someone/.local/statex"
		);
		assert_eq!(expand("$$5.00", &stub).unwrap(), "$5.00");
		assert_eq!(expand("100% $ free", &stub).unwrap(), "100% $ free");
		assert!(expand("${XDG_STATE_HOME", &stub).is_err());
		assert!(expand("${}", &stub).is_err());
	}
}
