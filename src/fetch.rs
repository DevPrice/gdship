use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tempfile::NamedTempFile;

use crate::digest::{Sha256, copy_hashed};
use crate::report::Reporter;

/// Per-user cache of downloaded tools.
#[derive(Debug, Clone)]
pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn from_env() -> Result<Self> {
        cache_dir_from(|key| std::env::var_os(key))
            .map(Self::new)
            .ok_or_else(|| anyhow!("cannot find a cache directory; set GDSHIP_CACHE_DIR"))
    }

    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// Where downloads and extractions are staged. It is inside the cache so the final
    /// rename stays on one volume.
    pub(crate) fn temp_dir(&self) -> PathBuf {
        self.dir.join("tmp")
    }
}

fn cache_dir_from(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let var = |key: &str| env(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(dir) = var("GDSHIP_CACHE_DIR") {
        return Some(dir);
    }
    if cfg!(windows) {
        return var("LOCALAPPDATA").map(|dir| dir.join("gdship"));
    }
    if let Some(dir) = var("XDG_CACHE_HOME") {
        return Some(dir.join("gdship"));
    }
    let home = var("HOME")?;
    if cfg!(target_os = "macos") {
        Some(home.join("Library").join("Caches").join("gdship"))
    } else {
        Some(home.join(".cache").join("gdship"))
    }
}

/// The largest file gdship downloads. The hash is only checked once a download ends, so
/// without a cap a hostile server could fill the disk before being caught.
pub const MAX_DOWNLOAD_BYTES: u64 = 1 << 30;

/// Downloads files into the cache's staging area, verifying every one it hands out.
pub struct Fetcher {
    agent: ureq::Agent,
    cache: Cache,
    max_bytes: u64,
    reporter: Reporter,
}

impl Fetcher {
    pub fn new(cache: Cache, reporter: Reporter) -> Self {
        let config = ureq::Agent::config_builder()
            .user_agent(concat!("gdship/", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_recv_body(Some(Duration::from_secs(30 * 60)))
            .build();
        Self {
            agent: config.into(),
            cache,
            max_bytes: MAX_DOWNLOAD_BYTES,
            reporter,
        }
    }

    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    /// Downloads `url`, failing unless its sha256 is `expected`. The file is deleted when
    /// the returned handle drops.
    pub fn fetch_pinned(&self, url: &str, expected: &Sha256) -> Result<NamedTempFile> {
        let (actual, temp) = self.download(url)?;
        if actual != *expected {
            bail!(
                "sha256 mismatch for {url}\n  expected {expected}\n  got      {actual}\n\
                 The file changed after gdship pinned it. Install butler yourself and set \
                 GDSHIP_BUTLER, or update gdship."
            );
        }
        Ok(temp)
    }

    fn download(&self, url: &str) -> Result<(Sha256, NamedTempFile)> {
        self.reporter.action("Downloading", url);
        let response = self
            .agent
            .get(url)
            .call()
            .with_context(|| format!("cannot download {url}"))?;

        let temp_dir = self.cache.temp_dir();
        std::fs::create_dir_all(&temp_dir)
            .with_context(|| format!("cannot create {}", temp_dir.display()))?;
        let mut temp = NamedTempFile::new_in(&temp_dir)
            .with_context(|| format!("cannot create a file in {}", temp_dir.display()))?;
        let mut body = response.into_body().into_reader().take(self.max_bytes + 1);
        let (size, sha256) = copy_hashed(&mut body, temp.as_file_mut())
            .with_context(|| format!("cannot download {url}"))?;
        if size > self.max_bytes {
            bail!(
                "cannot download {url}: it is larger than the {} MiB limit",
                self.max_bytes >> 20
            );
        }
        Ok((sha256, temp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_cache_dir_wins() {
        let env = |key: &str| match key {
            "GDSHIP_CACHE_DIR" => Some(OsString::from("/explicit")),
            _ => Some(OsString::from("/other")),
        };
        assert_eq!(cache_dir_from(env), Some(PathBuf::from("/explicit")));
    }

    #[test]
    fn platform_cache_dir() {
        let env = |key: &str| match key {
            "LOCALAPPDATA" => Some(OsString::from("C:\\Users\\me\\AppData\\Local")),
            "XDG_CACHE_HOME" => Some(OsString::from("/xdg")),
            "HOME" => Some(OsString::from("/home/me")),
            _ => None,
        };
        let expected = if cfg!(windows) {
            PathBuf::from("C:\\Users\\me\\AppData\\Local").join("gdship")
        } else {
            PathBuf::from("/xdg").join("gdship")
        };
        assert_eq!(cache_dir_from(env), Some(expected));
    }

    #[test]
    fn empty_vars_are_ignored() {
        let env = |key: &str| match key {
            "GDSHIP_CACHE_DIR" | "XDG_CACHE_HOME" => Some(OsString::new()),
            "LOCALAPPDATA" => Some(OsString::from("L")),
            "HOME" => Some(OsString::from("H")),
            _ => None,
        };
        let expected = if cfg!(windows) {
            PathBuf::from("L").join("gdship")
        } else if cfg!(target_os = "macos") {
            PathBuf::from("H")
                .join("Library")
                .join("Caches")
                .join("gdship")
        } else {
            PathBuf::from("H").join(".cache").join("gdship")
        };
        assert_eq!(cache_dir_from(env), Some(expected));
    }
}
