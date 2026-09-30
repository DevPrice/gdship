use std::fmt::Display;

use anstyle::{AnsiColor, Style};

const VERB: Style = AnsiColor::Green.on_default().bold();
const WARNING: Style = AnsiColor::Yellow.on_default().bold();
const ERROR: Style = AnsiColor::Red.on_default().bold();

/// Width of the right-aligned verb column, e.g. "   Exporting html5".
const VERB_WIDTH: usize = 12;

/// Writes progress to stdout and diagnostics to stderr, one plain line per event.
///
/// Color is decided per stream by `anstream` (TTY, `NO_COLOR`, `CLICOLOR_FORCE`). Under
/// GitHub Actions, warnings and errors are emitted as workflow annotations instead.
#[derive(Debug, Clone, Copy)]
pub struct Reporter {
    github_actions: bool,
}

impl Reporter {
    pub fn from_env() -> Self {
        Self {
            github_actions: std::env::var_os("GITHUB_ACTIONS").is_some_and(|v| v == "true"),
        }
    }

    pub fn action(&self, verb: &str, subject: impl Display) {
        let subject = escape_controls(&subject.to_string());
        anstream::println!("{VERB}{verb:>VERB_WIDTH$}{VERB:#} {subject}");
    }

    pub fn warn(&self, message: impl Display) {
        if self.github_actions {
            anstream::eprintln!("::warning::{}", escape_annotation(&message.to_string()));
        } else {
            anstream::eprintln!("{WARNING}warning{WARNING:#}: {message}");
        }
    }

    pub fn error(&self, message: impl Display) {
        if self.github_actions {
            anstream::eprintln!("::error::{}", escape_annotation(&message.to_string()));
        } else {
            anstream::eprintln!("{ERROR}error{ERROR:#}: {message}");
        }
    }
}

/// Shows control characters as escapes, so text from a project, such as a preset name,
/// can neither start a line GitHub Actions reads as a workflow command nor send the
/// terminal escape sequences.
fn escape_controls(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() {
                c.escape_default().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// Escapes a workflow-command message so multi-line text stays one annotation.
/// See https://github.com/actions/toolkit/blob/main/packages/core/src/command.ts
fn escape_annotation(message: &str) -> String {
    message
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotation_escapes_percent_before_newlines() {
        assert_eq!(escape_annotation("50%\r\nnext"), "50%25%0D%0Anext");
    }

    #[test]
    fn control_characters_are_escaped_but_other_text_is_kept() {
        assert_eq!(
            escape_controls("x\n::add-mask::y\r\u{1b}[31m"),
            "x\\n::add-mask::y\\r\\u{1b}[31m"
        );
        assert_eq!(escape_controls("日本 \"Web\""), "日本 \"Web\"");
    }
}
