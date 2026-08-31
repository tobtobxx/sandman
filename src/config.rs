//! `config.toml`: everything about the world Sandman runs in.
//!
//! Which models, where the database and the trace go, which Channels open, what
//! the Watcher listens on. Not policy — nothing here decides what the swarm
//! does, only what it is made of.
//!
//! **Nothing has a built-in fallback.** A missing key is an error at start, and
//! an unknown key is one too. The one place a default lives is
//! `default-config.toml`, compiled in with `include_str!` and written out the
//! first time Sandman finds no configuration — after which it stops, because
//! there is nothing sensible to run before a human has read that file. The
//! exception is `[models]`, where a Role left out falls back to `all`; absence
//! there *means* something, which is why it is allowed to mean it.
//!
//! Any string may name an environment variable — `$NAME`, `${NAME}`, `$$` for a
//! literal `$`. A variable that is not set is an error rather than an empty
//! string: `$XDG_STATE_HOME/sandman/sandman.sqlite` silently becoming
//! `/sandman/sandman.sqlite` is how a database ends up somewhere nobody meant.
//! Expansion happens on the parsed tree before it becomes a [`Config`], so it
//! reaches every string in the file and every string added to it later.
//!
//! Defines: [`Config`], [`ModelSpec`], [`ConfigError`], [`DEFAULT`].

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use crate::roles::RoleName;

/// The configuration written out when there is none. The only defaults there
/// are, and they are text rather than code so that what a human reads and what
/// Sandman runs cannot drift apart.
pub const DEFAULT: &str = include_str!("default-config.toml");

/// The whole file.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
	/// Every model this Sandman knows, by slug. Everything else names one of
	/// these; a slug nothing names is kept and not complained about.
	pub model: BTreeMap<String, ModelSpec>,
	pub models: ModelChoices,
	pub sandman: Sandman,
	pub metacognition: Metacognition,
	pub embedding: Embedding,
	pub channels: Channels,
	pub tools: Tools,
	pub bench: Bench,
}

/// One model, and how to reach it.
///
/// Hashed by everything it holds, so two purposes that name the same model —
/// whether by the same slug or by two slugs that say the same thing — share one
/// adapter. See [`crate::model::Models`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSpec {
	/// A chat-completions URL.
	pub endpoint: String,
	/// Sent as `Authorization: Bearer`. May be empty for a local endpoint.
	pub api_key: String,
	/// What the endpoint calls the model.
	pub model: String,
	/// How much the model may think before it answers. `false` in the file is
	/// `None` here and asks for no reasoning at all; anything else is sent as
	/// written.
	#[serde(deserialize_with = "effort")]
	pub effort: Option<String>,
}

/// Which model does which work.
///
/// The Roles are named fields rather than a map, so [`ModelChoices::for_role`]
/// matches [`RoleName`] exhaustively: a Role added without a line here does not
/// compile.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelChoices {
	/// What every Session uses unless one of the rest names another.
	pub all: String,
	/// The Session that talks to a human.
	pub comms: Option<String>,
	pub planning: Option<String>,
	pub research: Option<String>,
	pub memory: Option<String>,
	pub task_manager: Option<String>,
}

/// Where Sandman keeps things, and what it listens on.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sandman {
	pub sqlite_path: PathBuf,
	pub log_path: PathBuf,
	/// How `sandman task`, `sandman list` and `sandman spend` reach a Sandman
	/// that is already running.
	pub control_socket: PathBuf,
	pub webui_address: IpAddr,
	pub webui_port: u16,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metacognition {
	/// How many messages may pass with no review and no interrupt before one
	/// fires.
	pub interrupt_interval: usize,
	pub model: String,
}

/// What the `memory` Role searches with. Not a chat model: its own endpoint,
/// and no effort to set.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Embedding {
	pub endpoint: String,
	pub api_key: String,
	pub model: String,
	/// Longer inputs cost more and embed worse. The cap is here so one runaway
	/// Brief cannot fail a whole batch.
	pub max_input_chars: usize,
}

/// The ways a human reaches the swarm.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Channels {
	/// The terminal. Turned off, the trace goes to stdout as well as to the
	/// log, because nothing else is using it.
	pub stdio: bool,
	/// The Watcher's chat pane. The UI is served either way; this only decides
	/// whether that pane is a Channel or says it is turned off.
	pub web: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
	/// What `web_search` asks.
	pub searxng_endpoint: String,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bench {
	/// The grader's model, by slug.
	pub grader: String,
}

/// Why Sandman has no configuration to run on.
///
/// [`ConfigError::Written`] is not a failure of anything — it is the first-start
/// path, and it is an error because it is a reason to stop.
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
		"there is nowhere to keep a configuration: neither $XDG_CONFIG_HOME \
		 nor $HOME is set. Name one with --config."
	)]
	Nowhere,
}

impl Config {
	/// Where the configuration lives: `--config`, else
	/// `$XDG_CONFIG_HOME/sandman/config.toml`, else `~/.config/sandman/config.toml`.
	pub fn path(flag: Option<PathBuf>) -> Result<PathBuf, ConfigError> {
		if let Some(path) = flag {
			return Ok(path);
		}
		if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
			return Ok(PathBuf::from(dir).join("sandman").join("config.toml"));
		}
		if let Ok(home) = std::env::var("HOME") {
			return Ok(PathBuf::from(home)
				.join(".config")
				.join("sandman")
				.join("config.toml"));
		}
		Err(ConfigError::Nowhere)
	}

	/// Read the configuration, or write the default one and stop.
	///
	/// Writing happens only when there is no file at all. A file that is there
	/// and will not parse is left exactly as it is: overwriting a configuration
	/// a human is in the middle of editing would be the worst possible answer to
	/// a missing comma.
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

	/// The configuration as it is, and nothing written if it is not there.
	///
	/// What a bench reads: a case is not the place to create a human's
	/// configuration, and one that cannot find it should say so and stop.
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

	/// Parse one configuration: expand what it names, then read it, then check
	/// that every slug it names exists.
	pub fn parse(text: &str) -> Result<Config, ConfigError> {
		Config::parse_with(text, &|name| std::env::var(name).ok())
	}

	/// The same, against an environment the caller supplies. For anything that
	/// must not read the one it is running in — a test, or a configuration read
	/// on behalf of somewhere else.
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

	/// Every slug this configuration names, and where it named it.
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

	/// A model named by something that has no table is an error at start, not a
	/// failed call in the middle of a run.
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

	/// One model by slug.
	pub fn spec(&self, slug: &str) -> Option<&ModelSpec> {
		self.model.get(slug)
	}

	/// The model named by `models.all` — what a Session uses when nothing names
	/// another, and the name a Run is recorded under.
	pub fn for_all(&self) -> &ModelSpec {
		self.resolve(&self.models.all)
	}

	/// The model a Role's Workers talk to.
	pub fn for_role(&self, role: RoleName) -> &ModelSpec {
		self.resolve(self.models.named_for(role).unwrap_or(&self.models.all))
	}

	/// The model a Comms Session talks to.
	pub fn for_comms(&self) -> &ModelSpec {
		self.resolve(self.models.comms.as_deref().unwrap_or(&self.models.all))
	}

	/// The model a review or an interrupt talks to.
	pub fn for_metacognition(&self) -> &ModelSpec {
		self.resolve(&self.metacognition.model)
	}

	/// The model a bench grader talks to.
	pub fn for_grader(&self) -> &ModelSpec {
		self.resolve(&self.bench.grader)
	}

	/// Every slug is checked at load, so by the time anything asks, it is there.
	fn resolve(&self, slug: &str) -> &ModelSpec {
		self.spec(slug).expect("slugs are checked at load")
	}
}

impl ModelChoices {
	/// The slug named for this Role, if one is. Matched exhaustively: a Role
	/// with no line in this struct does not compile.
	fn named_for(&self, role: RoleName) -> Option<&str> {
		match role {
			RoleName::Research => self.research.as_deref(),
			RoleName::Planning => self.planning.as_deref(),
			RoleName::Memory => self.memory.as_deref(),
			RoleName::TaskManager => self.task_manager.as_deref(),
		}
	}
}

/// Write the default configuration, and the directory it goes in.
///
/// Readable and writable by its owner alone: it carries API keys.
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

/// Expand every string in the tree, keys excepted — a slug is a name in this
/// file, not a name in the environment.
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

/// One string, with what it names put in.
///
/// `$NAME` and `${NAME}` are the environment; `$$` is a literal `$`; a `$`
/// before anything that is not a name is itself. A name that is not set is an
/// error — an empty string in a path is how a database ends up somewhere nobody
/// meant.
fn expand(
	text: &str,
	env: &dyn Fn(&str) -> Option<String>,
) -> Result<String, ConfigError> {
	let mut out = String::with_capacity(text.len());
	let mut rest = text;

	while let Some(at) = rest.find('$') {
		out.push_str(&rest[..at]);
		rest = &rest[at + 1..];

		if let Some(tail) = rest.strip_prefix('$') {
			out.push('$');
			rest = tail;
			continue;
		}

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

		if name.is_empty() {
			// `$` before anything that cannot start a name is a `$`. `${}` is
			// not that — it is someone meaning a name and writing none.
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

		match env(name) {
			Some(value) => out.push_str(&value),
			None => return Err(ConfigError::UnsetVar(name.to_string())),
		}
		rest = tail;
	}

	out.push_str(rest);
	Ok(out)
}

/// `false` means no reasoning at all; anything else is a level, sent as written.
///
/// `true` is refused. It would have to mean "some amount, you choose", and
/// nothing here can choose.
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

	/// The environment the shipped default names. Stubbed rather than set, so
	/// the test says what it depends on and no other test can see it.
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
