mod support;

use support::{Fixture, run};

fn tagged_repo() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.init_git();
    fixture.git(&["tag", "-a", "v0.3.0", "-m", "v0.3.0"]);
    fixture
}

fn exports(fixture: &Fixture) -> Vec<String> {
    fixture
        .calls_to("godot")
        .into_iter()
        .filter(|c| c.contains(&"--export-release".to_owned()))
        .map(|c| c[4].clone())
        .collect()
}

fn build_dir(fixture: &Fixture, channel: &str) -> String {
    fixture
        .path(&format!(".gdship/build/{channel}"))
        .display()
        .to_string()
}

#[test]
fn exports_everything_then_pushes_each_channel() {
    let fixture = tagged_repo();
    let result = run(fixture.gdship().arg("push"));
    assert_eq!(result.code, 0, "{result:?}");

    let calls = fixture.calls();
    let roles: Vec<(&str, &str)> = calls
        .iter()
        .map(|c| (c[0].as_str(), c.get(1).map_or("", String::as_str)))
        .collect();
    assert_eq!(
        roles,
        [
            ("godot", "--version"),
            ("godot", "--headless"),
            ("godot", "--headless"),
            ("godot", "--headless"),
            ("butler", "push"),
            ("butler", "push"),
            ("butler", "status"),
            ("butler", "status"),
        ]
    );
    let butler = fixture.calls_to("butler");
    assert_eq!(
        butler[0],
        [
            "push",
            &build_dir(&fixture, "html5"),
            "username/my-game:html5",
            "--userversion",
            "0.3.0",
            "--if-changed",
        ]
    );
    assert_eq!(butler[1][2], "username/my-game:windows");
    assert_eq!(butler[2], ["status", "username/my-game:html5"]);
    assert_eq!(butler[3], ["status", "username/my-game:windows"]);

    assert!(result.stdout.contains("Pushing html5 0.3.0"), "{result:?}");
    assert!(
        result
            .stdout
            .contains("Pushed windows 0.3.0 to username/my-game:windows"),
        "{result:?}"
    );
    assert!(
        result.stdout.contains("status of username/my-game:html5"),
        "{result:?}"
    );
}

#[test]
fn only_butler_sees_the_itch_api_key() {
    let mut fixture = tagged_repo();
    fixture.write("addons.toml", "");
    let env_log = fixture.temp.path().join("env.log");
    fixture.env("FAKE_ENV_LOG", &env_log);
    fixture.env("BUTLER_API_KEY", "secret");
    let result = run(fixture.gdship().args(["push", "--allow-dirty"]));
    assert_eq!(result.code, 0, "{result:?}");

    let log = std::fs::read_to_string(env_log).unwrap();
    let seen: Vec<(&str, &str)> = log
        .lines()
        .map(|line| line.split_once('\t').unwrap())
        .collect();
    for role in ["godot", "gdget", "butler"] {
        assert!(
            seen.iter().any(|(r, _)| *r == role),
            "{role} never ran: {log}"
        );
    }
    for (role, key) in seen {
        let expected = if role == "butler" {
            "secret"
        } else {
            "<unset>"
        };
        assert_eq!(key, expected, "{role}: {log}");
    }
}

#[test]
fn dirty_tree_is_refused_before_exporting() {
    let fixture = tagged_repo();
    fixture.write("scenes/new.tscn", "");
    let result = run(fixture.gdship().arg("push"));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(result.stderr.contains("?? scenes/"), "{result:?}");
    assert!(result.stderr.contains("--allow-dirty"), "{result:?}");
    assert!(exports(&fixture).is_empty());
    assert!(fixture.calls_to("butler").is_empty());
}

#[test]
fn allow_dirty_uses_git_describe() {
    let fixture = tagged_repo();
    fixture.write("project.godot", &format!("{}\n", support::PROJECT_GODOT));
    let result = run(fixture
        .gdship()
        .args(["push", "--allow-dirty", "--only", "html5"]));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(fixture.calls_to("butler")[0][4], "0.3.0-dirty");
}

#[test]
fn untagged_commits_are_versioned_by_git_describe() {
    let mut fixture = Fixture::new();
    fixture.init_git();
    let hash = fixture
        .git(&["rev-parse", "--short", "HEAD"])
        .trim()
        .to_owned();
    let result = run(fixture.gdship().args(["push", "--only", "html5"]));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(fixture.calls_to("butler")[0][4], hash);
    assert!(!result.stderr.contains("warning"), "{result:?}");

    fixture.git(&["tag", "-a", "v0.3.0", "-m", "v0.3.0"]);
    fixture.write("more.gd", "");
    fixture.commit();
    let hash = fixture
        .git(&["rev-parse", "--short", "HEAD"])
        .trim()
        .to_owned();
    let result = run(fixture.gdship().args(["push", "--only", "html5"]));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(fixture.calls_to("butler")[2][4], format!("0.3.0-1-g{hash}"));
}

#[test]
fn projects_outside_git_let_itch_number_the_builds() {
    let fixture = Fixture::new();
    fixture.write("gdship.toml", "itch = \"username/my-game\"\n");
    let result = run(fixture.gdship().arg("push"));
    assert_eq!(result.code, 0, "{result:?}");
    assert!(
        result.stderr.contains("is not in a git repository"),
        "{result:?}"
    );
    assert!(
        result.stderr.contains("itch.io will number the builds"),
        "{result:?}"
    );
    let butler = fixture.calls_to("butler");
    assert_eq!(
        butler[0],
        [
            "push",
            &build_dir(&fixture, "html5"),
            "username/my-game:html5",
            "--if-changed",
        ]
    );
    assert!(
        result.stdout.contains("Pushed html5 to username"),
        "{result:?}"
    );

    let result = run(fixture
        .gdship()
        .args(["push", "--version", "7", "--only", "html5"]));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(fixture.calls_to("butler")[4][3..5], ["--userversion", "7"]);
    assert!(!result.stderr.contains("number the builds"), "{result:?}");

    let result = run(fixture.gdship().args(["push", "--tag", "v1.0.0"]));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(
        result.stderr.contains("--tag needs a git repository"),
        "{result:?}"
    );
}

#[test]
fn repositories_without_commits_let_itch_number_the_builds() {
    let mut fixture = Fixture::new();
    fixture.init_git();
    std::fs::remove_dir_all(fixture.path(".git")).unwrap();
    fixture.git(&["init", "--quiet"]);
    let result = run(fixture
        .gdship()
        .args(["push", "--allow-dirty", "--only", "html5"]));
    assert_eq!(result.code, 0, "{result:?}");
    assert!(result.stderr.contains("no commits"), "{result:?}");
    assert!(!fixture.calls_to("butler")[0].contains(&"--userversion".to_owned()));
}

#[test]
fn explicit_version_skips_the_tag_lookup() {
    let mut fixture = Fixture::new();
    fixture.init_git();
    let result = run(fixture
        .gdship()
        .args(["push", "--version", "v1.0-rc", "--only", "windows"]));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(fixture.calls_to("butler")[0][4], "v1.0-rc");
}

#[test]
fn unignored_state_dir_is_refused() {
    let fixture = tagged_repo();
    fixture.write(".gitignore", "*.tmp\n");
    fixture.commit();
    fixture.git(&["tag", "-f", "-a", "v0.3.0", "-m", "v0.3.0"]);
    let result = run(fixture.gdship().arg("push"));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(
        result.stderr.contains("add `.gdship/` to .gitignore"),
        "{result:?}"
    );
}

#[test]
fn push_needs_gdship_toml() {
    let mut fixture = Fixture::new();
    fixture.init_git();
    std::fs::remove_file(fixture.path("gdship.toml")).unwrap();
    let result = run(fixture.gdship().args(["push", "--version", "1"]));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(result.stderr.contains("no gdship.toml"), "{result:?}");
}

#[test]
fn tag_is_created_pushed_and_used_as_the_version() {
    let mut fixture = Fixture::new();
    fixture.init_git();
    let result = run(fixture.gdship().args(["push", "--tag", "v1.4.0"]));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(fixture.tags(), ["v1.4.0"]);
    assert_eq!(fixture.calls_to("butler")[0][4], "1.4.0");
    let message = fixture.git(&[
        "tag",
        "-l",
        "--format=%(objecttype) %(contents:subject)",
        "v1.4.0",
    ]);
    assert_eq!(message.trim(), "tag v1.4.0");
    let remote = std::process::Command::new("git")
        .arg("-C")
        .arg(fixture.remote())
        .args(["tag", "--list"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&remote.stdout).trim(), "v1.4.0");
}

#[test]
fn existing_tag_is_refused() {
    let fixture = tagged_repo();
    let result = run(fixture.gdship().args(["push", "--tag", "v0.3.0"]));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(
        result.stderr.contains("tag v0.3.0 already exists"),
        "{result:?}"
    );
}

#[test]
fn tag_is_rolled_back_when_an_export_fails() {
    let mut fixture = Fixture::new();
    fixture.init_git();
    fixture.env("FAKE_GODOT_SILENT", "Windows Desktop");
    let result = run(fixture.gdship().args(["push", "--tag", "v2.0.0"]));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(
        result.stderr.contains("export of channel `windows` failed"),
        "{result:?}"
    );
    assert!(result.stdout.contains("Deleted tag v2.0.0"), "{result:?}");
    assert!(fixture.tags().is_empty());
    assert!(
        fixture.calls_to("butler").is_empty(),
        "nothing may be pushed"
    );
}

#[test]
fn tag_is_rolled_back_when_the_first_push_fails() {
    let mut fixture = Fixture::new();
    fixture.init_git();
    fixture.env("FAKE_BUTLER_FAIL", "html5");
    let result = run(fixture.gdship().args(["push", "--tag", "v2.0.0"]));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(
        result.stderr.contains("not pushed: html5, windows"),
        "{result:?}"
    );
    assert!(fixture.tags().is_empty());
}

#[test]
fn partial_push_failure_lists_what_was_pushed() {
    let mut fixture = tagged_repo();
    fixture.write(
        "gdship.toml",
        "itch = \"username/my-game\"\n[channels]\nhtml5 = \"Web\"\nwindows = \"Windows Desktop\"\nwin-beta = \"Windows Desktop\"\n",
    );
    fixture.commit();
    fixture.git(&["tag", "-f", "-a", "v0.3.0", "-m", "v0.3.0"]);
    fixture.env("FAKE_BUTLER_FAIL", "windows");
    let result = run(fixture.gdship().arg("push"));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(
        result
            .stderr
            .contains("pushing username/my-game:windows failed"),
        "{result:?}"
    );
    assert!(result.stderr.contains("pushed:     html5\n"), "{result:?}");
    assert!(
        result.stderr.contains("not pushed: windows, win-beta"),
        "{result:?}"
    );
    assert!(
        result
            .stderr
            .contains("gdship push --only windows --only win-beta"),
        "{result:?}"
    );
    let pushes: Vec<_> = fixture
        .calls_to("butler")
        .into_iter()
        .map(|c| c[2].clone())
        .collect();
    assert_eq!(
        pushes,
        ["username/my-game:html5", "username/my-game:windows"]
    );
}

#[test]
fn failed_tag_push_warns_with_the_retry_command() {
    let mut fixture = Fixture::new();
    fixture.init_git();
    fixture.git(&["remote", "remove", "origin"]);
    let result = run(fixture.gdship().args(["push", "--tag", "v3.0.0"]));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(
        result.stderr.contains("warning: every channel was pushed"),
        "{result:?}"
    );
    assert!(
        result.stderr.contains("git push origin v3.0.0"),
        "{result:?}"
    );
    assert_eq!(
        fixture.tags(),
        ["v3.0.0"],
        "the release exists, so the tag stays"
    );
    assert_eq!(fixture.calls_to("butler").len(), 4);
}

#[test]
fn dry_run_runs_neither_tool() {
    let mut fixture = Fixture::new();
    fixture.init_git();
    let result = run(fixture
        .gdship()
        .args(["push", "--dry-run", "--tag", "v5.0.0"]));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(fixture.calls_to("godot"), [["--version"]]);
    assert!(fixture.calls_to("butler").is_empty());
    assert!(fixture.tags().is_empty());
    assert!(!fixture.path(".gdship").exists());

    let out = &result.stdout;
    assert!(
        out.contains("Would run git tag -a v5.0.0 -m v5.0.0"),
        "{result:?}"
    );
    assert!(out.contains("--import"), "{result:?}");
    assert!(
        out.contains("--export-release \"Windows Desktop\""),
        "{result:?}"
    );
    assert!(
        out.contains("username/my-game:html5 --userversion 5.0.0 --if-changed"),
        "{result:?}"
    );
    assert!(
        out.contains("Would run git push origin refs/tags/v5.0.0"),
        "{result:?}"
    );
}

#[test]
fn dry_run_still_checks_everything() {
    let fixture = tagged_repo();
    fixture.write("untracked.txt", "");
    let result = run(fixture.gdship().args(["push", "--dry-run"]));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(result.stderr.contains("uncommitted changes"), "{result:?}");
}

#[test]
fn login_runs_butler_login() {
    let fixture = Fixture::new();
    let result = run(fixture.gdship().arg("login"));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(fixture.calls_to("butler"), [["login"]]);
}

#[test]
fn status_asks_butler_about_each_channel() {
    let mut fixture = Fixture::new();
    fixture.init_git();
    let result = run(fixture.gdship().arg("status"));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(
        fixture.calls_to("butler"),
        [
            ["status", "username/my-game:html5"],
            ["status", "username/my-game:windows"],
        ]
    );
    assert!(
        fixture.calls_to("godot").is_empty(),
        "status must not need Godot"
    );
}

#[test]
fn missing_butler_override_is_an_error() {
    let mut fixture = tagged_repo();
    fixture.env("GDSHIP_BUTLER", fixture.temp.path().join("nope"));
    let result = run(fixture.gdship().arg("push"));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(
        result.stderr.contains("from GDSHIP_BUTLER does not exist"),
        "{result:?}"
    );
    assert!(
        exports(&fixture).is_empty(),
        "butler is found before exporting"
    );
}
