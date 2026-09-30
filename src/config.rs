use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

pub(crate) const PROJECT_CONFIG_FILE: &str = "gdship.toml";

const NEWER_VERSION_HINT: &str = "it may need a newer version of gdship";

/// An itch.io game as `<user>/<game>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ItchTarget {
    user: String,
    game: String,
}

impl FromStr for ItchTarget {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let is_segment = |p: &str| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        };
        match s.split_once('/') {
            Some((user, game)) if is_segment(user) && is_segment(game) => Ok(Self {
                user: user.to_owned(),
                game: game.to_owned(),
            }),
            _ => bail!(
                "`{s}` is not an itch.io game; expected `<user>/<game>`, e.g. `username/my-game`"
            ),
        }
    }
}

impl fmt::Display for ItchTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.user, self.game)
    }
}

/// An itch.io channel name. itch tags a channel with a platform when its name contains
/// `windows`, `linux`, `mac` or `android`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Channel(String);

impl Channel {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Channel {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.is_empty()
            || !s
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            bail!("`{s}` is not a valid channel name; use lowercase letters, digits and `-`");
        }
        Ok(Self(s.to_owned()))
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `gdship.toml`, next to `project.godot`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectConfig {
    #[serde(deserialize_with = "from_str")]
    pub(crate) itch: ItchTarget,
    /// Channel to export preset name, in file order. `None` means derive them from the
    /// presets.
    #[serde(default)]
    pub(crate) channels: Option<ChannelMap>,
}

/// The `[channels]` table, in the order the file lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelMap(pub Vec<(Channel, String)>);

impl ProjectConfig {
    pub(crate) fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text).map_err(with_newer_version_hint)?;
        if config.channels.as_ref().is_some_and(|c| c.0.is_empty()) {
            bail!("[channels] lists no channels; remove it to derive them from the presets");
        }
        Ok(config)
    }

    pub(crate) fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("invalid {}", path.display()))
    }
}

/// The per-user `config.toml`, for paths that differ between machines.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UserConfig {
    pub(crate) godot: Option<PathBuf>,
}

impl UserConfig {
    pub(crate) fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).map_err(with_newer_version_hint)
    }

    /// Reads the file at `path`; a missing file is an empty config.
    pub(crate) fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).with_context(|| format!("invalid {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    pub(crate) fn from_env() -> Result<Self> {
        match user_config_path(|key| std::env::var_os(key)) {
            Some(path) => Self::load(&path),
            None => Ok(Self::default()),
        }
    }

    /// Records `godot` in the user config at `path`, keeping whatever the file already
    /// holds. Callers only do this when the file sets no `godot` yet.
    pub(crate) fn save_godot(path: &Path, godot: &Path) -> Result<()> {
        let mut text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
        };
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        let value = godot
            .to_str()
            .ok_or_else(|| anyhow!("{} is not valid UTF-8", godot.display()))?;
        text.push_str(&format!("godot = {}\n", toml_string(value)));
        let saved =
            Self::parse(&text).with_context(|| format!("cannot update {}", path.display()))?;
        if saved.godot.as_deref() != Some(godot) {
            bail!(
                "cannot update {}: it would not read back as {}",
                path.display(),
                godot.display()
            );
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))
    }
}

/// A TOML string for `value`: a literal string when it can be one, so Windows paths keep
/// their backslashes readable, otherwise an escaped basic string.
fn toml_string(value: &str) -> String {
    if !value.contains('\'') && !value.chars().any(char::is_control) {
        return format!("'{value}'");
    }
    let mut quoted = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            c if c.is_control() => quoted.push_str(&format!("\\u{:04X}", c as u32)),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

/// Where the user config lives, or `None` when the environment names no candidate.
pub(crate) fn user_config_path(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let var = |key: &str| env(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(path) = var("GDSHIP_CONFIG") {
        return Some(path);
    }
    let dir = if let Some(dir) = var("APPDATA").filter(|_| cfg!(windows)) {
        dir
    } else if let Some(dir) = var("XDG_CONFIG_HOME") {
        dir
    } else {
        let home = home_dir(&env)?;
        if cfg!(target_os = "macos") {
            home.join("Library").join("Application Support")
        } else {
            home.join(".config")
        }
    };
    Some(dir.join("gdship").join("config.toml"))
}

/// The user's home directory: `HOME`, or `USERPROFILE` on Windows.
pub(crate) fn home_dir(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let var = |key: &str| env(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    var("HOME").or_else(|| {
        if cfg!(windows) {
            var("USERPROFILE")
        } else {
            None
        }
    })
}

fn with_newer_version_hint(err: toml::de::Error) -> anyhow::Error {
    let message = err.to_string();
    if message.contains("unknown field") {
        anyhow!("{}\n{NEWER_VERSION_HINT}", message.trim_end())
    } else {
        anyhow!("{}", message.trim_end())
    }
}

fn from_str<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr<Err = anyhow::Error>,
{
    let text = String::deserialize(deserializer)?;
    text.parse()
        .map_err(|e| de::Error::custom(format!("{e:#}")))
}

impl<'de> Deserialize<'de> for ChannelMap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ChannelsVisitor;

        impl<'de> Visitor<'de> for ChannelsVisitor {
            type Value = ChannelMap;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a table of channel names to export preset names")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ChannelMap, A::Error> {
                let mut channels = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    let channel = key
                        .parse()
                        .map_err(|e| de::Error::custom(format!("{e:#}")))?;
                    let preset: String = map.next_value()?;
                    if preset.is_empty() {
                        return Err(de::Error::custom(format!(
                            "channel `{key}` names an empty preset"
                        )));
                    }
                    channels.push((channel, preset));
                }
                Ok(ChannelMap(channels))
            }
        }

        deserializer.deserialize_map(ChannelsVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(result: Result<impl fmt::Debug>) -> String {
        format!("{:#}", result.unwrap_err())
    }

    #[test]
    fn parses_target_and_channels_in_order() {
        let config = ProjectConfig::parse(
            "itch = \"username/my-game\"\n\n[channels]\nwindows = \"Windows Desktop\"\nhtml5 = \"Web\"\n",
        )
        .unwrap();
        assert_eq!(config.itch.to_string(), "username/my-game");
        let channels: Vec<_> = config
            .channels
            .unwrap()
            .0
            .into_iter()
            .map(|(c, p)| (c.to_string(), p))
            .collect();
        assert_eq!(
            channels,
            [
                ("windows".to_owned(), "Windows Desktop".to_owned()),
                ("html5".to_owned(), "Web".to_owned()),
            ]
        );

        let config = ProjectConfig::parse("itch = \"a_b/c-1\"").unwrap();
        assert_eq!(config.channels, None);
    }

    #[test]
    fn itch_is_required() {
        let err = error(ProjectConfig::parse("[channels]\nhtml5 = \"Web\""));
        assert!(err.contains("missing field `itch`"), "{err}");
    }

    #[test]
    fn rejects_malformed_targets() {
        for target in [
            "username",
            "username/",
            "/game",
            "a/b/c",
            "dev price/g",
            "a/b.c",
        ] {
            let err = error(ProjectConfig::parse(&format!("itch = \"{target}\"")));
            assert!(err.contains("expected `<user>/<game>`"), "{target}: {err}");
        }
    }

    #[test]
    fn rejects_bad_channel_names() {
        for channel in ["HTML5", "win_64", "\"a b\"", "\"\""] {
            let err = error(ProjectConfig::parse(&format!(
                "itch = \"a/b\"\n[channels]\n{channel} = \"Web\""
            )));
            assert!(err.contains("not a valid channel name"), "{channel}: {err}");
        }
        let err = error(ProjectConfig::parse("itch = \"a/b\"\n[channels]\n"));
        assert!(err.contains("lists no channels"), "{err}");
        let err = error(ProjectConfig::parse(
            "itch = \"a/b\"\n[channels]\nhtml5 = \"\"",
        ));
        assert!(err.contains("empty preset"), "{err}");
    }

    #[test]
    fn unknown_keys_suggest_a_newer_gdship() {
        let err = error(ProjectConfig::parse("itch = \"a/b\"\nbuild_dir = \"out\""));
        assert!(err.contains("unknown field `build_dir`"), "{err}");
        assert!(err.contains("newer version of gdship"), "{err}");

        let err = error(UserConfig::parse("butler = \"/bin/butler\""));
        assert!(err.contains("unknown field `butler`"), "{err}");
        assert!(err.contains("newer version of gdship"), "{err}");
    }

    #[test]
    fn saving_godot_creates_or_extends_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("gdship").join("config.toml");
        let godot = PathBuf::from(r"C:\Apps\Godot 4.7\Godot_v4.7.2-stable_win64_console.exe");
        UserConfig::save_godot(&path, &godot).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "godot = 'C:\\Apps\\Godot 4.7\\Godot_v4.7.2-stable_win64_console.exe'\n"
        );
        assert_eq!(UserConfig::load(&path).unwrap().godot, Some(godot));

        std::fs::write(&path, "# my settings").unwrap();
        let odd = PathBuf::from("/opt/it's \"godot\"\\bin");
        UserConfig::save_godot(&path, &odd).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# my settings\ngodot = \""), "{text}");
        assert_eq!(UserConfig::load(&path).unwrap().godot, Some(odd));
    }

    #[test]
    fn user_config_is_optional() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        assert_eq!(UserConfig::load(&path).unwrap(), UserConfig::default());
        std::fs::write(&path, "godot = \"C:/Tools/Godot.exe\"").unwrap();
        assert_eq!(
            UserConfig::load(&path).unwrap().godot,
            Some(PathBuf::from("C:/Tools/Godot.exe"))
        );
    }

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |key| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn explicit_config_path_wins() {
        let vars = [
            ("GDSHIP_CONFIG", "/x/gdship.toml"),
            ("APPDATA", "A"),
            ("HOME", "/h"),
        ];
        assert_eq!(
            user_config_path(env(&vars)),
            Some(PathBuf::from("/x/gdship.toml"))
        );
    }

    #[test]
    fn platform_config_path() {
        let vars = [
            ("GDSHIP_CONFIG", ""),
            ("APPDATA", "A"),
            ("XDG_CONFIG_HOME", "/xdg"),
            ("HOME", "/h"),
        ];
        let expected = if cfg!(windows) {
            PathBuf::from("A")
        } else {
            PathBuf::from("/xdg")
        };
        assert_eq!(
            user_config_path(env(&vars)),
            Some(expected.join("gdship").join("config.toml"))
        );

        let expected = if cfg!(target_os = "macos") {
            PathBuf::from("/h")
                .join("Library")
                .join("Application Support")
        } else {
            PathBuf::from("/h").join(".config")
        };
        assert_eq!(
            user_config_path(env(&[("HOME", "/h")])),
            Some(expected.join("gdship").join("config.toml"))
        );
        assert_eq!(user_config_path(env(&[])), None);
    }
}
