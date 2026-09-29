use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

/// A command gdship runs, kept as data so `--dry-run` can print it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolCommand {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<OsString>,
}

impl ToolCommand {
    pub(crate) fn new(program: &Path) -> Self {
        Self {
            program: program.to_owned(),
            args: Vec::new(),
        }
    }

    pub(crate) fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        command
    }

    /// Runs with the terminal attached, for tools that talk to the user.
    pub(crate) fn run_attached(&self) -> Result<ExitStatus> {
        self.command()
            .status()
            .with_context(|| format!("cannot run {}", self.program.display()))
    }

    /// Runs with stdout and stderr written to `log`, and also echoed to gdship's own
    /// streams when `stream` is set.
    pub(crate) fn run_logged(&self, log: &Path, stream: bool) -> Result<ExitStatus> {
        let file = File::create(log).with_context(|| format!("cannot create {}", log.display()))?;
        let file = Arc::new(Mutex::new(file));
        let mut child = self
            .command()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("cannot run {}", self.program.display()))?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let copiers = [
            copy_to_log(
                stdout,
                Arc::clone(&file),
                stream.then(|| Box::new(io::stdout()) as _),
            ),
            copy_to_log(
                stderr,
                Arc::clone(&file),
                stream.then(|| Box::new(io::stderr()) as _),
            ),
        ];
        let status = child
            .wait()
            .with_context(|| format!("cannot wait for {}", self.program.display()))?;
        for copier in copiers {
            copier
                .join()
                .expect("log copier panicked")
                .with_context(|| format!("cannot write {}", log.display()))?;
        }
        Ok(status)
    }
}

fn copy_to_log(
    mut from: impl Read + Send + 'static,
    log: Arc<Mutex<File>>,
    mut echo: Option<Box<dyn Write + Send>>,
) -> std::thread::JoinHandle<io::Result<()>> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            let n = match from.read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            log.lock()
                .expect("log lock poisoned")
                .write_all(&buf[..n])?;
            if let Some(echo) = &mut echo {
                // A closed terminal must not fail the export; the log still has it all.
                let _ = echo.write_all(&buf[..n]).and_then(|()| echo.flush());
            }
        }
    })
}

impl fmt::Display for ToolCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", quote(self.program.as_os_str()))?;
        for arg in &self.args {
            write!(f, " {}", quote(arg))?;
        }
        Ok(())
    }
}

/// Quotes an argument for display when it has characters a shell would split on.
fn quote(arg: &OsStr) -> String {
    let text = arg.to_string_lossy();
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:\\=@+,".contains(c));
    if plain {
        text.into_owned()
    } else {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// "exit code 1", or how the process was killed, the same on every platform.
pub(crate) fn describe_exit(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit code {code}"),
        None => status.to_string(),
    }
}

/// The last `count` lines of the file at `path`, for showing a failure's cause.
pub(crate) fn tail(path: &Path, count: usize) -> io::Result<String> {
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    Ok(lines[lines.len().saturating_sub(count)..].join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_quotes_only_when_needed() {
        let command = ToolCommand::new(Path::new("/bin/godot"))
            .arg("--export-release")
            .arg("Windows Desktop")
            .arg("C:\\out\\game.exe")
            .arg("say \"hi\"");
        assert_eq!(
            command.to_string(),
            "/bin/godot --export-release \"Windows Desktop\" C:\\out\\game.exe \"say \\\"hi\\\"\""
        );
    }

    #[test]
    fn tail_keeps_the_last_lines() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("log");
        let text: String = (1..=50).map(|n| format!("line {n}\n")).collect();
        std::fs::write(&path, text).unwrap();
        let tail = tail(&path, 40).unwrap();
        assert!(tail.starts_with("line 11\n"), "{tail}");
        assert!(tail.ends_with("line 50"), "{tail}");
        std::fs::write(&path, "one\ntwo").unwrap();
        assert_eq!(super::tail(&path, 40).unwrap(), "one\ntwo");
    }
}
