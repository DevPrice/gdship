//! Internals of the `gdship` binary, exposed for its integration tests. Not a stable API.

#[cfg(not(any(unix, windows)))]
compile_error!("gdship supports Unix and Windows only");

mod addons;
mod archive;
pub mod butler;
pub mod cli;
mod commands;
mod config;
mod configfile;
pub mod digest;
mod exe;
mod export;
pub mod fetch;
mod git;
mod godot;
mod godot_project;
mod process;
mod project;
pub mod report;

use std::fmt;

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

/// How a command finished when it did not hit a hard error.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    /// The command completed but must exit non-zero, e.g. every channel was pushed but
    /// the tag could not be. The reason was already reported.
    Failure,
}

/// Runs one parsed command.
pub fn run(cli: Cli, reporter: Reporter) -> anyhow::Result<Outcome> {
    match cli.command {
        Command::Export(args) => commands::export(&args, reporter),
        Command::Push {
            export,
            version,
            tag,
            allow_dirty,
            dry_run,
        } => commands::push(
            &commands::PushOptions {
                export: &export,
                version: version.as_deref(),
                tag: tag.as_deref(),
                allow_dirty,
                dry_run,
            },
            reporter,
        ),
        Command::Login => commands::login(reporter),
        Command::Status => commands::status(reporter),
    }
}
