use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Export a Godot project and push each build to an itch.io channel with butler.
#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create gdship.toml for the project, asking which itch.io game it pushes to.
    Init,

    /// Export every channel into .gdship/build/ and verify the builds.
    Export(ExportArgs),

    /// Export every channel, then push each build to itch.io.
    Push {
        #[command(flatten)]
        export: ExportArgs,

        /// itch user version to push. Skips the git tag lookup.
        #[arg(long, value_name = "V", value_parser = parse_version, conflicts_with = "tag")]
        version: Option<String>,

        /// Create an annotated tag on HEAD, use it as the version, and push it to origin
        /// once every channel is pushed.
        #[arg(long, value_name = "TAG", value_parser = parse_tag, conflicts_with = "allow_dirty")]
        tag: Option<String>,

        /// Push even with uncommitted or untracked files.
        #[arg(long)]
        allow_dirty: bool,

        /// Check everything and print the Godot and butler commands without running them.
        #[arg(long)]
        dry_run: bool,
    },

    /// Log butler in to itch.io, opening the browser if needed.
    Login,

    /// Show butler's status for each channel.
    Status,
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    /// Only this channel. Can be repeated.
    #[arg(long = "only", value_name = "CHANNEL")]
    pub only: Vec<String>,

    /// Godot binary to use, overriding GDSHIP_GODOT and the user config.
    #[arg(long, value_name = "PATH")]
    pub godot: Option<PathBuf>,

    /// Stream Godot's output as well as logging it.
    #[arg(short, long)]
    pub verbose: bool,
}

impl Cli {
    /// Parses the process's arguments, exiting with status 2 on a usage error.
    pub fn parse_args() -> Self {
        Self::try_parse_args_from(std::env::args_os()).unwrap_or_else(|e| e.exit())
    }

    pub fn try_parse_args_from<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        Self::try_parse_from(args)
    }
}

fn parse_version(text: &str) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("must not be empty".into());
    }
    if text.starts_with('-') {
        return Err("must not start with `-`, which butler would read as an option".into());
    }
    if text.chars().any(char::is_control) {
        return Err("must not contain control characters".into());
    }
    Ok(text.to_owned())
}

/// Accepts names git would accept as a tag, minus the ones that would read as an option
/// on a command line.
fn parse_tag(tag: &str) -> Result<String, String> {
    let valid = !tag.is_empty()
        && !tag.starts_with(['-', '.', '/'])
        && !tag.ends_with(['.', '/'])
        && !tag.ends_with(".lock")
        && !tag.contains("..")
        && !tag.contains("//")
        && !tag.contains("@{")
        && tag != "@"
        && !tag
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c));
    if valid {
        Ok(tag.to_owned())
    } else {
        Err(format!("`{tag}` is not a valid git tag name"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_args_from(std::iter::once(&"gdship").chain(args))
    }

    #[test]
    fn push_takes_version_or_tag_but_not_both() {
        let cli = parse(&["push", "--version", "1.0", "--only", "a", "--only", "b"]).unwrap();
        let Command::Push {
            export, version, ..
        } = cli.command
        else {
            panic!("not push");
        };
        assert_eq!(version.as_deref(), Some("1.0"));
        assert_eq!(export.only, ["a", "b"]);

        let err = parse(&["push", "--version", "1", "--tag", "v1"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        let err = parse(&["push", "--tag", "v1", "--allow-dirty"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn rejects_bad_tags_and_versions() {
        for tag in ["-x", "a..b", "a b", "a:b", "v1.lock", "a/", ""] {
            assert!(parse(&["push", "--tag", tag]).is_err(), "{tag}");
        }
        assert!(parse(&["push", "--tag", "release/v1.2"]).is_ok());
        assert!(parse(&["push", "--version", "1\n2"]).is_err());
        assert!(parse(&["push", "--version=-x"]).is_err());
    }
}
