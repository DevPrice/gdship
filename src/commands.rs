use std::ffi::OsString;

use anyhow::{Context, Result};

use crate::UsageError;
use crate::cli::ExportArgs;
use crate::config::UserConfig;
use crate::export::ExportPlan;
use crate::godot::Godot;
use crate::godot_project::{
    EXPORT_PRESETS_FILE, ProjectInfo, Target, load_presets, resolve_targets,
};
use crate::project::{GODOT_PROJECT_FILE, Project};
use crate::report::Reporter;

fn env(key: &str) -> Option<OsString> {
    std::env::var_os(key)
}

/// Steps 1 to 3: finds the project, reads it, picks the channels and checks Godot.
pub(crate) fn prepare(
    args: &ExportArgs,
    require_config: bool,
    reporter: Reporter,
) -> Result<ExportPlan> {
    let cwd = std::env::current_dir().context("cannot read the current directory")?;
    let project = Project::discover(&cwd)?;
    let config = if require_config {
        Some(project.require_config()?)
    } else {
        project.config()?
    };
    let info = ProjectInfo::load(&project.root().join(GODOT_PROJECT_FILE))?;
    let presets = load_presets(&project.root().join(EXPORT_PRESETS_FILE))?;
    let (targets, skipped) =
        resolve_targets(config.as_ref().and_then(|c| c.channels.as_ref()), &presets)?;
    for preset in skipped {
        reporter.action(
            "Skipping",
            format!(
                "preset \"{}\": no channel for platform {}; name one in [channels] to export it",
                preset.name, preset.platform
            ),
        );
    }
    let targets = select(targets, &args.only)?;
    let user_config = UserConfig::from_env()?;
    let godot = Godot::resolve(args.godot.as_deref(), &user_config, &env)?;
    godot.check_project(&info)?;
    godot.check_templates(&targets, &env)?;
    Ok(ExportPlan::new(
        &project,
        info.name.as_deref(),
        &godot,
        &targets,
    ))
}

/// Keeps the targets `--only` names, in their configured order.
fn select(targets: Vec<Target>, only: &[String]) -> Result<Vec<Target>> {
    if only.is_empty() {
        return Ok(targets);
    }
    for name in only {
        if !targets.iter().any(|t| t.channel.as_str() == name) {
            let known: Vec<_> = targets.iter().map(|t| t.channel.as_str()).collect();
            return Err(UsageError(format!(
                "--only {name}: no such channel; channels: {}",
                known.join(", ")
            ))
            .into());
        }
    }
    Ok(targets
        .into_iter()
        .filter(|t| only.iter().any(|name| name == t.channel.as_str()))
        .collect())
}

pub(crate) fn export(args: &ExportArgs, reporter: Reporter) -> Result<()> {
    let plan = prepare(args, false, reporter)?;
    plan.run(args.verbose, reporter)?;
    for export in &plan.exports {
        reporter.action(
            "Exported",
            format!("{} to {}", export.channel, export.output.display()),
        );
    }
    Ok(())
}
