//! Runtime context handed to each subcommand.
//!
//! Resolved configuration, credentials, and (later) an authed HTTP client live
//! here rather than in the parsed args, so each subcommand receives ready-to-use
//! dependencies. Today it carries the resolved base URL + API key; config-file
//! loading and the export client are added in subsequent steps.

use crate::cli::GlobalArgs;

/// Default API base URL when neither `--api-url` nor the environment overrides it.
///
/// TODO: temporary dev default for local testing — switch to `https://app.wordiy.dev`
/// before release.
pub const DEFAULT_API_URL: &str = "http://localhost:3001";

#[derive(Debug)]
pub struct Context {
    pub api_url: String,
    /// Consumed when the export client is built (Phase 2); unused for now.
    #[allow(dead_code)]
    pub api_key: Option<String>,
    pub verbose: bool,
}

impl Context {
    /// Build a context from the parsed global flags.
    ///
    /// Precedence (CLI > env > config file > default) is only partially realised
    /// here: clap already folds env vars into the flags, and the config layer joins
    /// once config loading exists.
    pub fn from_global(global: &GlobalArgs) -> Self {
        Self {
            api_url: global
                .api_url
                .clone()
                .unwrap_or_else(|| DEFAULT_API_URL.to_string()),
            api_key: global.api_key.clone(),
            verbose: global.verbose,
        }
    }
}
