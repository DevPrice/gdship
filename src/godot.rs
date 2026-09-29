use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};

use crate::config::{UserConfig, home_dir};
use crate::exe::find_on_path;
use crate::godot_project::{ProjectInfo, Target};

/// A Godot version as `godot --version` prints it, e.g. `4.7.2.stable.mono.official.abc1234`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GodotVersion {
    pub(crate) major: u32,
    pub(crate) minor: u32,
    /// 0 when the version string leaves it out, as Godot does for `x.y.0`.
    pub(crate) patch: u32,
    /// `stable`, `rc2`, `beta1`, `dev`...
    pub(crate) status: String,
    /// A .NET build, whose templates are separate.
    pub(crate) mono: bool,
    full: String,
}

impl GodotVersion {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let full = text.trim();
        let mut parts = full.split('.');
        let number = |part: Option<&str>| -> Option<u32> {
            let part = part?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        };
        let major = number(parts.next())?;
        let minor = number(parts.next())?;
        let mut next = parts.next()?;
        let patch = match number(Some(next)) {
            Some(patch) => {
                next = parts.next()?;
                patch
            }
            None => 0,
        };
        if next.is_empty() || !next.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        let status = next.to_owned();
        let mono = parts.next() == Some("mono");
        Some(Self {
            major,
            minor,
            patch,
            status,
            mono,
            full: full.to_owned(),
        })
    }

    /// The export templates folder name, e.g. `4.7.2.stable`, `4.7.stable` for 4.7.0, or
    /// `4.7.2.stable.mono`.
    pub(crate) fn templates_folder(&self) -> String {
        let mut name = format!("{}.{}", self.major, self.minor);
        if self.patch != 0 {
            name.push_str(&format!(".{}", self.patch));
        }
        name.push('.');
        name.push_str(&self.status);
        if self.mono {
            name.push_str(".mono");
        }
        name
    }
}

impl fmt::Display for GodotVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.full)
    }
}

/// A Godot binary and its version.
#[derive(Debug, Clone)]
pub(crate) struct Godot {
    pub(crate) path: PathBuf,
    pub(crate) version: GodotVersion,
}

impl Godot {
    /// Finds the configured Godot binary; `None` when nothing names one.
    pub(crate) fn find(
        flag: Option<&Path>,
        user_config: &UserConfig,
        env: &dyn Fn(&str) -> Option<OsString>,
    ) -> Result<Option<PathBuf>> {
        find_godot(flag, user_config, env)
    }

    /// Asks the binary at `path` for its version.
    pub(crate) fn at(path: &Path) -> Result<Self> {
        let version = query_version(path)?;
        Ok(Self {
            path: path.to_owned(),
            version,
        })
    }

    /// Checks that this Godot is the version the project was made with.
    pub(crate) fn check_project(&self, project: &ProjectInfo) -> Result<()> {
        let (major, minor) = project.godot_version;
        if (self.version.major, self.version.minor) != (major, minor) {
            bail!(
                "{} is Godot {}, but the project is for Godot {major}.{minor} (config/features \
                 in project.godot); use a {major}.{minor} build with --godot or GDSHIP_GODOT",
                self.path.display(),
                self.version
            );
        }
        Ok(())
    }

    /// Checks that export templates are installed for every target that needs them.
    pub(crate) fn check_templates(
        &self,
        targets: &[Target],
        env: &dyn Fn(&str) -> Option<OsString>,
    ) -> Result<()> {
        if targets
            .iter()
            .all(|t| t.preset.custom_release_template.is_some())
        {
            return Ok(());
        }
        let dir = templates_root(&self.path, env)?.join(self.version.templates_folder());
        if !dir.is_dir() {
            bail!(
                "export templates for Godot {} are not installed: {} does not exist. Install \
                 them from the editor's Editor > Manage Export Templates, or set \
                 custom_template/release in each preset",
                self.version,
                dir.display()
            );
        }
        Ok(())
    }
}

pub(crate) const NOT_FOUND: &str = "cannot find Godot: pass --godot, set GDSHIP_GODOT, set \
     `godot` in the user config, or put godot on PATH";

fn find_godot(
    flag: Option<&Path>,
    user_config: &UserConfig,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<Option<PathBuf>> {
    let explicit = |path: &Path, source: &str| -> Result<PathBuf> {
        if path.is_file() {
            Ok(path.to_owned())
        } else {
            bail!(
                "Godot binary {} from {source} does not exist",
                path.display()
            )
        }
    };
    if let Some(path) = flag {
        return explicit(path, "--godot").map(Some);
    }
    if let Some(path) = env("GDSHIP_GODOT").filter(|v| !v.is_empty()) {
        return explicit(Path::new(&path), "GDSHIP_GODOT").map(Some);
    }
    if let Some(path) = &user_config.godot {
        return explicit(path, "the user config").map(Some);
    }
    Ok(find_on_path("godot", env).or_else(|| find_on_path("godot4", env)))
}

fn query_version(path: &Path) -> Result<GodotVersion> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .with_context(|| format!("cannot run {}", path.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        bail!(
            "`{} --version` failed ({}): {}{}",
            path.display(),
            output.status,
            stdout.trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    stdout
        .lines()
        .rev()
        .find_map(GodotVersion::parse)
        .ok_or_else(|| {
            anyhow!(
                "cannot read the version printed by `{} --version`: {}",
                path.display(),
                stdout.trim()
            )
        })
}

/// The folder holding one subfolder of export templates per Godot version.
pub(crate) fn templates_root(
    godot: &Path,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<PathBuf> {
    if let Some(dir) = self_contained_dir(godot, cfg!(target_os = "macos")) {
        return Ok(dir.join("editor_data").join("export_templates"));
    }
    let var = |key: &str| env(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    let data_dir = if cfg!(windows) {
        var("APPDATA")
            .map(|dir| dir.join("Godot"))
            .ok_or_else(|| anyhow!("cannot find the export templates: APPDATA is not set"))?
    } else if cfg!(target_os = "macos") {
        home_dir(env)
            .map(|home| {
                home.join("Library")
                    .join("Application Support")
                    .join("Godot")
            })
            .ok_or_else(|| anyhow!("cannot find the export templates: HOME is not set"))?
    } else {
        var("XDG_DATA_HOME")
            .or_else(|| home_dir(env).map(|home| home.join(".local").join("share")))
            .map(|dir| dir.join("godot"))
            .ok_or_else(|| anyhow!("cannot find the export templates: HOME is not set"))?
    };
    Ok(data_dir.join("export_templates"))
}

/// The folder holding `editor_data/` when Godot runs in self-contained mode, which a
/// `._sc_` or `_sc_` file next to the binary turns on. Inside a macOS app bundle, whose
/// contents are read-only, Godot also looks next to the `.app`, and keeps its data there
/// whichever marker it found. See `EditorPaths::EditorPaths` in Godot's
/// editor/file_system/editor_paths.cpp.
fn self_contained_dir(godot: &Path, macos: bool) -> Option<PathBuf> {
    let has_marker = |dir: &Path| dir.join("._sc_").is_file() || dir.join("_sc_").is_file();
    let exe_dir = godot.parent()?;
    let mut found = has_marker(exe_dir);
    let mut data_dir = exe_dir;
    if macos
        && exe_dir.ends_with("MacOS")
        && exe_dir.parent().is_some_and(|d| d.ends_with("Contents"))
        && let Some(outside) = exe_dir.ancestors().nth(3)
    {
        found |= has_marker(outside);
        data_dir = outside;
    }
    found.then(|| data_dir.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::godot_project::Preset;

    fn version(text: &str) -> GodotVersion {
        GodotVersion::parse(text).unwrap_or_else(|| panic!("{text}"))
    }

    #[test]
    fn parses_official_custom_and_mono_versions() {
        let v = version("4.7.2.stable.official.abc1234");
        assert_eq!((v.major, v.minor, v.patch), (4, 7, 2));
        assert_eq!(v.status, "stable");
        assert!(!v.mono);
        assert_eq!(v.to_string(), "4.7.2.stable.official.abc1234");

        let v = version("4.7.2.stable.custom_build.abc1234\n");
        assert_eq!((v.patch, v.mono), (2, false));

        let v = version("4.7.2.stable.mono.official.abc1234");
        assert!(v.mono);

        let v = version("4.7.stable.official.abc1234");
        assert_eq!((v.major, v.minor, v.patch), (4, 7, 0));

        let v = version("4.8.dev.custom_build");
        assert_eq!(v.status, "dev");
    }

    #[test]
    fn rejects_non_versions() {
        for text in [
            "",
            "Godot Engine v4.7",
            "4",
            "4.7",
            "4.x.stable",
            "4.7.2",
            "WARNING: x",
        ] {
            assert_eq!(GodotVersion::parse(text), None, "{text}");
        }
    }

    #[test]
    fn templates_folder_drops_a_zero_patch_and_marks_mono() {
        assert_eq!(
            version("4.7.2.stable.official.a").templates_folder(),
            "4.7.2.stable"
        );
        assert_eq!(
            version("4.7.stable.official.a").templates_folder(),
            "4.7.stable"
        );
        assert_eq!(version("4.7.rc2.official.a").templates_folder(), "4.7.rc2");
        assert_eq!(
            version("4.7.2.stable.mono.official.a").templates_folder(),
            "4.7.2.stable.mono"
        );
        assert_eq!(
            version("4.7.stable.mono.custom_build.a").templates_folder(),
            "4.7.stable.mono"
        );
    }

    fn env(vars: Vec<(&'static str, OsString)>) -> impl Fn(&str) -> Option<OsString> {
        move |key| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())
    }

    fn godot(dir: &Path, version: &str) -> Godot {
        Godot {
            path: dir.join("godot"),
            version: GodotVersion::parse(version).unwrap(),
        }
    }

    fn target(custom: Option<&str>) -> Target {
        Target {
            channel: "html5".parse().unwrap(),
            preset: Preset {
                name: "Web".into(),
                platform: "Web".into(),
                export_path: String::new(),
                custom_release_template: custom.map(str::to_owned),
            },
        }
    }

    #[test]
    fn platform_templates_root() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let home = temp.path().join("home");
        let env = env(vec![
            ("APPDATA", data.clone().into()),
            ("XDG_DATA_HOME", data.clone().into()),
            ("HOME", home.clone().into()),
        ]);
        let root = templates_root(&temp.path().join("bin").join("godot"), &env).unwrap();
        let expected = if cfg!(windows) {
            data.join("Godot")
        } else if cfg!(target_os = "macos") {
            home.join("Library")
                .join("Application Support")
                .join("Godot")
        } else {
            data.join("godot")
        };
        assert_eq!(root, expected.join("export_templates"));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_falls_back_to_local_share() {
        let env = env(vec![("HOME", "/home/me".into())]);
        assert_eq!(
            templates_root(Path::new("/usr/bin/godot"), &env).unwrap(),
            Path::new("/home/me/.local/share/godot/export_templates")
        );
    }

    #[test]
    fn self_contained_mode_uses_editor_data() {
        for marker in ["._sc_", "_sc_"] {
            let temp = tempfile::tempdir().unwrap();
            std::fs::write(temp.path().join(marker), "").unwrap();
            let root = templates_root(&temp.path().join("godot"), &|_| None).unwrap();
            assert_eq!(
                root,
                temp.path().join("editor_data").join("export_templates")
            );
        }
    }

    #[test]
    fn macos_bundles_keep_self_contained_data_beside_the_app() {
        let temp = tempfile::tempdir().unwrap();
        let exe_dir = temp.path().join("Godot.app").join("Contents").join("MacOS");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let binary = exe_dir.join("Godot");
        assert_eq!(self_contained_dir(&binary, true), None);

        std::fs::write(temp.path().join("_sc_"), "").unwrap();
        assert_eq!(
            self_contained_dir(&binary, true),
            Some(temp.path().to_owned())
        );
        assert_eq!(self_contained_dir(&binary, false), None);

        std::fs::remove_file(temp.path().join("_sc_")).unwrap();
        std::fs::write(exe_dir.join("._sc_"), "").unwrap();
        assert_eq!(
            self_contained_dir(&binary, true),
            Some(temp.path().to_owned())
        );
        assert_eq!(self_contained_dir(&binary, false), Some(exe_dir));
    }

    #[test]
    fn missing_templates_name_the_folder_and_version() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("_sc_"), "").unwrap();
        let godot = godot(temp.path(), "4.7.stable.official.abc");
        let err = godot
            .check_templates(&[target(None)], &|_| None)
            .unwrap_err()
            .to_string();
        let folder = temp
            .path()
            .join("editor_data")
            .join("export_templates")
            .join("4.7.stable");
        assert!(err.contains(&folder.display().to_string()), "{err}");
        assert!(err.contains("Godot 4.7.stable.official.abc"), "{err}");

        godot
            .check_templates(&[target(Some("/t/web.zip"))], &|_| None)
            .unwrap();
        std::fs::create_dir_all(&folder).unwrap();
        godot.check_templates(&[target(None)], &|_| None).unwrap();
    }

    #[test]
    fn project_version_must_match() {
        let godot = godot(Path::new("/g"), "4.6.3.stable.official.a");
        let project = |godot_version| ProjectInfo {
            name: None,
            godot_version,
        };
        let err = godot
            .check_project(&project((4, 7)))
            .unwrap_err()
            .to_string();
        assert!(err.contains("is Godot 4.6.3.stable.official.a"), "{err}");
        assert!(err.contains("project is for Godot 4.7"), "{err}");
        godot.check_project(&project((4, 6))).unwrap();
    }

    #[test]
    fn lookup_order() {
        let temp = tempfile::tempdir().unwrap();
        let file = |name: &str| {
            let path = temp.path().join(name);
            std::fs::write(&path, "").unwrap();
            path
        };
        let (flag, from_env, from_config) = (file("flag"), file("env"), file("config"));
        let config = UserConfig {
            godot: Some(from_config.clone()),
        };
        let with_env = env(vec![("GDSHIP_GODOT", from_env.clone().into())]);

        assert_eq!(
            find_godot(Some(&flag), &config, &with_env).unwrap(),
            Some(flag)
        );
        assert_eq!(
            find_godot(None, &config, &with_env).unwrap(),
            Some(from_env)
        );
        assert_eq!(
            find_godot(None, &config, &|_| None).unwrap(),
            Some(from_config)
        );
        assert_eq!(
            find_godot(None, &UserConfig::default(), &|_| None).unwrap(),
            None
        );

        let godot4 = file(&format!("godot4{}", std::env::consts::EXE_SUFFIX));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&godot4, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let on_path = env(vec![("PATH", temp.path().into())]);
        assert_eq!(
            find_godot(None, &UserConfig::default(), &on_path).unwrap(),
            Some(godot4)
        );

        let missing = UserConfig {
            godot: Some(temp.path().join("nope")),
        };
        let err = find_godot(None, &missing, &|_| None).unwrap_err();
        assert!(
            err.to_string()
                .contains("from the user config does not exist"),
            "{err}"
        );
    }
}
