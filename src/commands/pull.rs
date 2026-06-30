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
        eprintln!("[debug] api_url={}", ctx.api_url);
    }

    // A destination is mandatory. Once config loading exists this also accepts
    // `pull.path` from the config file.
    let Some(path) = &args.path else {
        return fail("Missing --path: no destination directory was provided");
    };

    let states: Vec<&str> = args.states.iter().map(|s| s.as_wire()).collect();
    println!(
        "pull: not yet implemented (skeleton) — path={} format={} languages={:?} \
         states={:?} empty_dir={}",
        path.display(),
        args.format.as_wire(),
        args.languages,
        states,
        args.empty_dir,
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
            api_url: "https://app.wordiy.dev".into(),
            api_key: None,
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
