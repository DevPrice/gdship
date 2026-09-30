use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Finds executable `name` in the `PATH` that `env` returns, trying `name.exe` on
/// Windows. Relative entries such as `.` are skipped, so a project can't supply its own
/// tools.
pub(crate) fn find_on_path(name: &str, env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let path = env("PATH")?;
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(&file))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_path_in_order() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let file = format!("tool{}", std::env::consts::EXE_SUFFIX);
        for dir in [first.path(), second.path()] {
            let path = dir.join(&file);
            std::fs::write(&path, "").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let path = std::env::join_paths([first.path(), second.path()]).unwrap();
        let env = move |key: &str| (key == "PATH").then(|| path.clone());
        assert_eq!(find_on_path("tool", &env), Some(first.path().join(&file)));
        assert_eq!(find_on_path("other", &env), None);
        assert_eq!(find_on_path("tool", &|_| None), None);
    }
}
