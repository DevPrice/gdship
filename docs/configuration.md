# Configuration

## Channels

By default, gdship makes one itch.io channel for each export preset in
`export_presets.cfg`, named by platform:

| Preset platform | Channel |
|---|---|
| Web | `html5` |
| Windows Desktop | `windows` |
| Linux | `linux` |
| macOS | `mac` |
| Android | `android` |

itch.io tags a channel with its platform when the name contains `windows`, `linux`,
`mac` or `android`. Presets for other platforms are skipped with a note.

To choose the channels yourself, or if two presets share a platform, add a `[channels]`
table to `gdship.toml` that maps each channel name to a preset name. The table then
lists every channel, in the order gdship exports and pushes them:

```toml
itch = "username/my-game"

[channels]
html5 = "Web"
windows = "Windows Desktop"
windows-demo = "Windows Demo"
```

Channel names may contain lowercase letters, digits and `-`.

gdship names each build after the file name in the preset's export path. Web builds are
always `index.html`, because itch.io serves the `index.html` at the root of a build.

## Find Godot

gdship uses the first of these that is set:

1.  `--godot PATH`
2.  The `GDSHIP_GODOT` environment variable
3.  `godot` in your [user config](#user-config)
4.  `godot`, then `godot4`, on `PATH`

If none of these is set and gdship runs in a terminal, it asks for the path to Godot
and saves your answer in the user config. You can paste a quoted path or, on macOS,
give the `Godot.app` bundle. Outside a terminal, as in CI, gdship stops with an error
instead.

On Windows, point gdship at the `_console.exe` that comes with Godot. The plain `.exe`
works too, and gdship still captures its output, but the console build is the one Godot
provides for command-line use.

The Godot version must match the `major.minor` version in `project.godot`, and the
export templates for that exact version must be installed, unless a preset sets a
custom release template. Install templates from the Godot editor's **Editor > Manage
Export Templates** dialog.

## User config

To keep machine-specific paths out of the project, gdship reads a `config.toml`. It
writes the file itself when it asks for Godot, or you can create it:

```toml
godot = "C:/Tools/Godot/Godot_v4.7.2-stable_win64_console.exe"
```

gdship looks for it at the first of these that applies:

- the path in `GDSHIP_CONFIG`
- `%APPDATA%\gdship\config.toml` on Windows
- `$XDG_CONFIG_HOME/gdship/config.toml`
- `~/Library/Application Support/gdship/config.toml` on macOS
- `~/.config/gdship/config.toml`

## butler

gdship uses butler from `GDSHIP_BUTLER` or your `PATH`. Otherwise it downloads the
butler version pinned in gdship, checks its sha256 hash, and caches it per user. Set
`GDSHIP_CACHE_DIR` to move the cache.
