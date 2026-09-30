mod support;

use support::{Fixture, run};

#[test]
fn writes_gdship_toml_from_the_answer() {
    let fixture = Fixture::new();
    let result = run(fixture
        .gdship()
        .arg("init")
        .write_stdin("https://devprice.itch.io/idle-factory\n"));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(
        std::fs::read_to_string(fixture.path("gdship.toml")).unwrap(),
        "itch = \"devprice/idle-factory\"\n"
    );
    assert!(result.stderr.contains("itch.io game"), "{result:?}");
    assert!(result.stdout.contains("Created"), "{result:?}");

    let result = run(fixture
        .gdship()
        .args(["push", "--dry-run", "--version", "1"]));
    assert!(!result.stderr.contains("no gdship.toml"), "{result:?}");
}

#[test]
fn works_from_a_subdirectory() {
    let fixture = Fixture::new();
    fixture.write("scenes/main.tscn", "");
    let result = run(fixture
        .gdship()
        .current_dir(fixture.path("scenes"))
        .arg("init")
        .write_stdin("devprice/idle-factory\n"));
    assert_eq!(result.code, 0, "{result:?}");
    assert!(fixture.path("gdship.toml").is_file());
}

#[test]
fn never_overwrites_an_existing_file() {
    let fixture = Fixture::new();
    fixture.write("gdship.toml", "itch = \"someone/else\"\n");
    let result = run(fixture
        .gdship()
        .arg("init")
        .write_stdin("devprice/idle-factory\n"));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(result.stderr.contains("already exists"), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(fixture.path("gdship.toml")).unwrap(),
        "itch = \"someone/else\"\n"
    );
}

#[test]
fn explains_bad_answers_and_writes_nothing() {
    let fixture = Fixture::new();
    let result = run(fixture.gdship().arg("init").write_stdin("devprice\nnope\n"));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(
        result.stderr.contains("expected `<user>/<game>`"),
        "{result:?}"
    );
    assert!(!fixture.path("gdship.toml").exists());
}

#[test]
fn needs_a_godot_project() {
    let fixture = Fixture::new();
    std::fs::remove_file(fixture.path("project.godot")).unwrap();
    let result = run(fixture
        .gdship()
        .arg("init")
        .write_stdin("devprice/idle-factory\n"));
    assert_eq!(result.code, 1, "{result:?}");
    assert!(
        result.stderr.contains("no project.godot found"),
        "{result:?}"
    );
}

#[test]
fn adds_the_state_dir_to_an_existing_gitignore() {
    let fixture = Fixture::new();
    fixture.write(".gitignore", ".godot/\n");
    let result = run(fixture
        .gdship()
        .arg("init")
        .write_stdin("devprice/idle-factory\n"));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(
        std::fs::read_to_string(fixture.path(".gitignore")).unwrap(),
        ".godot/\n/.gdship/\n"
    );
    assert!(result.stdout.contains("Updated .gitignore"), "{result:?}");
}

#[test]
fn leaves_gitignore_alone_when_absent_or_already_ignoring() {
    let fixture = Fixture::new();
    let result = run(fixture
        .gdship()
        .arg("init")
        .write_stdin("devprice/idle-factory\n"));
    assert_eq!(result.code, 0, "{result:?}");
    assert!(!fixture.path(".gitignore").exists());

    let fixture = Fixture::new();
    fixture.write(".gitignore", ".gdship/\n");
    let result = run(fixture
        .gdship()
        .arg("init")
        .write_stdin("devprice/idle-factory\n"));
    assert_eq!(result.code, 0, "{result:?}");
    assert_eq!(
        std::fs::read_to_string(fixture.path(".gitignore")).unwrap(),
        ".gdship/\n"
    );
    assert!(!result.stdout.contains("Updated"), "{result:?}");
}
