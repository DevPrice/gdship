use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use zip::ZipArchive;

use crate::digest::{Sha256, copy_hashed};

/// A file written by [`extract`], relative to the destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtractedFile {
    /// `/`-separated path relative to the destination.
    pub(crate) path: String,
    pub(crate) sha256: Sha256,
}

/// Caps on what one archive may expand to, so a zip bomb fails instead of filling the
/// disk. Sizes are counted as bytes are written, not taken from the zip's headers.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) max_entries: usize,
    pub(crate) max_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            max_bytes: 2 << 30,
        }
    }
}

/// Extracts every entry of the zip at `path` into `dest`, which must not exist yet,
/// keeping Unix mode bits.
pub(crate) fn extract(path: &Path, dest: &Path) -> Result<Vec<ExtractedFile>> {
    extract_with_limits(path, dest, Limits::default())
}

pub(crate) fn extract_with_limits(
    path: &Path,
    dest: &Path,
    limits: Limits,
) -> Result<Vec<ExtractedFile>> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut zip = ZipArchive::new(file)
        .with_context(|| format!("{} is not a valid zip archive", path.display()))?;
    if zip.len() > limits.max_entries {
        bail!(
            "{} has {} entries, more than the limit of {}",
            path.display(),
            zip.len(),
            limits.max_entries
        );
    }
    std::fs::create_dir(dest).with_context(|| format!("cannot create {}", dest.display()))?;
    let mut extracted = Vec::new();
    let mut remaining = limits.max_bytes;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .with_context(|| format!("cannot read entry {index} of {}", path.display()))?;
        let segments = entry_segments(entry.name())
            .and_then(|segments| check_portable(&segments).map(|()| segments))
            .with_context(|| format!("unsafe entry in {}", path.display()))?;
        let out: PathBuf = segments.iter().fold(dest.to_owned(), |p, s| p.join(s));
        let relative = segments.join("/");
        if entry.is_dir() {
            std::fs::create_dir_all(&out)
                .with_context(|| format!("cannot create {}", out.display()))?;
            continue;
        }
        if entry.is_symlink() {
            bail!(
                "{} contains a symbolic link ({relative}), which gdship does not extract",
                path.display()
            );
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let mut file =
            File::create_new(&out).with_context(|| format!("cannot create {}", out.display()))?;
        let (size, sha256) = copy_hashed(&mut (&mut entry).take(remaining + 1), &mut file)
            .with_context(|| format!("cannot extract {relative} from {}", path.display()))?;
        if size > remaining {
            bail!(
                "{} expands to more than the {} MiB limit",
                path.display(),
                limits.max_bytes >> 20
            );
        }
        remaining -= size;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode & 0o777))
                .with_context(|| format!("cannot set permissions on {}", out.display()))?;
        }
        extracted.push(ExtractedFile {
            path: relative,
            sha256,
        });
    }
    Ok(extracted)
}

/// Splits a zip entry name into path segments, rejecting anything that could escape the
/// destination. Control characters are refused too, since names end up in log output.
fn entry_segments(name: &str) -> Result<Vec<String>> {
    if name.chars().any(char::is_control) {
        bail!("{name:?} contains control characters");
    }
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains(':') {
        bail!("`{name}` is an absolute path");
    }
    let mut segments = Vec::new();
    for segment in normalized.split('/') {
        match segment {
            "" | "." => {}
            ".." => bail!("`{name}` points outside the archive"),
            _ => segments.push(segment.to_owned()),
        }
    }
    if segments.is_empty() {
        bail!("`{name}` is an empty path");
    }
    Ok(segments)
}

/// Refuses names Windows would silently change or treat as devices: trailing dots and
/// spaces are stripped (so `a.` and `a` collide), and `CON`, `NUL`, `COM1`... open devices
/// on Windows 10.
fn check_portable(segments: &[String]) -> Result<()> {
    for segment in segments {
        if segment.ends_with('.') || segment.ends_with(' ') {
            bail!("`{segment}` ends with a dot or space, which Windows strips");
        }
        if segment.contains(['<', '>', '"', '|', '?', '*']) {
            bail!("`{segment}` contains a character Windows does not allow in file names");
        }
        if is_reserved_on_windows(segment) {
            bail!("`{segment}` is a reserved device name on Windows");
        }
    }
    Ok(())
}

fn is_reserved_on_windows(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ((stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && stem.as_bytes()[3].is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use zip::write::SimpleFileOptions;

    use super::*;

    enum Item<'a> {
        File(&'a str, &'a [u8]),
        Dir(&'a str),
        Symlink(&'a str, &'a str),
    }

    fn write_zip(dir: &Path, items: &[Item]) -> PathBuf {
        let path = dir.join("fixture.zip");
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        let options = SimpleFileOptions::default().unix_permissions(0o755);
        for item in items {
            match item {
                Item::File(name, data) => {
                    zip.start_file(*name, options).unwrap();
                    zip.write_all(data).unwrap();
                }
                Item::Dir(name) => zip.add_directory(*name, options).unwrap(),
                Item::Symlink(name, target) => zip.add_symlink(*name, *target, options).unwrap(),
            }
        }
        zip.finish().unwrap();
        path
    }

    #[test]
    fn extracts_everything_and_hashes_files() {
        let temp = tempfile::tempdir().unwrap();
        let zip = write_zip(
            temp.path(),
            &[
                Item::File("butler", b"\x7fELF"),
                Item::File("7z.so", b"lib"),
                Item::Dir("empty/"),
            ],
        );
        let dest = temp.path().join("out");
        let files = extract(&zip, &dest).unwrap();
        assert_eq!(
            files,
            [
                ExtractedFile {
                    path: "butler".into(),
                    sha256: Sha256::of_bytes(b"\x7fELF"),
                },
                ExtractedFile {
                    path: "7z.so".into(),
                    sha256: Sha256::of_bytes(b"lib"),
                },
            ]
        );
        assert_eq!(std::fs::read(dest.join("7z.so")).unwrap(), b"lib");
        assert!(dest.join("empty").is_dir());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dest.join("butler"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }

    #[test]
    fn rejects_entries_that_escape_or_windows_would_alter() {
        for name in [
            "../evil",
            "a/../../evil",
            "/abs",
            "C:/win",
            "a\\..\\..\\b",
            "line\nbreak",
            "trail.",
            "aux.dll",
            "q?.txt",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let zip = write_zip(temp.path(), &[Item::File(name, b"x")]);
            let err = extract(&zip, &temp.path().join("out")).unwrap_err();
            assert!(
                format!("{err:#}").contains("unsafe entry"),
                "{name}: {err:#}"
            );
        }
    }

    #[test]
    fn enforces_entry_and_size_limits() {
        let temp = tempfile::tempdir().unwrap();
        let zip = write_zip(
            temp.path(),
            &[
                Item::File("one.bin", &[0; 600]),
                Item::File("two.bin", &[0; 600]),
            ],
        );
        let few_entries = Limits {
            max_entries: 1,
            ..Limits::default()
        };
        let err = extract_with_limits(&zip, &temp.path().join("a"), few_entries).unwrap_err();
        assert!(err.to_string().contains("more than the limit"), "{err}");

        let small = Limits {
            max_bytes: 1000,
            ..Limits::default()
        };
        let err = extract_with_limits(&zip, &temp.path().join("b"), small).unwrap_err();
        assert!(err.to_string().contains("MiB limit"), "{err}");
    }

    #[test]
    fn rejects_symlinks_and_non_zips() {
        let temp = tempfile::tempdir().unwrap();
        let zip = write_zip(temp.path(), &[Item::Symlink("link", "../../outside")]);
        let err = extract(&zip, &temp.path().join("out")).unwrap_err();
        assert!(err.to_string().contains("symbolic link (link)"), "{err}");

        let page = temp.path().join("page.zip");
        std::fs::write(&page, "<html>not found</html>").unwrap();
        let err = extract(&page, &temp.path().join("out2")).unwrap_err();
        assert!(
            err.to_string().contains("is not a valid zip archive"),
            "{err}"
        );
    }
}
