use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::archive::extract;
use crate::digest::Sha256;
use crate::exe::find_on_path;
use crate::fetch::{Cache, Fetcher};
use crate::report::Reporter;

/// The butler release gdship downloads when none is installed. Updating it means
/// updating [`PINS`] and releasing gdship.
pub const BUTLER_VERSION: &str = "15.31.0";

/// itch's broth service, which hosts butler builds per channel and version.
const BROTH_URL: &str = "https://broth.itch.zone/butler";

/// The sha256 of each broth channel's archive for [`BUTLER_VERSION`].
const PINS: &[(&str, &str)] = &[
    (
        "windows-amd64",
        "92e42f011db049128583ac88258d3309d00c69018d2a48b378c7eb5709d9efde",
    ),
    (
        "linux-amd64",
        "4f2a3f22b12f870923504d4b6935535cad377b45859f5fe9419e3adc0611a48c",
    ),
    (
        "linux-arm64",
        "2ffd4071dfa715024eedd2b4ac7406c1c7802452214d89a48496adb9c171b737",
    ),
    (
        "darwin-amd64",
        "70a4b8543fddee7031052ea76f1846ba071913148e20cf7a2f18421702fc1929",
    ),
    (
        "darwin-arm64",
        "5a5fcd3dc83de480748223388d9b5d6eab8df786bedb5d6730ffe59df87b2c5b",
    ),
];

/// Lists each extracted file's sha256, so a damaged cache entry is noticed and replaced.
const CACHE_MANIFEST: &str = ".gdship-files";

/// A butler archive gdship trusts: where to get it and what it must hash to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub version: String,
    pub url: String,
    pub sha256: Sha256,
}

impl Pin {
    /// The pinned archive for the machine gdship runs on.
    pub fn for_host() -> Result<Self> {
        Self::for_platform(std::env::consts::OS, std::env::consts::ARCH)
    }

    fn for_platform(os: &str, arch: &str) -> Result<Self> {
        let channel = match (os, arch) {
            // broth has no Windows ARM build; the amd64 one runs under emulation.
            ("windows", "x86_64" | "aarch64") => "windows-amd64",
            ("linux", "x86_64") => "linux-amd64",
            ("linux", "aarch64") => "linux-arm64",
            ("macos", "x86_64") => "darwin-amd64",
            ("macos", "aarch64") => "darwin-arm64",
            _ => bail!(
                "itch publishes no butler build for {os}/{arch}; install butler and set \
                 GDSHIP_BUTLER to its path"
            ),
        };
        let (_, sha256) = PINS
            .iter()
            .find(|(c, _)| *c == channel)
            .expect("every mapped channel is pinned");
        Ok(Self {
            version: BUTLER_VERSION.to_owned(),
            url: format!("{BROTH_URL}/{channel}/{BUTLER_VERSION}/archive/default"),
            sha256: sha256.parse().expect("pins are valid hashes"),
        })
    }
}

fn exe_name() -> String {
    format!("butler{}", std::env::consts::EXE_SUFFIX)
}

/// Finds butler: `GDSHIP_BUTLER`, then `butler` on PATH, then the cached pinned copy,
/// downloading it if needed.
pub(crate) fn find_butler(
    env: &dyn Fn(&str) -> Option<OsString>,
    reporter: Reporter,
) -> Result<PathBuf> {
    if let Some(path) = env("GDSHIP_BUTLER").filter(|v| !v.is_empty()) {
        let path = PathBuf::from(path);
        if !path.is_file() {
            bail!(
                "butler binary {} from GDSHIP_BUTLER does not exist",
                path.display()
            );
        }
        return Ok(path);
    }
    if let Some(path) = find_on_path("butler", env) {
        return Ok(path);
    }
    let fetcher = Fetcher::new(Cache::from_env()?, reporter);
    install_pinned(&fetcher, &Pin::for_host()?, reporter)
}

/// Returns butler from `<cache>/butler/<version>/`, downloading and extracting the pinned
/// archive first unless a verified copy is already there.
pub fn install_pinned(fetcher: &Fetcher, pin: &Pin, reporter: Reporter) -> Result<PathBuf> {
    let dir = fetcher.cache().dir().join("butler").join(&pin.version);
    let exe = dir.join(exe_name());
    if dir.exists() {
        if cache_is_intact(&dir)? {
            return Ok(exe);
        }
        reporter.warn(format!(
            "cached butler in {} is damaged; downloading it again",
            dir.display()
        ));
        std::fs::remove_dir_all(&dir)
            .with_context(|| format!("cannot remove {}", dir.display()))?;
    }

    let archive = fetcher.fetch_pinned(&pin.url, &pin.sha256)?;
    let staging_root = fetcher.cache().temp_dir();
    std::fs::create_dir_all(&staging_root)
        .with_context(|| format!("cannot create {}", staging_root.display()))?;
    let staging = tempfile::tempdir_in(&staging_root)
        .with_context(|| format!("cannot create a folder in {}", staging_root.display()))?;
    let stage = staging.path().join("butler");
    let files = extract(archive.path(), &stage)?;
    if !files.iter().any(|f| f.path == exe_name()) {
        bail!("{} has no {} at its root", pin.url, exe_name());
    }
    let manifest: String = files
        .iter()
        .map(|f| format!("{}  {}\n", f.sha256, f.path))
        .collect();
    std::fs::write(stage.join(CACHE_MANIFEST), manifest)
        .with_context(|| format!("cannot write {}", stage.display()))?;

    let parent = dir.parent().expect("the cache dir has a parent");
    std::fs::create_dir_all(parent)
        .with_context(|| format!("cannot create {}", parent.display()))?;
    if let Err(e) = std::fs::rename(&stage, &dir) {
        // Another gdship may have finished the same install first.
        if !cache_is_intact(&dir).unwrap_or(false) {
            return Err(e).with_context(|| format!("cannot move butler into {}", dir.display()));
        }
    }
    Ok(exe)
}

/// Whether every file the manifest lists, including butler itself, still has its hash.
fn cache_is_intact(dir: &Path) -> Result<bool> {
    let manifest = match std::fs::read_to_string(dir.join(CACHE_MANIFEST)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", dir.display())),
    };
    let mut has_exe = false;
    for line in manifest.lines() {
        let Some((sha256, path)) = line.split_once("  ") else {
            return Ok(false);
        };
        let Ok(expected) = sha256.parse::<Sha256>() else {
            return Ok(false);
        };
        let file = path.split('/').fold(dir.to_owned(), |p, s| p.join(s));
        match Sha256::of_file(&file) {
            Ok(actual) if actual == expected => {}
            _ => return Ok(false),
        }
        has_exe |= path == exe_name();
    }
    Ok(has_exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_platform_has_a_pin() {
        for (os, arch, channel) in [
            ("windows", "x86_64", "windows-amd64"),
            ("windows", "aarch64", "windows-amd64"),
            ("linux", "x86_64", "linux-amd64"),
            ("linux", "aarch64", "linux-arm64"),
            ("macos", "x86_64", "darwin-amd64"),
            ("macos", "aarch64", "darwin-arm64"),
        ] {
            let pin = Pin::for_platform(os, arch).unwrap();
            assert_eq!(
                pin.url,
                format!(
                    "https://broth.itch.zone/butler/{channel}/{BUTLER_VERSION}/archive/default"
                )
            );
        }
        let err = Pin::for_platform("freebsd", "x86_64").unwrap_err();
        assert!(err.to_string().contains("set GDSHIP_BUTLER"), "{err}");
    }
}
