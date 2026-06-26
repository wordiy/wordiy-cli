//! Command-line surface for `wordiy`, defined with clap's derive API.
//!
//! `GlobalArgs` are marked `global = true` so they may appear before or after the
//! subcommand. They are kept separate from per-command args and resolved into a
//! [`crate::context::Context`].

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

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
    /// API key (project key or personal access token).
    #[arg(long, short = 'k', global = true, env = "WORDIY_API_KEY")]
    pub api_key: Option<String>,

    /// Project ID (required for a personal access token; derived from a project key).
    #[arg(long = "project-id", short = 'p', global = true)]
    pub project_id: Option<u32>,

    /// API base URL.
    #[arg(long = "api-url", short = 'u', global = true, env = "WORDIY_API_URL")]
    pub api_url: Option<String>,

    /// Project branch.
    #[arg(long, short = 'b', global = true, env = "WORDIY_BRANCH")]
    pub branch: Option<String>,

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

/// Arguments for `wordiy pull`.
///
/// The full flag surface is declared now so `--help` and parsing are complete;
/// the export + unzip pipeline is wired in a later step. Domain enums (format,
/// state) are still plain strings here and become typed enums when the request
/// is built.
#[derive(Debug, Args)]
pub struct PullArgs {
    /// Destination directory for the downloaded files.
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Languages to export (repeatable); omit to export all.
    #[arg(long, short = 'l')]
    pub languages: Vec<String>,

    /// Translation states to include (repeatable): UNTRANSLATED | TRANSLATED | REVIEWED.
    #[arg(long, short = 's')]
    pub states: Vec<String>,

    /// Structure delimiter for nested keys (`''` disables nesting).
    #[arg(long, short = 'd')]
    pub delimiter: Option<String>,

    /// Namespaces to export (repeatable); empty string targets the default namespace.
    #[arg(long, short = 'n')]
    pub namespaces: Vec<String>,

    /// Only export keys having these tags (repeatable).
    #[arg(long, short = 't')]
    pub tags: Vec<String>,

    /// Exclude keys having these tags (repeatable).
    #[arg(long = "exclude-tags")]
    pub exclude_tags: Vec<String>,

    /// Export arrays as arrays where the format supports it.
    #[arg(long = "support-arrays")]
    pub support_arrays: bool,

    /// Empty the destination directory before extracting (destructive).
    #[arg(long = "empty-dir")]
    pub empty_dir: bool,

    /// Server-side file layout template (e.g. `{namespace}/{languageTag}.{extension}`).
    #[arg(long = "file-structure-template")]
    pub file_structure_template: Option<String>,

    /// Localization format.
    #[arg(long, default_value = "JSON_WORDIY")]
    pub format: String,

    /// Keep running and re-pull on remote changes.
    #[arg(long)]
    pub watch: bool,
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
        let cli = Cli::try_parse_from([
            "wordiy", "pull", "--path", "./i18n", "--project-id", "42", "--api-key", "pk_x",
        ])
        .expect("should parse");

        assert_eq!(cli.global.project_id, Some(42));
        assert_eq!(cli.global.api_key.as_deref(), Some("pk_x"));

        let Command::Pull(args) = cli.command;
        assert_eq!(args.path.as_deref(), Some(std::path::Path::new("./i18n")));
        assert_eq!(args.format, "JSON_WORDIY"); // default
        assert!(!args.empty_dir);
    }

    #[test]
    fn parses_pull_filters() {
        let cli = Cli::try_parse_from([
            "wordiy", "pull", "-l", "en", "-l", "fr", "--format", "JSON_ICU", "--empty-dir",
        ])
        .expect("should parse");

        let Command::Pull(args) = cli.command;
        assert_eq!(args.languages, vec!["en", "fr"]);
        assert_eq!(args.format, "JSON_ICU");
        assert!(args.empty_dir);
        assert!(!args.support_arrays);
    }
}
