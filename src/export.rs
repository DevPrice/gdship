use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};

use crate::config::Channel;
use crate::godot::Godot;
use crate::godot_project::{Preset, Target};
use crate::process::{ToolCommand, describe_exit, tail};
use crate::project::Project;
use crate::report::Reporter;

/// Lines of a failed export's log shown in the error.
const LOG_TAIL_LINES: usize = 40;

/// Filesystems such as FAT store modification times in 2-second steps, so a file written
/// just after the export started can look older than the start.
const MTIME_SLACK: Duration = Duration::from_secs(2);

/// One channel's export, planned but not yet run.
#[derive(Debug, Clone)]
pub(crate) struct Export {
    pub(crate) channel: Channel,
    pub(crate) preset: Preset,
    /// `.gdship/build/<channel>/`, wiped before the export.
    pub(crate) build_dir: PathBuf,
    pub(crate) output: PathBuf,
    pub(crate) log: PathBuf,
    pub(crate) command: ToolCommand,
}

/// Every command an export run needs, so `--dry-run` can print them.
#[derive(Debug, Clone)]
pub(crate) struct ExportPlan {
    state_dir: PathBuf,
    import: ToolCommand,
    import_log: PathBuf,
    pub(crate) exports: Vec<Export>,
}

impl ExportPlan {
    pub(crate) fn new(
        project: &Project,
        project_name: Option<&str>,
        godot: &Godot,
        targets: &[Target],
    ) -> Self {
        let root = project.root();
        let state_dir = project.state_dir();
        let logs = state_dir.join("logs");
        let base = || {
            ToolCommand::new(&godot.path)
                .arg("--headless")
                .arg("--path")
                .arg(root)
        };
        let exports = targets
            .iter()
            .map(|target| {
                let build_dir = state_dir.join("build").join(target.channel.as_str());
                let output = build_dir.join(output_file_name(&target.preset, project_name));
                Export {
                    channel: target.channel.clone(),
                    preset: target.preset.clone(),
                    command: base()
                        .arg("--export-release")
                        .arg(&target.preset.name)
                        .arg(&output),
                    log: logs.join(format!("{}.log", target.channel)),
                    build_dir,
                    output,
                }
            })
            .collect();
        Self {
            import: base().arg("--import"),
            import_log: logs.join("import.log"),
            state_dir,
            exports,
        }
    }

    /// The import and every export, in the order [`Self::run`] runs them.
    pub(crate) fn commands(&self) -> impl Iterator<Item = &ToolCommand> {
        std::iter::once(&self.import).chain(self.exports.iter().map(|e| &e.command))
    }

    /// Imports once, then exports and verifies each channel in order, stopping at the
    /// first failure.
    pub(crate) fn run(&self, verbose: bool, reporter: Reporter) -> Result<()> {
        self.prepare_state_dir()?;
        reporter.action("Importing", "project");
        let status = self.import.run_logged(&self.import_log, verbose)?;
        if !status.success() {
            return Err(log_failure(
                &self.import_log,
                format!("Godot's import failed with {}", describe_exit(status)),
            ));
        }
        for export in &self.exports {
            reporter.action(
                "Exporting",
                format!("{} (preset \"{}\")", export.channel, export.preset.name),
            );
            export.run(verbose)?;
        }
        Ok(())
    }

    /// Creates `.gdship/` with a `.gdignore`, so Godot doesn't import the builds.
    fn prepare_state_dir(&self) -> Result<()> {
        let logs = self.import_log.parent().expect("logs have a parent");
        std::fs::create_dir_all(logs)
            .with_context(|| format!("cannot create {}", logs.display()))?;
        let gdignore = self.state_dir.join(".gdignore");
        if !gdignore.exists() {
            std::fs::write(&gdignore, "")
                .with_context(|| format!("cannot create {}", gdignore.display()))?;
        }
        Ok(())
    }
}

impl Export {
    fn run(&self, verbose: bool) -> Result<()> {
        match std::fs::remove_dir_all(&self.build_dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("cannot remove {}", self.build_dir.display()));
            }
        }
        std::fs::create_dir_all(&self.build_dir)
            .with_context(|| format!("cannot create {}", self.build_dir.display()))?;
        let started = SystemTime::now();
        let status = self.command.run_logged(&self.log, verbose)?;
        let problem = if status.success() {
            self.verify(started)
        } else {
            Some(format!("Godot failed with {}", describe_exit(status)))
        };
        match problem {
            Some(problem) => Err(log_failure(
                &self.log,
                format!("export of channel `{}` failed: {problem}", self.channel),
            )),
            None => Ok(()),
        }
    }

    /// Godot has been known to exit 0 after a failed export, so the output is checked too.
    fn verify(&self, started: SystemTime) -> Option<String> {
        let output = self.output.display();
        let metadata = match std::fs::metadata(&self.output) {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => return Some(format!("Godot exited 0 but did not write {output}")),
        };
        if metadata.len() == 0 {
            return Some(format!("{output} is empty"));
        }
        if let Ok(modified) = metadata.modified()
            && modified + MTIME_SLACK < started
        {
            return Some(format!("{output} is older than the export"));
        }
        if self.preset.is_web() {
            for extension in ["wasm", "pck"] {
                if !has_extension(&self.build_dir, extension) {
                    return Some(format!(
                        "the Web build has no .{extension} file in {}",
                        self.build_dir.display()
                    ));
                }
            }
        }
        None
    }
}

fn has_extension(dir: &Path, extension: &str) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            entry.path().extension().is_some_and(|e| e == extension)
                && entry.file_type().is_ok_and(|t| t.is_file())
        })
    })
}

/// An error carrying the tail of `log`, which is where Godot says what went wrong.
fn log_failure(log: &Path, problem: String) -> anyhow::Error {
    match tail(log, LOG_TAIL_LINES) {
        Ok(lines) if !lines.trim().is_empty() => anyhow::anyhow!(
            "{problem}\n--- last {LOG_TAIL_LINES} lines of {} ---\n{lines}\n--- end of log ---",
            log.display()
        ),
        _ => anyhow::anyhow!("{problem} (log: {})", log.display()),
    }
}

/// The file Godot writes: the name from the preset's `export_path`, except that Web
/// builds are always `index.html` because itch serves the one at the build root.
pub(crate) fn output_file_name(preset: &Preset, project_name: Option<&str>) -> String {
    if preset.is_web() {
        return "index.html".to_owned();
    }
    let from_path = preset
        .export_path
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty() && *name != "." && *name != "..");
    if let Some(name) = from_path {
        return name.to_owned();
    }
    let extension = match preset.platform.as_str() {
        "Windows Desktop" => ".exe",
        "Linux" => ".x86_64",
        "macOS" => ".zip",
        "Android" => ".apk",
        "iOS" => ".ipa",
        _ => "",
    };
    format!("{}{extension}", slugify(project_name.unwrap_or("game")))
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "game".to_owned()
    } else {
        slug.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(platform: &str, export_path: &str) -> Preset {
        Preset {
            name: "p".into(),
            platform: platform.into(),
            export_path: export_path.into(),
            custom_release_template: None,
        }
    }

    #[test]
    fn output_names_come_from_the_export_path() {
        assert_eq!(
            output_file_name(&preset("Windows Desktop", "exports/windows/idle.exe"), None),
            "idle.exe"
        );
        assert_eq!(
            output_file_name(&preset("Linux", "C:\\builds\\idle.x86_64"), None),
            "idle.x86_64"
        );
        assert_eq!(
            output_file_name(&preset("Web", "exports/web/game.html"), None),
            "index.html"
        );
    }

    #[test]
    fn empty_export_paths_use_the_slugified_project_name() {
        let name = Some("Idle Factory: Deluxe!");
        assert_eq!(
            output_file_name(&preset("Windows Desktop", ""), name),
            "idle-factory-deluxe.exe"
        );
        assert_eq!(
            output_file_name(&preset("Linux", ""), name),
            "idle-factory-deluxe.x86_64"
        );
        assert_eq!(
            output_file_name(&preset("macOS", ""), name),
            "idle-factory-deluxe.zip"
        );
        assert_eq!(
            output_file_name(&preset("Android", "exports/"), None),
            "game.apk"
        );
        assert_eq!(
            output_file_name(&preset("Linux", ""), Some("日本")),
            "game.x86_64"
        );
    }
}
