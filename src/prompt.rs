use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::{ItchTarget, UserConfig};
use crate::godot::Godot;

/// Wrong answers accepted before giving up.
const ATTEMPTS: usize = 3;

/// Whether gdship can ask the user questions: both stdin and stderr, where questions
/// go, must be a terminal. CI runs never are.
pub(crate) fn is_interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Asks `question` until `check` accepts an answer, explaining each rejection. An
/// empty answer or the end of input gives up.
fn ask<T>(
    input: &mut impl BufRead,
    output: &mut impl Write,
    question: &str,
    mut check: impl FnMut(&str) -> Result<T, String>,
) -> Result<T> {
    let mut problem = None;
    for _ in 0..ATTEMPTS {
        if let Some(problem) = problem.take() {
            writeln!(output, "{problem}")?;
        }
        write!(output, "{question}: ")?;
        output.flush()?;
        let mut line = String::new();
        let read = input
            .read_line(&mut line)
            .context("cannot read the answer")?;
        let answer = clean_answer(&line);
        if read == 0 || answer.is_empty() {
            bail!("no answer given");
        }
        match check(answer) {
            Ok(value) => return Ok(value),
            Err(err) => problem = Some(err),
        }
    }
    bail!("{}", problem.expect("every attempt left a problem"))
}

/// Asks for the Godot binary until one answers `--version`, then saves it to the user
/// config at `config_path` so later runs find it.
pub(crate) fn ask_for_godot(
    input: &mut impl BufRead,
    output: &mut impl Write,
    config_path: Option<&Path>,
) -> Result<Godot> {
    writeln!(
        output,
        "gdship needs a Godot 4 binary, and none is configured."
    )?;
    let godot = ask(input, output, "Path to Godot", |answer| {
        let path = executable_in(PathBuf::from(answer));
        if !path.is_file() {
            return Err(format!("{} is not a file.", path.display()));
        }
        Godot::at(&path).map_err(|err| format!("{err:#}"))
    })?;
    save(&godot.path, config_path, output)?;
    Ok(godot)
}

/// Asks which itch.io game to push to, as `<user>/<game>` or the game's address.
pub(crate) fn ask_for_itch(
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<ItchTarget> {
    ask(
        input,
        output,
        "itch.io game, as <user>/<game> or its itch.io address",
        |answer| parse_itch(answer).map_err(|err| format!("{err:#}")),
    )
}

/// Accepts `devprice/idle-factory` or `https://devprice.itch.io/idle-factory`.
fn parse_itch(answer: &str) -> Result<ItchTarget> {
    let address = answer
        .strip_prefix("https://")
        .or_else(|| answer.strip_prefix("http://"))
        .unwrap_or(answer);
    if let Some((host, path)) = address.split_once('/')
        && let Some(user) = host.to_ascii_lowercase().strip_suffix(".itch.io")
    {
        let game = path.split(['/', '?', '#']).next().unwrap_or_default();
        return format!("{user}/{game}").parse();
    }
    answer.parse()
}

fn save(godot: &Path, config_path: Option<&Path>, output: &mut impl Write) -> Result<()> {
    match config_path {
        Some(config_path) => {
            let path = std::path::absolute(godot).unwrap_or_else(|_| godot.to_owned());
            UserConfig::save_godot(config_path, &path)?;
            writeln!(output, "Saved it to {}", config_path.display())?;
        }
        None => writeln!(
            output,
            "Not saved: there is no user config folder; set GDSHIP_CONFIG to choose one."
        )?,
    }
    Ok(())
}

/// Trims the answer and one pair of surrounding quotes, which Windows' "Copy as path"
/// and dragging a file into a terminal both add.
fn clean_answer(line: &str) -> &str {
    let answer = line.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = answer
            .strip_prefix(quote)
            .and_then(|a| a.strip_suffix(quote))
        {
            return inner;
        }
    }
    answer
}

/// Accepts a macOS `Godot.app` bundle by looking for the binary inside it.
fn executable_in(path: PathBuf) -> PathBuf {
    if path.is_dir() && path.extension().is_some_and(|e| e == "app") {
        let binary = path.join("Contents").join("MacOS").join("Godot");
        if binary.is_file() {
            return binary;
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_lose_whitespace_and_quotes() {
        assert_eq!(
            clean_answer("  C:\\Apps\\godot.exe \r\n"),
            "C:\\Apps\\godot.exe"
        );
        assert_eq!(
            clean_answer("\"C:\\My Apps\\godot.exe\"\n"),
            "C:\\My Apps\\godot.exe"
        );
        assert_eq!(clean_answer("'/opt/godot'"), "/opt/godot");
        assert_eq!(clean_answer("\"unbalanced"), "\"unbalanced");
    }

    #[test]
    fn app_bundles_resolve_to_their_binary() {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("Godot.app");
        let binary = app.join("Contents").join("MacOS").join("Godot");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, "").unwrap();
        assert_eq!(executable_in(app), binary);
        assert_eq!(executable_in(temp.path().to_owned()), temp.path());
    }

    #[test]
    fn empty_answer_or_end_of_input_gives_up() {
        for input in ["", "\n", "  \n"] {
            let mut output = Vec::new();
            let err = ask_for_godot(&mut input.as_bytes(), &mut output, None).unwrap_err();
            assert!(err.to_string().contains("no answer given"), "{err}");
        }
    }

    /// A stand-in for Godot that only answers `--version`.
    fn fake_godot(dir: &Path) -> PathBuf {
        if cfg!(windows) {
            let path = dir.join("godot.cmd");
            std::fs::write(&path, "@echo 4.7.2.stable.official.abc1234\r\n").unwrap();
            path
        } else {
            let path = dir.join("godot");
            std::fs::write(&path, "#!/bin/sh\necho 4.7.2.stable.official.abc1234\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            path
        }
    }

    #[test]
    fn saves_the_first_godot_that_answers() {
        let temp = tempfile::tempdir().unwrap();
        let godot = fake_godot(temp.path());
        let config = temp.path().join("config").join("config.toml");
        let input = format!(
            "{}\n\"{}\"\n",
            temp.path().join("nope").display(),
            godot.display()
        );
        let mut output = Vec::new();

        let found = ask_for_godot(&mut input.as_bytes(), &mut output, Some(&config)).unwrap();
        assert_eq!(found.path, godot);
        assert_eq!(found.version.templates_folder(), "4.7.2.stable");
        assert_eq!(UserConfig::load(&config).unwrap().godot, Some(godot));
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("is not a file."), "{output}");
        assert!(
            output.contains(&format!("Saved it to {}", config.display())),
            "{output}"
        );
    }

    #[test]
    fn itch_games_come_as_targets_or_addresses() {
        for answer in [
            "devprice/idle-factory",
            "https://devprice.itch.io/idle-factory",
            "devprice.itch.io/idle-factory/",
            "http://DevPrice.itch.io/idle-factory?secret=x",
        ] {
            let target = parse_itch(answer).unwrap_or_else(|e| panic!("{answer}: {e}"));
            assert_eq!(
                target.to_string().to_ascii_lowercase(),
                "devprice/idle-factory"
            );
        }
        for answer in [
            "devprice",
            "https://itch.io/idle-factory",
            "https://devprice.itch.io/",
        ] {
            assert!(parse_itch(answer).is_err(), "{answer}");
        }
    }

    #[test]
    fn asks_again_for_a_malformed_game() {
        let mut output = Vec::new();
        let input = "idle-factory\ndevprice/idle-factory\n";
        let target = ask_for_itch(&mut input.as_bytes(), &mut output).unwrap();
        assert_eq!(target.to_string(), "devprice/idle-factory");
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("expected `<user>/<game>`"), "{output}");
    }

    #[test]
    fn keeps_asking_after_a_wrong_path() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("nope");
        let input = format!("{0}\n{0}\n{0}\n", missing.display());
        let mut output = Vec::new();
        let err = ask_for_godot(&mut input.as_bytes(), &mut output, None).unwrap_err();
        let output = String::from_utf8(output).unwrap();
        assert_eq!(output.matches("Path to Godot: ").count(), 3, "{output}");
        assert_eq!(output.matches("is not a file.").count(), 2, "{output}");
        assert!(err.to_string().contains("is not a file"), "{err}");
    }
}
