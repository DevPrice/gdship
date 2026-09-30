use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};

use crate::process::describe_exit;

/// Most dirty paths listed in the clean-tree error.
const DIRTY_PATHS_SHOWN: usize = 10;

/// Runs git in the project root.
#[derive(Debug, Clone)]
pub(crate) struct Git {
    root: PathBuf,
}

impl Git {
    pub(crate) fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
        }
    }

    fn output(&self, args: &[&str]) -> Result<Output> {
        Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .output()
            .context("cannot run git; is it installed and on PATH?")
    }

    /// Runs git and returns its trimmed stdout, failing with its stderr.
    fn stdout(&self, args: &[&str]) -> Result<String> {
        let output = self.output(args)?;
        if !output.status.success() {
            bail!(
                "`git {}` failed with {}: {}",
                args.join(" "),
                describe_exit(output.status),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    /// Fails if anything is uncommitted, untracked, or a modified submodule, since all of
    /// it would end up in the build.
    pub(crate) fn check_clean(&self) -> Result<()> {
        let status = self.stdout(&["status", "--porcelain"])?;
        if status.is_empty() {
            return Ok(());
        }
        let lines: Vec<&str> = status.lines().collect();
        let mut shown = lines[..lines.len().min(DIRTY_PATHS_SHOWN)].join("\n  ");
        if lines.len() > DIRTY_PATHS_SHOWN {
            shown.push_str(&format!(
                "\n  ... and {} more",
                lines.len() - DIRTY_PATHS_SHOWN
            ));
        }
        bail!(
            "the working tree has uncommitted changes, which would end up in the build:\n  \
             {shown}\nCommit or stash them, or pass --allow-dirty"
        )
    }

    /// Fails unless `.gdship/` is ignored. Otherwise the builds make the tree dirty and
    /// the next push refuses to run.
    pub(crate) fn check_state_dir_ignored(&self) -> Result<()> {
        let output = self.output(&["check-ignore", "-q", ".gdship/"])?;
        match output.status.code() {
            Some(0) => Ok(()),
            Some(1) => bail!(
                ".gdship/ is not ignored by git; add `.gdship/` to .gitignore, or the builds \
                 will make the next push see a dirty tree"
            ),
            _ => bail!(
                "`git check-ignore` failed with {}: {}",
                describe_exit(output.status),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        }
    }

    /// Whether the project is inside a git work tree. A missing git counts as no.
    pub(crate) fn is_repo(&self) -> Result<bool> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["rev-parse", "--is-inside-work-tree"])
            .output();
        match output {
            Ok(output) => {
                Ok(output.status.success()
                    && String::from_utf8_lossy(&output.stdout).trim() == "true")
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e).context("cannot run git"),
        }
    }

    /// Whether this is a shallow clone, which may be missing the tags `describe` needs.
    pub(crate) fn is_shallow(&self) -> Result<bool> {
        Ok(self.stdout(&["rev-parse", "--is-shallow-repository"])? == "true")
    }

    pub(crate) fn tag_exists(&self, tag: &str) -> Result<bool> {
        let output = self.output(&[
            "rev-parse",
            "--quiet",
            "--verify",
            &format!("refs/tags/{tag}"),
        ])?;
        Ok(output.status.success())
    }

    /// Names HEAD with `git describe`, marking uncommitted changes with `-dirty`.
    pub(crate) fn head_version(&self) -> Result<HeadVersion> {
        let head = self.output(&["rev-parse", "--quiet", "--verify", "HEAD"])?;
        if !head.status.success() {
            return Ok(HeadVersion::NoCommits);
        }
        let tagged = self.output(&["describe", "--tags", "--dirty"])?;
        if tagged.status.success() {
            let described = String::from_utf8_lossy(&tagged.stdout);
            return Ok(HeadVersion::Tagged(strip_v(described.trim()).to_owned()));
        }
        Ok(HeadVersion::Untagged(
            self.stdout(&["describe", "--always", "--dirty"])?,
        ))
    }

    /// The version a new tag gives, failing if the tag already exists.
    pub(crate) fn new_tag_version(&self, tag: &str) -> Result<String> {
        if self.tag_exists(tag)? {
            bail!(
                "tag {tag} already exists; pick a new one, or check it out and push without --tag"
            );
        }
        Ok(strip_v(tag).to_owned())
    }

    /// Creates an annotated tag on HEAD, with the tag name as its message.
    pub(crate) fn create_tag(&self, tag: &str) -> Result<()> {
        self.stdout(&["tag", "-a", tag, "-m", tag]).map(drop)
    }

    pub(crate) fn delete_tag(&self, tag: &str) -> Result<()> {
        self.stdout(&["tag", "-d", tag]).map(drop)
    }

    pub(crate) fn push_tag(&self, tag: &str) -> Result<()> {
        self.stdout(&["push", "origin", &format!("refs/tags/{tag}")])
            .map(drop)
    }
}

/// HEAD as `git describe` names it. Tags lose a leading `v` before a digit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HeadVersion {
    /// `0.3.0` on the tag, `0.3.0-4-gabc1234` four commits after it.
    Tagged(String),
    /// No tag is reachable, so just the short commit hash.
    Untagged(String),
    /// The repository has nothing committed to name.
    NoCommits,
}

/// `v0.3.0` becomes `0.3.0`; `version2` and `vv1` stay as they are.
pub(crate) fn strip_v(tag: &str) -> &str {
    match tag.strip_prefix('v') {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest,
        _ => tag,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A repository with one commit, with git's identity and user config set per command.
    pub(crate) struct Repo {
        pub(crate) dir: tempfile::TempDir,
    }

    impl Repo {
        pub(crate) fn new() -> Self {
            let repo = Self {
                dir: tempfile::tempdir().unwrap(),
            };
            repo.git(&["init", "--quiet", "-b", "main"]);
            repo.write(".gitignore", ".gdship/\n");
            repo.write("project.godot", "");
            repo.commit();
            repo
        }

        pub(crate) fn path(&self) -> &Path {
            self.dir.path()
        }

        pub(crate) fn write(&self, name: &str, contents: &str) {
            std::fs::write(self.path().join(name), contents).unwrap();
        }

        pub(crate) fn commit(&self) {
            self.git(&["add", "--all"]);
            self.git(&["commit", "--quiet", "-m", "commit"]);
        }

        pub(crate) fn git(&self, args: &[&str]) -> String {
            let output = Command::new("git")
                .arg("-C")
                .arg(self.path())
                .args([
                    "-c",
                    "user.name=gdship",
                    "-c",
                    "user.email=gdship@example.com",
                ])
                .args(["-c", "tag.gpgSign=false", "-c", "commit.gpgSign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "git {args:?}: {output:?}");
            String::from_utf8(output.stdout).unwrap()
        }

        pub(crate) fn handle(&self) -> Git {
            Git::new(self.path())
        }
    }

    #[test]
    fn clean_tree_check_sees_changes_and_untracked_files() {
        let repo = Repo::new();
        repo.handle().check_clean().unwrap();

        repo.write("new.gd", "");
        let err = repo.handle().check_clean().unwrap_err().to_string();
        assert!(err.contains("?? new.gd"), "{err}");
        assert!(err.contains("--allow-dirty"), "{err}");

        repo.commit();
        repo.write("project.godot", "changed");
        let err = repo.handle().check_clean().unwrap_err().to_string();
        assert!(err.contains("M project.godot"), "{err}");

        std::fs::create_dir(repo.path().join(".gdship")).unwrap();
        repo.write(".gdship/build.txt", "");
        repo.git(&["checkout", "--", "project.godot"]);
        repo.handle().check_clean().unwrap();
    }

    #[test]
    fn long_dirty_lists_are_cut_short() {
        let repo = Repo::new();
        for n in 0..12 {
            repo.write(&format!("f{n:02}.gd"), "");
        }
        let err = repo.handle().check_clean().unwrap_err().to_string();
        assert!(err.contains("f09.gd"), "{err}");
        assert!(!err.contains("f10.gd"), "{err}");
        assert!(err.contains("... and 2 more"), "{err}");
    }

    #[test]
    fn state_dir_must_be_ignored() {
        let repo = Repo::new();
        repo.handle().check_state_dir_ignored().unwrap();

        repo.write(".gitignore", "*.tmp\n");
        let err = repo
            .handle()
            .check_state_dir_ignored()
            .unwrap_err()
            .to_string();
        assert!(err.contains("add `.gdship/` to .gitignore"), "{err}");

        let not_a_repo = tempfile::tempdir().unwrap();
        let err = Git::new(not_a_repo.path())
            .check_state_dir_ignored()
            .unwrap_err()
            .to_string();
        assert!(err.contains("`git check-ignore` failed"), "{err}");
    }

    #[test]
    fn head_version_describes_tags_and_commits() {
        let repo = Repo::new();
        let git = repo.handle();
        let hash = repo
            .git(&["rev-parse", "--short", "HEAD"])
            .trim()
            .to_owned();
        assert_eq!(
            git.head_version().unwrap(),
            HeadVersion::Untagged(hash.clone())
        );

        repo.git(&["tag", "-a", "v0.3.0", "-m", "v0.3.0"]);
        assert_eq!(
            git.head_version().unwrap(),
            HeadVersion::Tagged("0.3.0".into())
        );

        repo.write("more.gd", "");
        repo.commit();
        let hash = repo
            .git(&["rev-parse", "--short", "HEAD"])
            .trim()
            .to_owned();
        assert_eq!(
            git.head_version().unwrap(),
            HeadVersion::Tagged(format!("0.3.0-1-g{hash}"))
        );

        repo.write("project.godot", "changed");
        assert_eq!(
            git.head_version().unwrap(),
            HeadVersion::Tagged(format!("0.3.0-1-g{hash}-dirty"))
        );
    }

    #[test]
    fn head_version_of_an_empty_repository() {
        let dir = tempfile::tempdir().unwrap();
        let status = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["init", "--quiet"])
            .status()
            .unwrap();
        assert!(status.success());
        let git = Git::new(dir.path());
        assert!(git.is_repo().unwrap());
        assert_eq!(git.head_version().unwrap(), HeadVersion::NoCommits);
    }

    #[test]
    fn shallow_clones_are_detected() {
        let repo = Repo::new();
        repo.git(&["tag", "-a", "v1.0.0", "-m", "v1.0.0"]);
        repo.write("more.gd", "");
        repo.commit();
        assert!(!repo.handle().is_shallow().unwrap());

        let clone = tempfile::tempdir().unwrap();
        let url = format!(
            "file:///{}",
            repo.path()
                .display()
                .to_string()
                .replace('\\', "/")
                .trim_start_matches('/')
        );
        let status = Command::new("git")
            .args(["clone", "--quiet", "--depth", "1", &url])
            .arg(clone.path().join("c"))
            .status()
            .unwrap();
        assert!(status.success());
        let git = Git::new(&clone.path().join("c"));
        assert!(git.is_shallow().unwrap());
        assert!(matches!(
            git.head_version().unwrap(),
            HeadVersion::Untagged(_)
        ));
    }

    #[test]
    fn new_tags_must_not_exist() {
        let repo = Repo::new();
        let git = repo.handle();
        assert_eq!(git.new_tag_version("v2.0.0").unwrap(), "2.0.0");
        repo.git(&["tag", "v2.0.0"]);
        let err = git.new_tag_version("v2.0.0").unwrap_err();
        assert!(
            err.to_string().contains("tag v2.0.0 already exists"),
            "{err}"
        );
    }

    #[test]
    fn strips_one_v_before_a_digit() {
        assert_eq!(strip_v("v0.3.0"), "0.3.0");
        assert_eq!(strip_v("0.3.0"), "0.3.0");
        assert_eq!(strip_v("version2"), "version2");
        assert_eq!(strip_v("vv1"), "vv1");
        assert_eq!(strip_v("v"), "v");
    }
}
