//! Internals of the `gdship` binary, exposed for its integration tests. Not a stable API.

#[cfg(not(any(unix, windows)))]
compile_error!("gdship supports Unix and Windows only");

pub mod cli;
#[allow(dead_code, reason = "the commands start using it in a later commit")]
mod config;
mod configfile;
#[allow(dead_code, reason = "the commands start using it in a later commit")]
mod godot_project;
#[allow(dead_code, reason = "the commands start using it in a later commit")]
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
pub fn run(cli: Cli, _reporter: Reporter) -> anyhow::Result<()> {
    match cli.command {
        Command::Export(_) | Command::Push { .. } | Command::Login | Command::Status => {
            bail!("not implemented yet")
        }
    }
}
