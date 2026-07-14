//! Runtime context handed to each subcommand.
//!
//! Resolved configuration, credentials, and (later) an authed HTTP client live
//! here rather than in the parsed args, so each subcommand receives ready-to-use
//! dependencies. Global values follow the precedence CLI flag > env var > config
//! file > built-in default.

use crate::cli::GlobalArgs;
use crate::config::Config;

/// Default API base URL when nothing else overrides it.
pub const DEFAULT_API_URL: &str = "https://www.wordiy.dev";

#[derive(Debug)]
pub struct Context {
    pub api_url: String,
    pub api_key: Option<String>,
    pub verbose: bool,
}

impl Context {
    /// Resolve global settings: a flag/env value (already folded together by clap)
    /// wins; otherwise the config file; otherwise the built-in default.
    pub fn from_global(global: &GlobalArgs, config: &Config) -> Self {
        Self {
            api_url: global
                .api_url
                .clone()
                .or_else(|| config.api_url.clone())
                .unwrap_or_else(|| DEFAULT_API_URL.to_string()),
            api_key: global.api_key.clone().or_else(|| config.api_key.clone()),
            verbose: global.verbose,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn globals(api_url: Option<&str>, api_key: Option<&str>) -> GlobalArgs {
        GlobalArgs {
            api_key: api_key.map(String::from),
            api_url: api_url.map(String::from),
            config: None,
            verbose: false,
            extra_header: vec![],
        }
    }

    #[test]
    fn config_fills_in_when_flag_absent() {
        let cfg = Config {
            api_url: Some("https://from-config".into()),
            api_key: Some("srv_cfg".into()),
            ..Default::default()
        };
        let ctx = Context::from_global(&globals(None, None), &cfg);
        assert_eq!(ctx.api_url, "https://from-config");
        assert_eq!(ctx.api_key.as_deref(), Some("srv_cfg"));
    }

    #[test]
    fn flag_overrides_config() {
        let cfg = Config {
            api_url: Some("https://from-config".into()),
            ..Default::default()
        };
        let ctx = Context::from_global(&globals(Some("https://from-flag"), None), &cfg);
        assert_eq!(ctx.api_url, "https://from-flag");
    }

    #[test]
    fn default_when_neither_set() {
        let ctx = Context::from_global(&globals(None, None), &Config::default());
        assert_eq!(ctx.api_url, DEFAULT_API_URL);
        assert!(ctx.api_key.is_none());
    }
}
