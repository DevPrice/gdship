use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

use crate::config::{Channel, ChannelMap};
use crate::configfile::{self, Section, parse_packed_strings};

pub(crate) const EXPORT_PRESETS_FILE: &str = "export_presets.cfg";

/// What gdship reads from `project.godot`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectInfo {
    pub(crate) name: Option<String>,
    /// The Godot `major.minor` in `config/features`, e.g. (4, 7).
    pub(crate) godot_version: (u32, u32),
}

impl ProjectInfo {
    pub(crate) fn parse(text: &str) -> Result<Self> {
        let sections = configfile::parse(text)?;
        let application = sections.iter().find(|s| s.name == "application");
        let name = match application {
            Some(section) => section.string("config/name")?.filter(|n| !n.is_empty()),
            None => None,
        };
        let features = application
            .and_then(|s| s.get("config/features"))
            .ok_or_else(|| {
                anyhow!(
                    "no config/features in [application]; open and save the project in \
                     Godot 4 to add it"
                )
            })?;
        let features = parse_packed_strings(features)
            .ok_or_else(|| anyhow!("config/features is not a PackedStringArray: {features}"))?;
        let godot_version = features
            .iter()
            .find_map(|f| parse_major_minor(f))
            .ok_or_else(|| {
                anyhow!("config/features names no Godot version, such as \"4.7\": {features:?}")
            })?;
        Ok(Self {
            name,
            godot_version,
        })
    }

    pub(crate) fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("invalid {}", path.display()))
    }
}

fn parse_major_minor(feature: &str) -> Option<(u32, u32)> {
    let (major, minor) = feature.split_once('.')?;
    let is_number = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !is_number(major) || !is_number(minor) {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// An export preset from `export_presets.cfg`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Preset {
    pub(crate) name: String,
    pub(crate) platform: String,
    pub(crate) export_path: String,
    /// `custom_template/release`, when set to a non-empty path.
    pub(crate) custom_release_template: Option<String>,
}

impl Preset {
    pub(crate) fn is_web(&self) -> bool {
        self.platform == "Web"
    }
}

/// Reads every `[preset.N]` in index order.
pub(crate) fn parse_presets(text: &str) -> Result<Vec<Preset>> {
    let sections = configfile::parse(text)?;
    let mut indexed = Vec::new();
    for section in &sections {
        let Some(index) = section
            .name
            .strip_prefix("preset.")
            .and_then(|i| i.parse::<u32>().ok())
        else {
            continue;
        };
        let required = |key: &str| -> Result<String> {
            section
                .string(key)?
                .ok_or_else(|| anyhow!("[{}] has no `{key}`", section.name))
        };
        let options_name = format!("preset.{index}.options");
        let options = sections.iter().find(|s| s.name == options_name);
        let custom_release_template = options
            .map(|o: &Section| o.string("custom_template/release"))
            .transpose()?
            .flatten()
            .filter(|p| !p.is_empty());
        indexed.push((
            index,
            Preset {
                name: required("name")?,
                platform: required("platform")?,
                export_path: section.string("export_path")?.unwrap_or_default(),
                custom_release_template,
            },
        ));
    }
    indexed.sort_by_key(|(index, _)| *index);
    Ok(indexed.into_iter().map(|(_, preset)| preset).collect())
}

pub(crate) fn load_presets(path: &Path) -> Result<Vec<Preset>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!(
            "no {EXPORT_PRESETS_FILE} in the project; add export presets in Godot's \
             Project > Export dialog"
        ),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    parse_presets(&text).with_context(|| format!("invalid {}", path.display()))
}

/// The itch channel each Godot platform gets when channels are derived.
const PLATFORM_CHANNELS: &[(&str, &str)] = &[
    ("Web", "html5"),
    ("Windows Desktop", "windows"),
    ("Linux", "linux"),
    ("macOS", "mac"),
    ("Android", "android"),
];

/// A channel and the preset that builds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub(crate) channel: Channel,
    pub(crate) preset: Preset,
}

/// The channels to build: the `[channels]` table when there is one, else one channel per
/// preset whose platform itch has a convention for. Returns the targets and the names of
/// presets skipped because their platform has no channel.
pub(crate) fn resolve_targets<'a>(
    channels: Option<&ChannelMap>,
    presets: &'a [Preset],
) -> Result<(Vec<Target>, Vec<&'a Preset>)> {
    if let Some(ChannelMap(channels)) = channels {
        let targets = channels
            .iter()
            .map(|(channel, preset_name)| {
                let mut matching = presets.iter().filter(|p| &p.name == preset_name);
                let preset = matching.next().ok_or_else(|| {
                    anyhow!(
                        "channel `{channel}` names preset \"{preset_name}\", which is not in \
                         {EXPORT_PRESETS_FILE}; presets: {}",
                        preset_names(presets)
                    )
                })?;
                if matching.next().is_some() {
                    bail!("{EXPORT_PRESETS_FILE} has more than one preset named \"{preset_name}\"");
                }
                Ok(Target {
                    channel: channel.clone(),
                    preset: preset.clone(),
                })
            })
            .collect::<Result<_>>()?;
        return Ok((targets, Vec::new()));
    }

    let mut targets: Vec<Target> = Vec::new();
    let mut skipped = Vec::new();
    for preset in presets {
        let Some((_, channel)) = PLATFORM_CHANNELS
            .iter()
            .find(|(platform, _)| *platform == preset.platform)
        else {
            skipped.push(preset);
            continue;
        };
        if let Some(other) = targets
            .iter()
            .find(|t| t.preset.platform == preset.platform)
        {
            bail!(
                "presets \"{}\" and \"{}\" are both {}; add a [channels] table to gdship.toml \
                 to say which channel each one builds",
                other.preset.name,
                preset.name,
                preset.platform
            );
        }
        targets.push(Target {
            channel: channel.parse().expect("table channels are valid"),
            preset: preset.clone(),
        });
    }
    if targets.is_empty() {
        bail!(
            "no export preset has a platform gdship can map to a channel ({}); add a \
             [channels] table to gdship.toml",
            PLATFORM_CHANNELS
                .iter()
                .map(|(p, _)| *p)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok((targets, skipped))
}

fn preset_names(presets: &[Preset]) -> String {
    if presets.is_empty() {
        return "none".to_owned();
    }
    presets
        .iter()
        .map(|p| format!("\"{}\"", p.name))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/export_presets.cfg");

    fn preset(name: &str, platform: &str) -> String {
        format!(
            "[preset.{n}]\n\nname=\"{name}\"\nplatform=\"{platform}\"\nexport_path=\"\"\n\n[preset.{n}.options]\n\ncustom_template/release=\"\"\n",
            n = name.len()
        )
    }

    fn channels(targets: &[Target]) -> Vec<(&str, &str)> {
        targets
            .iter()
            .map(|t| (t.channel.as_str(), t.preset.name.as_str()))
            .collect()
    }

    #[test]
    fn reads_a_real_godot_4_7_file() {
        let presets = parse_presets(FIXTURE).unwrap();
        assert_eq!(
            presets,
            [
                Preset {
                    name: "Web".into(),
                    platform: "Web".into(),
                    export_path: "exports/web/index.html".into(),
                    custom_release_template: None,
                },
                Preset {
                    name: "Windows Desktop".into(),
                    platform: "Windows Desktop".into(),
                    export_path: "exports/windows/idlefactory.exe".into(),
                    custom_release_template: None,
                },
            ]
        );
        let (targets, skipped) = resolve_targets(None, &presets).unwrap();
        assert_eq!(
            channels(&targets),
            [("html5", "Web"), ("windows", "Windows Desktop")]
        );
        assert!(skipped.is_empty());

        let sections = configfile::parse(FIXTURE).unwrap();
        let options = sections
            .iter()
            .find(|s| s.name == "preset.0.options")
            .unwrap();
        let head = options.string("html/head_include").unwrap().unwrap();
        assert!(head.contains("section: \"[preset.9]\",\n"), "{head}");
        assert!(head.contains("line: \"name=\\\"Fake\\\"\""), "{head}");
        assert_eq!(options.get("html/canvas_resize_policy"), Some("2"));
    }

    #[test]
    fn reads_custom_release_templates() {
        let text = "[preset.0]\nname=\"Linux\"\nplatform=\"Linux\"\n[preset.0.options]\ncustom_template/debug=\"\"\ncustom_template/release=\"/t/linux.x86_64\"\n";
        let presets = parse_presets(text).unwrap();
        assert_eq!(
            presets[0].custom_release_template.as_deref(),
            Some("/t/linux.x86_64")
        );
        assert_eq!(presets[0].export_path, "");
    }

    #[test]
    fn presets_are_ordered_by_index() {
        let text = "[preset.1]\nname=\"B\"\nplatform=\"Linux\"\n[preset.0]\nname=\"A\"\nplatform=\"Web\"\n";
        let names: Vec<_> = parse_presets(text)
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["A", "B"]);
    }

    #[test]
    fn project_info_reads_name_and_version() {
        let info = ProjectInfo::parse(
            "; Engine configuration file.\nconfig_version=5\n\n[application]\n\nconfig/name=\"Idle Factory\"\nconfig/features=PackedStringArray(\"4.7\", \"GL Compatibility\")\n",
        )
        .unwrap();
        assert_eq!(info.name.as_deref(), Some("Idle Factory"));
        assert_eq!(info.godot_version, (4, 7));
    }

    #[test]
    fn missing_features_is_an_error() {
        let err = ProjectInfo::parse("[application]\nconfig/name=\"x\"\n").unwrap_err();
        assert!(err.to_string().contains("no config/features"), "{err}");
        let err = ProjectInfo::parse("[application]\nconfig/features=PackedStringArray(\"C#\")\n")
            .unwrap_err();
        assert!(err.to_string().contains("names no Godot version"), "{err}");
    }

    #[test]
    fn duplicate_platforms_need_a_channels_table() {
        let text = [preset("Web", "Web"), preset("Web Threads", "Web")].concat();
        let presets = parse_presets(&text).unwrap();
        let err = resolve_targets(None, &presets).unwrap_err().to_string();
        assert!(
            err.contains("\"Web\" and \"Web Threads\" are both Web"),
            "{err}"
        );
        assert!(err.contains("[channels]"), "{err}");

        let map = ChannelMap(vec![
            ("html5".parse().unwrap(), "Web Threads".into()),
            ("html5-legacy".parse().unwrap(), "Web".into()),
        ]);
        let (targets, _) = resolve_targets(Some(&map), &presets).unwrap();
        assert_eq!(
            channels(&targets),
            [("html5", "Web Threads"), ("html5-legacy", "Web")]
        );
    }

    #[test]
    fn unmapped_platforms_are_skipped() {
        let text = [preset("iOS", "iOS"), preset("Mac", "macOS")].concat();
        let presets = parse_presets(&text).unwrap();
        let (targets, skipped) = resolve_targets(None, &presets).unwrap();
        assert_eq!(channels(&targets), [("mac", "Mac")]);
        assert_eq!(skipped[0].name, "iOS");

        let presets = parse_presets(&preset("iOS", "iOS")).unwrap();
        let err = resolve_targets(None, &presets).unwrap_err().to_string();
        assert!(err.contains("no export preset has a platform"), "{err}");
    }

    #[test]
    fn channels_table_must_name_existing_presets() {
        let presets = parse_presets(&preset("Web", "Web")).unwrap();
        let map = ChannelMap(vec![("windows".parse().unwrap(), "Windows".into())]);
        let err = resolve_targets(Some(&map), &presets)
            .unwrap_err()
            .to_string();
        assert!(err.contains("preset \"Windows\", which is not in"), "{err}");
        assert!(err.contains("presets: \"Web\""), "{err}");
    }
}
