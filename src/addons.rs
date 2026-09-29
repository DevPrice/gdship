use std::ffi::OsString;
use std::path::Path;

use anyhow::{Result, bail};

use crate::exe::find_on_path;
use crate::process::{ToolCommand, describe_exit};
use crate::report::Reporter;

const MANIFEST_FILE: &str = "addons.toml";

/// Checks that `addons/` matches gdget's `addons.toml` before exporting, so a build never
/// ships addons other than the pinned ones. Warns and carries on if gdget isn't on PATH.
pub(crate) fn check_addons(
    root: &Path,
    env: &dyn Fn(&str) -> Option<OsString>,
    reporter: Reporter,
) -> Result<()> {
    if !root.join(MANIFEST_FILE).is_file() {
        return Ok(());
    }
    let Some(gdget) = find_on_path("gdget", env) else {
        reporter.warn(format!(
            "{MANIFEST_FILE} exists but gdget is not on PATH, so addons/ was not checked \
             against it"
        ));
        return Ok(());
    };
    reporter.action("Checking", "addons with gdget");
    let status = ToolCommand::new(&gdget)
        .arg("-C")
        .arg(root)
        .arg("sync")
        .arg("--check")
        .run_attached()?;
    match status.code() {
        Some(0) => Ok(()),
        // gdget exits 1 for drift and for its own errors alike; its output says which.
        Some(1) => bail!(
            "`gdget sync --check` failed, see its output above; if addons/ has drifted from \
             {MANIFEST_FILE}, run `gdget sync` and commit the result"
        ),
        _ => bail!("`gdget sync --check` failed with {}", describe_exit(status)),
    }
}
