mod support;

use support::{Fixture, run};

fn build(fixture: &Fixture, relative: &str) -> std::path::PathBuf {
    fixture.path(&format!(".gdship/build/{relative}"))
}

#[test]
fn exports_every_derived_channel() {
    let fixture = Fixture::new();
    let result = run(fixture.gdship().arg("export"));
    assert_eq!(result.code, 0, "{result:?}");

    assert_eq!(
        std::fs::read_to_string(build(&fixture, "html5/index.html")).unwrap(),
        "built Web"
    );
    assert!(build(&fixture, "html5/index.wasm").is_file());
    assert!(build(&fixture, "html5/index.pck").is_file());
    assert_eq!(
        std::fs::read_to_string(build(&fixture, "windows/idlefactory.exe")).unwrap(),
        "built Windows Desktop"
    );
    assert!(fixture.path(".gdship/.gdignore").is_file());
    let import_log = std::fs::read_to_string(fixture.path(".gdship/logs/import.log")).unwrap();
    assert!(import_log.contains("importing on stderr"), "{import_log}");
    let web_log = std::fs::read_to_string(fixture.path(".gdship/logs/html5.log")).unwrap();
    assert!(web_log.contains("export line 50 for Web"), "{web_log}");

    let root = fixture.root().display().to_string();
    let calls = fixture.calls_to("godot");
    assert_eq!(calls[0], ["--version"]);
    assert_eq!(calls[1], ["--headless", "--path", &root, "--import"]);
    let output = build(&fixture, "html5/index.html");
    assert_eq!(
        calls[2],
        [
            "--headless",
            "--path",
            &root,
            "--export-release",
            "Web",
            &output.display().to_string()
        ]
    );
    assert_eq!(calls[3][4], "Windows Desktop");
    assert_eq!(calls.len(), 4);

    assert!(result.stdout.contains("Exporting html5"), "{result:?}");
    assert!(!result.stdout.contains("export line"), "{result:?}");
}

#[test]
fn only_limits_the_channels_and_rejects_unknown_ones() {
    let fixture = Fixture::new();
    let result = run(fixture.gdship().args(["export", "--only", "windows"]));
    assert_eq!(result.code, 0, "{result:?}");
    assert!(build(&fixture, "windows/idlefactory.exe").is_file());
    assert!(!build(&fixture, "html5").exists());

    let result = run(fixture.gdship().args(["export", "--only", "linux"]));
    assert_eq!(result.code, 2, "{result:?}");
    assert!(
        result
            .stderr
            .contains("--only linux: no such channel; channels: html5, windows"),
        "{result:?}"
    );
}

#[test]
fn channels_table_picks_presets_and_order() {
    let fixture = Fixture::new();
    fixture.write(
        "gdship.toml",
        "itch = \"devprice/idle-factory\"\n[channels]\nwin = \"Windows Desktop\"\nweb-build = \"Web\"\n",
    );
    let result = run(fixture.gdship().arg("export"));
    assert_eq!(result.code, 0, "{result:?}");
    let exported: Vec<_> = fixture
        .calls_to("godot")
        .into_iter()
        .filter(|c| c.contains(&"--export-release".to_owned()))
        .map(|c| c[4].clone())
        .collect();
    assert_eq!(exported, ["Windows Desktop", "Web"]);
    assert!(build(&fixture, "web-build/index.html").is_file());
}

#[test]
fn missing_templates_are_an_error() {
    let fixture = Fixture::new();
    std::fs::remove_dir(fixture.templates_root().join("4.7.2.stable")).unwrap();
    let result = run(fixture.gdship().arg("export"));
    assert_eq!(result.code, 1, "{result:?}");
    let folder = fixture.templates_root().join("4.7.2.stable");
    assert!(
        result.stderr.contains(&folder.display().to_string()),
        "{result:?}"
    );
    assert!(
        result
            .stderr
            .contains("Godot 4.7.2.stable.official.abc1234"),
        "{result:?}"
    );
    assert_eq!(fixture.calls_to("godot"), [["--version"]]);
}

#[test]
fn godot_version_mismatch_is_an_error() {
    let mut fixture = Fixture::new();
    fixture.env("FAKE_GODOT_VERSION", "4.6.3.stable.official.abc1234");
    let result = run(fixture.gdship().arg("export"));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(
        result.stderr.contains("is Godot 4.6.3.stable"),
        "{result:?}"
    );
    assert!(
        result.stderr.contains("project is for Godot 4.7"),
        "{result:?}"
    );
}

#[test]
fn silent_export_failure_is_caught_and_stops_the_run() {
    let mut fixture = Fixture::new();
    fixture.env("FAKE_GODOT_SILENT", "Web");
    let result = run(fixture.gdship().arg("export"));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(
        result.stderr.contains("export of channel `html5` failed"),
        "{result:?}"
    );
    assert!(result.stderr.contains("did not write"), "{result:?}");
    let log = fixture.path(".gdship/logs/html5.log");
    assert!(
        result.stderr.contains(&log.display().to_string()),
        "{result:?}"
    );
    assert!(
        result.stderr.contains("export line 20 for Web"),
        "{result:?}"
    );
    assert!(
        !result.stderr.contains("export line 5 for Web"),
        "{result:?}"
    );
    assert!(result.stderr.contains("exiting 0 anyway"), "{result:?}");
    assert!(
        !build(&fixture, "windows").exists(),
        "later channels must not run"
    );
}

#[test]
fn failed_export_reports_the_exit_code() {
    let mut fixture = Fixture::new();
    fixture.env("FAKE_GODOT_FAIL", "Windows Desktop");
    let result = run(fixture.gdship().arg("export"));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(
        result
            .stderr
            .contains("export of channel `windows` failed: Godot failed with exit code 1"),
        "{result:?}"
    );
    assert!(
        result.stderr.contains("Project export for preset"),
        "{result:?}"
    );
}

#[test]
fn failed_import_stops_before_exporting() {
    let mut fixture = Fixture::new();
    fixture.env("FAKE_GODOT_IMPORT_EXIT", "3");
    let result = run(fixture.gdship().arg("export"));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(
        result.stderr.contains("Godot's import failed"),
        "{result:?}"
    );
    assert_eq!(fixture.calls_to("godot").len(), 2);
}

#[test]
fn verbose_streams_godot_output() {
    let fixture = Fixture::new();
    let result = run(fixture.gdship().args(["export", "-v", "--only", "html5"]));
    assert_eq!(result.code, 0, "{result:?}");
    assert!(
        result.stdout.contains("export line 1 for Web"),
        "{result:?}"
    );
    assert!(result.stderr.contains("importing on stderr"), "{result:?}");
}

#[test]
fn build_dir_is_wiped_before_each_export() {
    let fixture = Fixture::new();
    fixture.write(".gdship/build/html5/stale.txt", "old");
    let result = run(fixture.gdship().args(["export", "--only", "html5"]));
    assert_eq!(result.code, 0, "{result:?}");
    assert!(!build(&fixture, "html5/stale.txt").exists());
}

#[test]
fn skipped_presets_are_noted() {
    let fixture = Fixture::new();
    let presets = format!(
        "{}\n[preset.2]\n\nname=\"iPhone\"\nplatform=\"iOS\"\nexport_path=\"\"\n",
        support::PRESETS
    );
    fixture.write("export_presets.cfg", &presets);
    let result = run(fixture.gdship().arg("export"));
    assert_eq!(result.code, 0, "{result:?}");
    assert!(
        result
            .stdout
            .contains("Skipping preset \"iPhone\": no channel for platform iOS"),
        "{result:?}"
    );
}

#[test]
fn works_from_a_subdirectory_and_honors_the_user_config() {
    let mut fixture = Fixture::new();
    std::fs::write(
        fixture.temp.path().join("config.toml"),
        format!("godot = {:?}", fixture.tool("godot").display().to_string()),
    )
    .unwrap();
    fixture.env("GDSHIP_GODOT", "");
    fixture.write("scenes/main.tscn", "");
    let result = run(fixture
        .gdship()
        .current_dir(fixture.path("scenes"))
        .args(["export", "--only", "windows"]));
    assert_eq!(result.code, 0, "{result:?}");
    assert!(build(&fixture, "windows/idlefactory.exe").is_file());
}
