//! `wordiy pull` — download translations into a local directory.
//!
//! Skeleton stage: argument handling and the command shape are in place; the
//! export request + ZIP extraction pipeline is wired in a later step. For now the
//! handler reports what it parsed so the wiring can be verified manually.

use crate::cli::PullArgs;
use crate::context::Context;
use crate::error::{fail, Result};

pub fn run(ctx: &Context, args: &PullArgs) -> Result<()> {
    if ctx.verbose {
        eprintln!(
            "[debug] api_url={} project_id={:?} branch={:?}",
            ctx.api_url, ctx.project_id, ctx.branch
        );
    }

    // A destination is mandatory. Once config loading exists this also accepts
    // `pull.path` from the config file.
    let Some(path) = &args.path else {
        return fail("Missing --path: no destination directory was provided");
    };

    println!(
        "pull: not yet implemented (skeleton) — path={} format={} languages={:?} \
         states={:?} namespaces={:?} empty_dir={} watch={}",
        path.display(),
        args.format,
        args.languages,
        args.states,
        args.namespaces,
        args.empty_dir,
        args.watch,
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command};
    use clap::Parser;

    fn ctx() -> Context {
        Context {
            api_url: "https://app.wordiy.io".into(),
            api_key: None,
            project_id: None,
            branch: None,
            verbose: false,
        }
    }

    fn pull_args(argv: &[&str]) -> PullArgs {
        let Command::Pull(args) = Cli::try_parse_from(argv).expect("parses").command;
        args
    }

    #[test]
    fn errors_when_path_is_missing() {
        let args = pull_args(&["wordiy", "pull"]);
        let err = run(&ctx(), &args).expect_err("should require a path");
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn succeeds_when_path_is_present() {
        let args = pull_args(&["wordiy", "pull", "--path", "./i18n"]);
        assert!(run(&ctx(), &args).is_ok());
    }
}
