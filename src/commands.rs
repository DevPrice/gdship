use std::ffi::OsString;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

use crate::addons::check_addons;
use crate::butler::find_butler;
use crate::cli::ExportArgs;
use crate::config::{Channel, ItchTarget, ProjectConfig, UserConfig, user_config_path};
use crate::export::ExportPlan;
use crate::git::{Git, VersionSource, resolve_version};
use crate::godot::{Godot, NOT_FOUND};
use crate::godot_project::{
    EXPORT_PRESETS_FILE, ProjectInfo, Target, load_presets, resolve_targets,
};
use crate::process::{ToolCommand, describe_exit};
use crate::project::{GODOT_PROJECT_FILE, Project};
use crate::prompt;
use crate::report::Reporter;
use crate::{Outcome, UsageError};

fn env(key: &str) -> Option<OsString> {
    std::env::var_os(key)
}

fn discover() -> Result<Project> {
    let cwd = std::env::current_dir().context("cannot read the current directory")?;
    Project::discover(&cwd)
}

/// The configured channels and their presets, noting presets that were skipped.
fn load_targets(
    project: &Project,
    config: Option<&ProjectConfig>,
    reporter: Reporter,
) -> Result<Vec<Target>> {
    let presets = load_presets(&project.root().join(EXPORT_PRESETS_FILE))?;
    let (targets, skipped) = resolve_targets(config.and_then(|c| c.channels.as_ref()), &presets)?;
    for preset in skipped {
        reporter.action(
            "Skipping",
            format!(
                "preset \"{}\": no channel for platform {}; name one in [channels] to export it",
                preset.name, preset.platform
            ),
        );
    }
    Ok(targets)
}

/// Everything resolved and checked before Godot runs.
struct Prepared {
    project: Project,
    config: Option<ProjectConfig>,
    plan: ExportPlan,
}

/// Steps 1 to 3: finds the project, reads it, picks the channels and checks Godot.
fn prepare(args: &ExportArgs, require_config: bool, reporter: Reporter) -> Result<Prepared> {
    let project = discover()?;
    let config = if require_config {
        Some(project.require_config()?)
    } else {
        project.config()?
    };
    let info = ProjectInfo::load(&project.root().join(GODOT_PROJECT_FILE))?;
    let targets = select(
        load_targets(&project, config.as_ref(), reporter)?,
        &args.only,
    )?;
    let user_config = UserConfig::from_env()?;
    let godot = find_godot(args.godot.as_deref(), &user_config)?;
    godot.check_project(&info)?;
    godot.check_templates(&targets, &env)?;
    let plan = ExportPlan::new(&project, info.name.as_deref(), &godot, &targets);
    Ok(Prepared {
        project,
        config,
        plan,
    })
}

/// Finds Godot, asking for it on a terminal when nothing names one.
fn find_godot(flag: Option<&Path>, user_config: &UserConfig) -> Result<Godot> {
    match Godot::find(flag, user_config, &env)? {
        Some(path) => Godot::at(&path),
        None if prompt::is_interactive() => prompt::ask_for_godot(
            &mut std::io::stdin().lock(),
            &mut std::io::stderr(),
            user_config_path(env).as_deref(),
        ),
        None => bail!("{NOT_FOUND}"),
    }
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

pub(crate) fn export(args: &ExportArgs, reporter: Reporter) -> Result<Outcome> {
    let Prepared { project, plan, .. } = prepare(args, false, reporter)?;
    check_addons(project.root(), &env, reporter)?;
    plan.run(args.verbose, reporter)?;
    for export in &plan.exports {
        reporter.action(
            "Exported",
            format!("{} to {}", export.channel, export.output.display()),
        );
    }
    Ok(Outcome::Success)
}

pub(crate) struct PushOptions<'a> {
    pub(crate) export: &'a ExportArgs,
    pub(crate) version: Option<&'a str>,
    pub(crate) tag: Option<&'a str>,
    pub(crate) allow_dirty: bool,
    pub(crate) dry_run: bool,
}

/// One butler push, planned.
struct Push {
    channel: Channel,
    target: String,
    command: ToolCommand,
}

fn butler_target(itch: &ItchTarget, channel: &Channel) -> String {
    format!("{itch}:{channel}")
}

fn push_command(butler: &Path, build_dir: &Path, target: &str, version: &str) -> ToolCommand {
    ToolCommand::new(butler)
        .arg("push")
        .arg(build_dir)
        .arg(target)
        .arg("--userversion")
        .arg(version)
        .arg("--if-changed")
}

/// Exports every channel, then pushes each build, so a failed export never leaves itch
/// half-updated.
pub(crate) fn push(options: &PushOptions<'_>, reporter: Reporter) -> Result<Outcome> {
    let Prepared {
        project,
        config,
        plan,
    } = prepare(options.export, true, reporter)?;
    let itch = config.expect("push requires gdship.toml").itch;

    let git = Git::new(project.root());
    if !options.allow_dirty {
        git.check_clean()?;
    }
    git.check_state_dir_ignored()?;
    let source = match (options.version, options.tag) {
        (Some(version), _) => VersionSource::Explicit(version),
        (None, Some(tag)) => VersionSource::NewTag(tag),
        (None, None) => VersionSource::Head {
            allow_dirty: options.allow_dirty,
        },
    };
    let version = resolve_version(&git, source)?;
    let butler = find_butler(&env, reporter)?;
    check_addons(project.root(), &env, reporter)?;

    let pushes: Vec<Push> = plan
        .exports
        .iter()
        .map(|export| {
            let target = butler_target(&itch, &export.channel);
            Push {
                channel: export.channel.clone(),
                command: push_command(&butler, &export.build_dir, &target, &version),
                target,
            }
        })
        .collect();

    if options.dry_run {
        dry_run(&plan, &pushes, options.tag, &version, reporter);
        return Ok(Outcome::Success);
    }

    if let Some(tag) = options.tag {
        reporter.action("Tagging", format!("HEAD as {tag}"));
        git.create_tag(tag)?;
    }
    if let Err(err) = plan.run(options.export.verbose, reporter) {
        return Err(roll_back_tag(&git, options.tag, err, reporter));
    }

    let mut pushed: Vec<&Push> = Vec::new();
    for push in &pushes {
        reporter.action("Pushing", format!("{} {version}", push.channel));
        let failure = match push.command.run_attached() {
            Ok(status) if status.success() => None,
            Ok(status) => Some(anyhow!("butler failed with {}", describe_exit(status))),
            Err(err) => Some(err),
        };
        if let Some(failure) = failure {
            let err = push_failure(&pushes, &pushed, push, &failure, options.tag);
            // Nothing reached itch.io under the tag, so it goes too.
            if pushed.is_empty() {
                return Err(roll_back_tag(&git, options.tag, err, reporter));
            }
            return Err(err);
        }
        pushed.push(push);
    }

    for push in &pushed {
        reporter.action(
            "Pushed",
            format!("{} {version} to {}", push.channel, push.target),
        );
    }
    for push in &pushed {
        let status = ToolCommand::new(&butler)
            .arg("status")
            .arg(&push.target)
            .run_attached();
        if !status.as_ref().is_ok_and(|s| s.success()) {
            reporter.warn(format!("`butler status {}` failed", push.target));
        }
    }

    if let Some(tag) = options.tag {
        reporter.action("Pushing", format!("tag {tag} to origin"));
        if let Err(err) = git.push_tag(tag) {
            reporter.warn(format!(
                "every channel was pushed to itch.io, but pushing tag {tag} to origin failed: \
                 {err:#}\nRetry with `git push origin {tag}`"
            ));
            return Ok(Outcome::Failure);
        }
    }
    Ok(Outcome::Success)
}

fn dry_run(
    plan: &ExportPlan,
    pushes: &[Push],
    tag: Option<&str>,
    version: &str,
    reporter: Reporter,
) {
    if let Some(tag) = tag {
        reporter.action("Would run", format!("git tag -a {tag} -m {tag}"));
    }
    for command in plan.commands() {
        reporter.action("Would run", command);
    }
    for push in pushes {
        reporter.action("Would run", &push.command);
    }
    if let Some(tag) = tag {
        reporter.action("Would run", format!("git push origin refs/tags/{tag}"));
    }
    reporter.action("Dry run", format!("of version {version}; nothing was run"));
}

/// Deletes the tag gdship created, when nothing reached itch.io under it.
fn roll_back_tag(
    git: &Git,
    tag: Option<&str>,
    err: anyhow::Error,
    reporter: Reporter,
) -> anyhow::Error {
    let Some(tag) = tag else { return err };
    match git.delete_tag(tag) {
        Ok(()) => reporter.action("Deleted", format!("tag {tag}, since nothing was pushed")),
        Err(delete) => reporter.warn(format!(
            "could not delete tag {tag}: {delete:#}\nDelete it with `git tag -d {tag}`"
        )),
    }
    err
}

/// Says which channels made it to itch.io and how to push the rest.
fn push_failure(
    all: &[Push],
    pushed: &[&Push],
    failed: &Push,
    failure: &anyhow::Error,
    tag: Option<&str>,
) -> anyhow::Error {
    let list = |channels: Vec<&str>| {
        if channels.is_empty() {
            "none".to_owned()
        } else {
            channels.join(", ")
        }
    };
    let not_pushed: Vec<&str> = all
        .iter()
        .map(|p| p.channel.as_str())
        .filter(|c| !pushed.iter().any(|p| p.channel.as_str() == *c))
        .collect();
    let mut message = format!(
        "pushing {} failed: {failure:#}\n  pushed:     {}\n  not pushed: {}",
        failed.target,
        list(pushed.iter().map(|p| p.channel.as_str()).collect()),
        list(not_pushed.clone()),
    );
    if !pushed.is_empty() {
        let only: Vec<_> = not_pushed.iter().map(|c| format!("--only {c}")).collect();
        message.push_str(&format!(
            "\nRetry the rest with `gdship push {}`",
            only.join(" ")
        ));
        if let Some(tag) = tag {
            message.push_str(&format!(
                "; tag {tag} stays on HEAD, so the retry uses the same version. The tag \
                 was not pushed to origin."
            ));
        }
    }
    anyhow!(message)
}

/// Writes `gdship.toml` from the answers to its questions. The questions are read from
/// stdin even when it isn't a terminal, so the answers can be piped in.
pub(crate) fn init(reporter: Reporter) -> Result<Outcome> {
    let project = discover()?;
    let path = project.config_path();
    if path.exists() {
        bail!("{} already exists", path.display());
    }
    let itch = prompt::ask_for_itch(&mut std::io::stdin().lock(), &mut std::io::stderr())?;
    // create_new, in case the file appeared while gdship was waiting for an answer.
    let mut file = std::fs::File::create_new(&path)
        .with_context(|| format!("cannot create {}", path.display()))?;
    writeln!(file, "itch = \"{itch}\"")
        .with_context(|| format!("cannot write {}", path.display()))?;
    reporter.action("Created", path.display());
    if project.ignore_state_dir()? {
        reporter.action("Updated", ".gitignore to ignore /.gdship/");
    }
    Ok(Outcome::Success)
}

pub(crate) fn login(reporter: Reporter) -> Result<Outcome> {
    let butler = find_butler(&env, reporter)?;
    let status = ToolCommand::new(&butler).arg("login").run_attached()?;
    if !status.success() {
        bail!("`butler login` failed with {}", describe_exit(status));
    }
    Ok(Outcome::Success)
}

pub(crate) fn status(reporter: Reporter) -> Result<Outcome> {
    let project = discover()?;
    let config = project.require_config()?;
    let targets = load_targets(&project, Some(&config), reporter)?;
    let butler = find_butler(&env, reporter)?;
    let mut outcome = Outcome::Success;
    for target in &targets {
        let target = butler_target(&config.itch, &target.channel);
        let status = ToolCommand::new(&butler)
            .arg("status")
            .arg(&target)
            .run_attached()?;
        if !status.success() {
            reporter.warn(format!(
                "`butler status {target}` failed with {}",
                describe_exit(status)
            ));
            outcome = Outcome::Failure;
        }
    }
    Ok(outcome)
}
