//! `.wordiyrc.toml` configuration: file-based defaults for CLI options, so common
//! flags don't have to be repeated. Config values are overridden by CLI flags and
//! environment variables (see the resolution in each command).
//!
//! Discovery searches from the current directory up the tree for `.wordiyrc.toml`
//! (or `.wordiyrc`); `--config <path>` loads an exact file instead. Keys are
//! camelCase, mirroring the flags.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{CliError, Result};

/// The recognized config file names, in priority order per directory.
const FILE_NAMES: [&str; 2] = [".wordiyrc.toml", ".wordiyrc"];

/// Parsed config file. Every field is optional; unknown keys are rejected so typos
/// surface as errors.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub api_url: Option<String>,
    pub api_key: Option<String>,
    #[serde(default)]
    pub pull: PullConfig,
    #[serde(default)]
    pub push: PushConfig,
}

/// The `[pull]` table.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PullConfig {
    pub path: Option<PathBuf>,
    pub format: Option<String>,
    pub languages: Option<Vec<String>>,
    pub states: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub exclude_tags: Option<Vec<String>>,
    pub key_prefix: Option<String>,
    pub empty_dir: Option<bool>,
}

/// The `[push]` table.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PushConfig {
    pub path: Option<PathBuf>,
    pub force_mode: Option<String>,
}

/// A parsed config plus the directory it was found in. Relative `pull.path` values
/// resolve against `dir`.
pub struct LoadedConfig {
    pub config: Config,
    pub dir: PathBuf,
}

/// Load configuration. With `explicit`, load exactly that file (error if it can't be
/// read or parsed). Otherwise search from the current directory upward; if nothing is
/// found, return defaults.
pub fn load(explicit: Option<&Path>) -> Result<LoadedConfig> {
    let found = match explicit {
        Some(p) => Some(p.to_path_buf()),
        None => find_config(&current_dir()?),
    };

    match found {
        Some(path) => {
            let config = parse_file(&path)?;
            let dir = path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));
            Ok(LoadedConfig { config, dir })
        }
        None => Ok(LoadedConfig {
            config: Config::default(),
            dir: current_dir()?,
        }),
    }
}

/// Search `start` and each ancestor for a recognized config file.
fn find_config(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        for name in FILE_NAMES {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn parse_file(path: &Path) -> Result<Config> {
    let text = fs::read_to_string(path)
        .map_err(|e| CliError::Message(format!("could not read config {}: {e}", path.display())))?;
    toml::from_str(&text)
        .map_err(|e| CliError::Message(format!("invalid config {}: {e}", path.display())))
}

fn current_dir() -> Result<PathBuf> {
    std::env::current_dir()
        .map_err(|e| CliError::Message(format!("could not read the current directory: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("wordiy_config_{tag}_{}_{n}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn parses_a_full_config() {
        let toml = r#"
            apiUrl = "https://www.wordiy.dev"
            apiKey = "srv_x"

            [pull]
            path = "res"
            format = "ANDROID_XML"
            languages = ["en", "ar"]
            states = ["TRANSLATED", "REVIEWED"]
            tags = ["mobile"]
            excludeTags = ["legacy"]
            keyPrefix = "home_"
            emptyDir = true
        "#;
        let c: Config = toml::from_str(toml).unwrap();
        assert_eq!(c.api_url.as_deref(), Some("https://www.wordiy.dev"));
        assert_eq!(c.api_key.as_deref(), Some("srv_x"));
        assert_eq!(c.pull.path.as_deref(), Some(Path::new("res")));
        assert_eq!(c.pull.format.as_deref(), Some("ANDROID_XML"));
        assert_eq!(c.pull.exclude_tags.as_deref(), Some(&["legacy".to_string()][..]));
        assert_eq!(c.pull.key_prefix.as_deref(), Some("home_"));
        assert_eq!(c.pull.empty_dir, Some(true));
    }

    #[test]
    fn empty_config_is_all_defaults() {
        let c: Config = toml::from_str("").unwrap();
        assert!(c.api_url.is_none() && c.api_key.is_none() && c.pull.path.is_none());
    }

    #[test]
    fn unknown_key_is_rejected() {
        // `apUrl` is a typo for `apiUrl`.
        assert!(toml::from_str::<Config>(r#"apUrl = "x""#).is_err());
    }

    #[test]
    fn find_config_walks_up_the_tree() {
        let base = temp_dir("walkup");
        let child = base.join("a/b");
        fs::create_dir_all(&child).unwrap();
        fs::write(base.join(".wordiyrc.toml"), "apiUrl = \"x\"").unwrap();

        assert_eq!(find_config(&child), Some(base.join(".wordiyrc.toml")));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn load_explicit_path_records_its_dir() {
        let base = temp_dir("explicit");
        fs::create_dir_all(&base).unwrap();
        let file = base.join("custom.toml");
        fs::write(&file, "apiKey = \"srv_z\"").unwrap();

        let loaded = load(Some(&file)).unwrap();
        assert_eq!(loaded.config.api_key.as_deref(), Some("srv_z"));
        assert_eq!(loaded.dir, base);
        let _ = fs::remove_dir_all(&base);
    }
}
