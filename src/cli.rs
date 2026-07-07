//! Command-line surface for `wordiy`, defined with clap's derive API.
//!
//! `GlobalArgs` are marked `global = true` so they may appear before or after the
//! subcommand. They are kept separate from per-command args and resolved into a
//! [`crate::context::Context`].
//!
//! The surface tracks the v1 export contract: the project is bound to the API key
//! (no project id), and only the format + `languages` + `filterState` filters take
//! effect, so unsupported flags are intentionally absent until the backend grows them.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Top-level parser.
#[derive(Debug, Parser)]
#[command(name = "wordiy", version, about = "Interact with the wordiy translation platform")]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

/// Flags shared by every subcommand.
#[derive(Debug, Args)]
pub struct GlobalArgs {
    /// API key for the project (e.g. a `srv_` server key).
    #[arg(long, short = 'k', global = true, env = "WORDIY_API_KEY")]
    pub api_key: Option<String>,

    /// API base URL.
    #[arg(long = "api-url", short = 'u', global = true, env = "WORDIY_API_URL")]
    pub api_url: Option<String>,

    /// Path to a config file (defaults to auto-discovery of `.wordiyrc`).
    #[arg(long, short = 'c', global = true)]
    pub config: Option<PathBuf>,

    /// Enable verbose/debug logging.
    #[arg(long, short = 'v', global = true)]
    pub verbose: bool,

    /// Extra header to send on every request (`Name: Value`); repeatable.
    #[arg(long = "extra-header", short = 'H', global = true)]
    pub extra_header: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Download translations from wordiy into a local directory.
    Pull(PullArgs),
}

/// Export container format. v1 supports Android resources XML only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum Format {
    #[default]
    #[value(name = "ANDROID_XML")]
    AndroidXml,
}

impl Format {
    /// The value sent in the export request body.
    pub fn as_wire(self) -> &'static str {
        match self {
            Format::AndroidXml => "ANDROID_XML",
        }
    }
}

/// Translation state filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum State {
    #[value(name = "UNTRANSLATED")]
    Untranslated,
    #[value(name = "TRANSLATED")]
    Translated,
    #[value(name = "REVIEWED")]
    Reviewed,
    #[value(name = "DISABLED")]
    Disabled,
}

impl State {
    /// The value sent in the export request body.
    pub fn as_wire(self) -> &'static str {
        match self {
            State::Untranslated => "UNTRANSLATED",
            State::Translated => "TRANSLATED",
            State::Reviewed => "REVIEWED",
            State::Disabled => "DISABLED",
        }
    }
}

/// Arguments for `wordiy pull`.
#[derive(Debug, Args)]
pub struct PullArgs {
    /// Destination directory for the downloaded files.
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Languages to export (repeatable); omit to export all.
    #[arg(long, short = 'l')]
    pub languages: Vec<String>,

    /// Translation states to include (repeatable); omit for the server default
    /// (translated + reviewed).
    #[arg(long, short = 's')]
    pub states: Vec<State>,

    /// Only export keys carrying at least one of these tags (repeatable);
    /// matched by name, case-insensitive.
    #[arg(long, short = 't')]
    pub tags: Vec<String>,

    /// Exclude keys carrying any of these tags (repeatable); matched by name,
    /// case-insensitive. Applied together with --tags.
    #[arg(long = "exclude-tags")]
    pub exclude_tags: Vec<String>,

    /// Only export keys whose name starts with this prefix (anchored,
    /// case-sensitive, literal — `_`/`%` are not wildcards).
    #[arg(long = "key-prefix")]
    pub key_prefix: Option<String>,

    /// Export format [default: ANDROID_XML].
    #[arg(long)]
    pub format: Option<Format>,

    /// Empty the destination directory before extracting (destructive).
    #[arg(long = "empty-dir", overrides_with = "no_empty_dir")]
    pub empty_dir: bool,

    /// Do not empty the destination directory (overrides `emptyDir` from the config).
    #[arg(long = "no-empty-dir", overrides_with = "empty_dir")]
    pub no_empty_dir: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_pull_with_globals_after_subcommand() {
        let cli = Cli::try_parse_from(["wordiy", "pull", "--path", "./i18n", "--api-key", "srv_x"])
            .expect("should parse");

        assert_eq!(cli.global.api_key.as_deref(), Some("srv_x"));

        let Command::Pull(args) = cli.command;
        assert_eq!(args.path.as_deref(), Some(std::path::Path::new("./i18n")));
        assert_eq!(args.format, None); // unset on the CLI → resolved later (default ANDROID_XML)
        assert!(!args.empty_dir);
    }

    #[test]
    fn parses_pull_filters() {
        let cli = Cli::try_parse_from([
            "wordiy", "pull", "-l", "en", "-l", "ar", "-s", "TRANSLATED", "-s", "REVIEWED",
            "-t", "checkout", "--exclude-tags", "legacy", "--key-prefix", "home_",
            "--format", "ANDROID_XML", "--empty-dir",
        ])
        .expect("should parse");

        let Command::Pull(args) = cli.command;
        assert_eq!(args.languages, vec!["en", "ar"]);
        assert_eq!(args.states, vec![State::Translated, State::Reviewed]);
        assert_eq!(args.tags, vec!["checkout"]);
        assert_eq!(args.exclude_tags, vec!["legacy"]);
        assert_eq!(args.key_prefix.as_deref(), Some("home_"));
        assert_eq!(args.format, Some(Format::AndroidXml));
        assert!(args.empty_dir);
    }

    #[test]
    fn rejects_unsupported_format() {
        // Only ANDROID_XML is accepted in v1.
        assert!(Cli::try_parse_from(["wordiy", "pull", "--format", "JSON_ICU"]).is_err());
    }

    #[test]
    fn empty_dir_negation_parses() {
        let Command::Pull(args) = Cli::try_parse_from(["wordiy", "pull", "--no-empty-dir"])
            .expect("should parse")
            .command;
        assert!(args.no_empty_dir);
        assert!(!args.empty_dir);
    }
}
