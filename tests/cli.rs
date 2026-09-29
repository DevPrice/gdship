use assert_cmd::Command;

fn gdship() -> Command {
    let mut cmd = Command::cargo_bin("gdship").unwrap();
    cmd.env_remove("GITHUB_ACTIONS");
    cmd
}

#[test]
fn usage_error_exits_2() {
    gdship().arg("frobnicate").assert().code(2);
}

#[test]
fn push_rejects_version_with_tag() {
    gdship()
        .args(["push", "--version", "1.0", "--tag", "v1.0"])
        .assert()
        .code(2);
}
