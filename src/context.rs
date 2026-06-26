//! Runtime context handed to each subcommand.
//!
//! Resolved configuration, credentials, and (later) an authed HTTP client live
//! here rather than in the parsed args, so each subcommand receives ready-to-use
//! dependencies. Today it is a thin placeholder carrying the global flags;
//! config-file loading, credential resolution, and the API client are added in
//! subsequent steps.

use crate::cli::GlobalArgs;

/// Placeholder default until config loading + a real default are wired in.
pub const DEFAULT_API_URL: &str = "https://app.wordiy.io";

#[derive(Debug)]
pub struct Context {
    pub api_url: String,
    /// Consumed when the authed HTTP client is built (step 3); unused in the skeleton.
    #[allow(dead_code)]
    pub api_key: Option<String>,
    pub project_id: Option<u32>,
    pub branch: Option<String>,
    pub verbose: bool,
}

impl Context {
    /// Build a context from the parsed global flags.
    ///
    /// Precedence (CLI > env > config file > stored credential > default) is only
    /// partially realised here: clap already folds env vars into the flags, and
    /// the remaining layers join once config + credential stores exist.
    pub fn from_global(global: &GlobalArgs) -> Self {
        Self {
            api_url: global
                .api_url
                .clone()
                .unwrap_or_else(|| DEFAULT_API_URL.to_string()),
            api_key: global.api_key.clone(),
            project_id: global.project_id,
            branch: global.branch.clone(),
            verbose: global.verbose,
        }
    }
}
