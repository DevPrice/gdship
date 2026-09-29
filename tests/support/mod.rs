#![allow(dead_code, reason = "each tests/*.rs crate uses a different subset")]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub(crate) const PRESETS: &str = include_str!("../fixtures/export_presets.cfg");

pub(crate) const PROJECT_GODOT: &str = "; Engine configuration file.\n\nconfig_version=5\n\n[application]\n\nconfig/name=\"Idle Factory\"\nconfig/features=PackedStringArray(\"4.7\", \"GL Compatibility\")\n";

/// Builds `tests/fake-tool` once per test binary, into its own target dir so it doesn't
/// wait on the lock of the `cargo test` that is running us.
pub(crate) fn fake_tool() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let target_dir = root.join("target").join("fake-tool");
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "--quiet", "--package", "fake-tool", "--target-dir"])
            .arg(&target_dir)
            .current_dir(root)
            .status()
            .unwrap();
        assert!(status.success(), "building fake-tool failed");
        target_dir
            .join("debug")
            .join(format!("fake-tool{}", std::env::consts::EXE_SUFFIX))
    })
}

/// A Godot project with the fixture presets, installed export templates, and fake tools,
/// isolated from the user's own configuration.
pub(crate) struct Fixture {
    pub temp: tempfile::TempDir,
    root: PathBuf,
    bin: PathBuf,
    envs: Vec<(String, OsString)>,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("game");
        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(root.join("project.godot"), PROJECT_GODOT).unwrap();
        std::fs::write(root.join("export_presets.cfg"), PRESETS).unwrap();
        let fixture = Self {
            root,
            bin,
            envs: Vec::new(),
            temp,
        };
        for tool in ["godot", "butler", "gdget"] {
            fixture.install_tool(tool);
        }
        std::fs::create_dir_all(fixture.templates_root().join("4.7.2.stable")).unwrap();
        fixture
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn tool(&self, name: &str) -> PathBuf {
        self.bin
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
    }

    pub(crate) fn install_tool(&self, name: &str) {
        std::fs::copy(fake_tool(), self.tool(name)).unwrap();
    }

    pub(crate) fn remove_tool(&self, name: &str) {
        std::fs::remove_file(self.tool(name)).unwrap();
    }

    fn data_dir(&self) -> PathBuf {
        self.temp.path().join("data")
    }

    fn home_dir(&self) -> PathBuf {
        self.temp.path().join("home")
    }

    /// Where Godot keeps export templates, given the env [`Self::gdship`] sets.
    pub(crate) fn templates_root(&self) -> PathBuf {
        let dir = if cfg!(windows) {
            self.data_dir().join("Godot")
        } else if cfg!(target_os = "macos") {
            self.home_dir()
                .join("Library")
                .join("Application Support")
                .join("Godot")
        } else {
            self.data_dir().join("godot")
        };
        dir.join("export_templates")
    }

    pub(crate) fn log_path(&self) -> PathBuf {
        self.temp.path().join("tools.log")
    }

    /// The fake tools' invocations, each as its role followed by its arguments.
    pub(crate) fn calls(&self) -> Vec<Vec<String>> {
        match std::fs::read_to_string(self.log_path()) {
            Ok(text) => text
                .lines()
                .map(|line| line.split('\t').map(str::to_owned).collect())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// The calls made to one tool, without the role.
    pub(crate) fn calls_to(&self, role: &str) -> Vec<Vec<String>> {
        self.calls()
            .into_iter()
            .filter(|call| call[0] == role)
            .map(|call| call[1..].to_vec())
            .collect()
    }

    pub(crate) fn write(&self, relative: &str, contents: &str) {
        let path = self.path(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    /// A path in the project from a `/`-separated relative path.
    pub(crate) fn path(&self, relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(self.root.clone(), |path, s| path.join(s))
    }

    /// Sets an environment variable for every later [`Self::gdship`] run.
    pub(crate) fn env(&mut self, key: &str, value: impl Into<OsString>) {
        self.envs.push((key.to_owned(), value.into()));
    }

    /// PATH with the fake tools first, so they shadow any real ones.
    pub(crate) fn path_var(&self) -> OsString {
        let system = std::env::var_os("PATH").unwrap_or_default();
        std::env::join_paths(
            std::iter::once(self.bin.clone()).chain(std::env::split_paths(&system)),
        )
        .unwrap()
    }

    pub(crate) fn gdship(&self) -> assert_cmd::Command {
        let mut cmd = assert_cmd::Command::cargo_bin("gdship").unwrap();
        cmd.current_dir(&self.root)
            .env("GDSHIP_GODOT", self.tool("godot"))
            .env("GDSHIP_BUTLER", self.tool("butler"))
            .env("GDSHIP_CONFIG", self.temp.path().join("config.toml"))
            .env("GDSHIP_CACHE_DIR", self.temp.path().join("cache"))
            .env("APPDATA", self.data_dir())
            .env("XDG_DATA_HOME", self.data_dir())
            .env("HOME", self.home_dir())
            .env("PATH", self.path_var())
            .env("FAKE_LOG", self.log_path())
            .env("NO_COLOR", "1")
            .env_remove("GITHUB_ACTIONS")
            .env_remove("BUTLER_API_KEY")
            .envs(self.envs.iter().map(|(k, v)| (k, v)));
        cmd
    }
}

/// Output of a finished command, for asserting on text.
pub(crate) struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub(crate) fn run(cmd: &mut assert_cmd::Command) -> Run {
    let output = cmd.output().unwrap();
    Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

impl std::fmt::Debug for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "exit {}\n--- stdout\n{}--- stderr\n{}",
            self.code, self.stdout, self.stderr
        )
    }
}

/// A local HTTP server for fixture downloads, so tests never touch the network.
pub(crate) struct TestServer {
    server: std::sync::Arc<tiny_http::Server>,
    base: String,
    routes: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>>,
    requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl TestServer {
    pub(crate) fn start() -> Self {
        use std::sync::{Arc, Mutex};
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let routes = Arc::new(Mutex::new(
            std::collections::HashMap::<String, Vec<u8>>::new(),
        ));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let server = Arc::clone(&server);
            let routes = Arc::clone(&routes);
            let requests = Arc::clone(&requests);
            std::thread::spawn(move || {
                for request in server.incoming_requests() {
                    let path = request.url().to_owned();
                    requests.lock().unwrap().push(path.clone());
                    let body = routes.lock().unwrap().get(&path).cloned();
                    let _ = match body {
                        Some(body) => request.respond(tiny_http::Response::from_data(body)),
                        None => request.respond(tiny_http::Response::empty(404)),
                    };
                }
            })
        };
        Self {
            server,
            base,
            routes,
            requests,
            thread: Some(thread),
        }
    }

    /// Serves `body` at `path` (e.g. "/butler.zip") and returns its full URL.
    pub(crate) fn serve(&self, path: &str, body: impl Into<Vec<u8>>) -> String {
        self.routes
            .lock()
            .unwrap()
            .insert(path.to_owned(), body.into());
        self.url(path)
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub(crate) fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Builds a zip archive in memory from `(path, contents)` pairs.
pub(crate) fn zip_bytes(files: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
    for (path, contents) in files {
        zip.start_file(*path, options).unwrap();
        zip.write_all(contents.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
