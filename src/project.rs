use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::{PROJECT_CONFIG_FILE, ProjectConfig};

pub(crate) const GODOT_PROJECT_FILE: &str = "project.godot";

/// A Godot project root: the directory holding `project.godot`.
#[derive(Debug, Clone)]
pub(crate) struct Project {
    root: PathBuf,
}

impl Project {
    /// Finds the nearest ancestor of `start` (inclusive) containing `project.godot`.
    pub(crate) fn discover(start: &Path) -> Result<Self> {
        for dir in start.ancestors() {
            if dir.join(GODOT_PROJECT_FILE).is_file() {
                return Ok(Self {
                    root: dir.to_owned(),
                });
            }
        }
        bail!(
            "no {GODOT_PROJECT_FILE} found in {} or any parent directory",
            start.display()
        )
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn config_path(&self) -> PathBuf {
        self.root.join(PROJECT_CONFIG_FILE)
    }

    /// Reads `gdship.toml`, or returns `None` if the project has none.
    pub(crate) fn config(&self) -> Result<Option<ProjectConfig>> {
        let path = self.config_path();
        if path.is_file() {
            ProjectConfig::load(&path).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Like [`Self::config`], but a missing file is an error.
    pub(crate) fn require_config(&self) -> Result<ProjectConfig> {
        match self.config()? {
            Some(config) => Ok(config),
            None => bail!(
                "no {PROJECT_CONFIG_FILE} next to {}; create one with at least \
                 `itch = \"<user>/<game>\"`",
                self.root.join(GODOT_PROJECT_FILE).display()
            ),
        }
    }

    /// gdship's working directory. Its contents are gdship's to delete.
    pub(crate) fn state_dir(&self) -> PathBuf {
        self.root.join(".gdship")
    }

    /// Adds `/.gdship/` to the project's `.gitignore` unless a line there already
    /// ignores it. Returns whether the file changed; a project without one is left alone.
    pub(crate) fn ignore_state_dir(&self) -> Result<bool> {
        let path = self.root.join(".gitignore");
        let mut text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
        };
        let ignored = text.lines().any(|line| {
            matches!(
                line.trim(),
                ".gdship" | ".gdship/" | "/.gdship" | "/.gdship/"
            )
        });
        if ignored {
            return Ok(false);
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("/.gdship/\n");
        std::fs::write(&path, text).with_context(|| format!("cannot write {}", path.display()))?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_nearest_project() {
        let temp = tempfile::tempdir().unwrap();
        let outer = temp.path();
        let game = outer.join("games").join("mygame");
        let deep = game.join("scenes").join("level1");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(outer.join(GODOT_PROJECT_FILE), "").unwrap();
        std::fs::write(game.join(GODOT_PROJECT_FILE), "").unwrap();

        assert_eq!(Project::discover(&deep).unwrap().root(), game);
        assert_eq!(
            Project::discover(&outer.join("games")).unwrap().root(),
            outer
        );
    }

    #[test]
    fn ignores_the_state_dir_once() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(GODOT_PROJECT_FILE), "").unwrap();
        let project = Project::discover(temp.path()).unwrap();
        let gitignore = temp.path().join(".gitignore");
        assert!(!project.ignore_state_dir().unwrap());
        assert!(!gitignore.exists(), "a missing .gitignore is not created");

        std::fs::write(&gitignore, ".godot/").unwrap();
        assert!(project.ignore_state_dir().unwrap());
        assert_eq!(
            std::fs::read_to_string(&gitignore).unwrap(),
            ".godot/\n/.gdship/\n"
        );
        assert!(!project.ignore_state_dir().unwrap());

        for existing in [".gdship", "  .gdship/  ", "/.gdship"] {
            std::fs::write(&gitignore, format!("# build\n{existing}\n")).unwrap();
            assert!(!project.ignore_state_dir().unwrap(), "{existing}");
        }
        std::fs::write(&gitignore, "").unwrap();
        assert!(project.ignore_state_dir().unwrap());
        assert_eq!(std::fs::read_to_string(&gitignore).unwrap(), "/.gdship/\n");
    }

    #[test]
    fn missing_project_and_config_are_errors() {
        let temp = tempfile::tempdir().unwrap();
        let err = Project::discover(temp.path()).unwrap_err().to_string();
        assert!(err.contains("no project.godot found"), "{err}");

        std::fs::write(temp.path().join(GODOT_PROJECT_FILE), "").unwrap();
        let project = Project::discover(temp.path()).unwrap();
        assert!(project.config().unwrap().is_none());
        let err = project.require_config().unwrap_err().to_string();
        assert!(err.contains("no gdship.toml"), "{err}");

        std::fs::write(project.config_path(), "itch = \"a/b\"").unwrap();
        assert!(project.require_config().is_ok());
    }
}
