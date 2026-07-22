//! wordiy CLI entry point.
//!
//! Layered startup: parse args, build a runtime `Context`, dispatch to a
//! subcommand. Argument-parsing failures are handled by clap (exit code 2);
//! application errors return from `run` and map to exit 1.

mod cli;
mod client;
mod commands;
mod config;
mod context;
mod error;
mod extract;
mod multipart;

use clap::Parser;
use std::process::ExitCode;

use crate::cli::{Cli, Command};
use crate::context::Context;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let verbose = cli.global.verbose;
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            if verbose {
                if let Some(detail) = err.detail() {
                    eprintln!("       {detail}");
                }
            }
            ExitCode::from(err.exit_code())
        }
    }
}

fn run(cli: Cli) -> error::Result<()> {
    match cli.command {
        Command::Pull(args) => {
            // `pull` needs config + credentials; loading here (not for `init`) also lets
            // `init` regenerate a config file that is currently broken.
            let loaded = config::load(cli.global.config.as_deref())?;
            let ctx = Context::from_global(&cli.global, &loaded.config);
            commands::pull::run(&ctx, &args, &loaded)
        }
        Command::Init(args) => commands::init::run(&args),
    }
}
