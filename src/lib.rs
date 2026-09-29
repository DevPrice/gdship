//! Internals of the `gdship` binary, exposed for its integration tests. Not a stable API.

#[cfg(not(any(unix, windows)))]
compile_error!("gdship supports Unix and Windows only");

pub mod cli;
mod commands;
mod config;
mod configfile;
mod exe;
mod export;
mod godot;
mod godot_project;
mod process;
mod project;
pub mod report;

use std::fmt;

use anyhow::bail;

use crate::cli::{Cli, Command};
use crate::report::Reporter;

/// A mistake in how gdship was invoked that clap can't catch, such as `--only` naming a
/// channel the project doesn't have. It exits with status 2 like clap's own errors.
#[derive(Debug)]
pub struct UsageError(pub String);

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UsageError {}

/// Runs one parsed command.
pub fn run(cli: Cli, reporter: Reporter) -> anyhow::Result<()> {
    match cli.command {
        Command::Export(args) => commands::export(&args, reporter),
        Command::Push { .. } | Command::Login | Command::Status => {
            bail!("not implemented yet")
        }
    }
}
