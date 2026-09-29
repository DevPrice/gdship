mod support;

use std::path::Path;

use gdship::butler::{Pin, install_pinned};
use gdship::digest::Sha256;
use gdship::fetch::{Cache, Fetcher};
use gdship::report::Reporter;
use support::{TestServer, zip_bytes};

const EXE: &str = std::env::consts::EXE_SUFFIX;

fn butler_zip() -> Vec<u8> {
    zip_bytes(&[
        (&format!("butler{EXE}"), "butler binary"),
        ("7z.dll", "dll"),
    ])
}

fn fetcher(cache: &Path) -> Fetcher {
    Fetcher::new(Cache::new(cache.to_owned()), Reporter::from_env())
}

fn pin(url: String, bytes: &[u8]) -> Pin {
    Pin {
        version: "1.2.3".into(),
        url,
        sha256: Sha256::of_bytes(bytes),
    }
}

fn install(cache: &Path, pin: &Pin) -> anyhow::Result<std::path::PathBuf> {
    install_pinned(&fetcher(cache), pin, Reporter::from_env())
}

#[test]
fn downloads_extracts_and_reuses_the_cache() {
    let server = TestServer::start();
    let bytes = butler_zip();
    let pin = pin(server.serve("/butler.zip", bytes.clone()), &bytes);
    let cache = tempfile::tempdir().unwrap();

    let exe = install(cache.path(), &pin).unwrap();
    let dir = cache.path().join("butler").join("1.2.3");
    assert_eq!(exe, dir.join(format!("butler{EXE}")));
    assert_eq!(std::fs::read_to_string(&exe).unwrap(), "butler binary");
    assert_eq!(std::fs::read_to_string(dir.join("7z.dll")).unwrap(), "dll");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    assert_eq!(install(cache.path(), &pin).unwrap(), exe);
    assert_eq!(
        server.requests().len(),
        1,
        "the second install must use the cache"
    );
}

#[test]
fn hash_mismatch_is_an_error_and_nothing_is_cached() {
    let server = TestServer::start();
    let pin = pin(server.serve("/butler.zip", butler_zip()), b"something else");
    let cache = tempfile::tempdir().unwrap();

    let message = format!("{:#}", install(cache.path(), &pin).unwrap_err());
    assert!(message.contains("sha256 mismatch"), "{message}");
    assert!(message.contains(&pin.sha256.to_string()), "{message}");
    assert!(
        message.contains(&Sha256::of_bytes(&butler_zip()).to_string()),
        "{message}"
    );
    assert!(!cache.path().join("butler").exists());
}

#[test]
fn damaged_cache_entry_is_downloaded_again() {
    let server = TestServer::start();
    let bytes = butler_zip();
    let pin = pin(server.serve("/butler.zip", bytes.clone()), &bytes);
    let cache = tempfile::tempdir().unwrap();
    let exe = install(cache.path(), &pin).unwrap();

    std::fs::write(&exe, "bit rot").unwrap();
    assert_eq!(install(cache.path(), &pin).unwrap(), exe);
    assert_eq!(std::fs::read_to_string(&exe).unwrap(), "butler binary");
    assert_eq!(server.requests().len(), 2);

    std::fs::remove_file(exe.with_file_name("7z.dll")).unwrap();
    install(cache.path(), &pin).unwrap();
    assert!(exe.with_file_name("7z.dll").is_file());
    assert_eq!(server.requests().len(), 3);
}

#[test]
fn half_written_cache_entry_is_replaced() {
    let server = TestServer::start();
    let bytes = butler_zip();
    let pin = pin(server.serve("/butler.zip", bytes.clone()), &bytes);
    let cache = tempfile::tempdir().unwrap();
    let dir = cache.path().join("butler").join("1.2.3");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("butler{EXE}")), "partial").unwrap();

    let exe = install(cache.path(), &pin).unwrap();
    assert_eq!(std::fs::read_to_string(exe).unwrap(), "butler binary");
}

#[test]
fn archive_without_butler_is_an_error() {
    let server = TestServer::start();
    let bytes = zip_bytes(&[("readme.txt", "no butler here")]);
    let pin = pin(server.serve("/butler.zip", bytes.clone()), &bytes);
    let cache = tempfile::tempdir().unwrap();

    let err = install(cache.path(), &pin).unwrap_err();
    assert!(err.to_string().contains("has no butler"), "{err}");
    assert!(!cache.path().join("butler").exists());
}

#[test]
fn http_errors_name_the_url() {
    let server = TestServer::start();
    let pin = pin(server.url("/missing.zip"), b"x");
    let cache = tempfile::tempdir().unwrap();

    let message = format!("{:#}", install(cache.path(), &pin).unwrap_err());
    assert!(message.contains(&pin.url), "{message}");
    assert!(message.contains("404"), "{message}");
}

#[test]
fn oversized_downloads_are_refused() {
    let server = TestServer::start();
    let url = server.serve("/big.zip", vec![0u8; 4096]);
    let cache = tempfile::tempdir().unwrap();
    let err = fetcher(cache.path())
        .with_max_bytes(1024)
        .fetch_pinned(&url, &Sha256::of_bytes(b""))
        .unwrap_err();
    assert!(err.to_string().contains("larger than"), "{err}");
}

#[test]
#[ignore = "downloads butler from itch.io; run after changing the pins"]
fn host_pin_matches_the_published_archive() {
    let cache = tempfile::tempdir().unwrap();
    let exe = install(cache.path(), &Pin::for_host().unwrap()).unwrap();
    let output = std::process::Command::new(exe)
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let version = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(
        version.contains(gdship::butler::BUTLER_VERSION),
        "{version}"
    );
}
