//! wordiy CLI entry point.
//!
//! Layered startup: parse args, build a runtime `Context`, dispatch to a
//! subcommand. Argument-parsing failures are handled by clap (exit code 2);
//! application errors return from `run` and map to exit 1.

mod cli;
mod commands;
mod context;
mod error;

use clap::Parser;
use std::process::ExitCode;

use crate::cli::{Cli, Command};
use crate::context::Context;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(err.exit_code())
        }
    }
}

fn run(cli: Cli) -> error::Result<()> {
    // The full Context (config file + resolved credentials + HTTP client) is built
    // in later steps; for now it carries the global flags only.
    let ctx = Context::from_global(&cli.global);

    match cli.command {
        Command::Pull(args) => commands::pull::run(&ctx, &args),
    }
}
