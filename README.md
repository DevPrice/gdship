# gdship

gdship exports a Godot 4 project with the Godot command line and pushes each build to
an [itch.io](https://itch.io) channel with [butler](https://itch.io/docs/butler/). It
runs the same way on your machine and in GitHub Actions.

gdship exports every channel and checks each build before it pushes any of them, so a
failed export never leaves your itch.io page half-updated. It takes the version from
your git tag, refuses to ship uncommitted files, and downloads a pinned, verified copy
of butler if you don't have one.

gdship is a sibling of [gdget](https://github.com/DevPrice/gdget). If your project has
an `addons.toml`, gdship checks that `addons/` matches it before exporting.

## Install

Download the archive for your platform from the
[releases page](https://github.com/DevPrice/gdship/releases), extract `gdship`, and add
it to your `PATH`.

Alternatively, if you have Rust 1.89 or later, install from
[crates.io](https://crates.io/crates/gdship):

```sh
cargo install gdship --locked
```

## Set up a project

1.  Create the game on itch.io if it doesn't exist yet; butler can't create games.
    Then, in your Godot project folder (the one that contains `project.godot`), run:

    ```sh
    gdship init
    ```

    When asked for the itch.io game, paste its address, such as
    `https://devprice.itch.io/idle-factory`, or type `devprice/idle-factory`. gdship
    writes `gdship.toml`:

    ```toml
    itch = "devprice/idle-factory"
    ```

2.  Add gdship's working folder to `.gitignore`:

    ```gitignore
    /.gdship/
    ```

    gdship writes builds and logs there. `push` refuses to run until it is ignored,
    because the builds would otherwise make the next push see uncommitted files.

3.  Commit `gdship.toml` and `.gitignore`.

4.  Try an export:

    ```sh
    gdship export
    ```

    This builds every channel into `.gdship/build/<channel>/` and checks the output,
    without git, tags or butler. `export` also works without `gdship.toml`.

### Channels

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
table that maps each channel name to a preset name. The table then lists every channel,
in the order gdship exports and pushes them:

```toml
itch = "devprice/idle-factory"

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

1.  `--godot <path>`
2.  The `GDSHIP_GODOT` environment variable
3.  `godot` in your user config file (below)
4.  `godot`, then `godot4`, on `PATH`

If none of these is set and gdship runs in a terminal, it asks for the path to Godot
and saves your answer in the user config. You can paste a quoted path or, on macOS,
give the `Godot.app` bundle. Outside a terminal, as in CI, gdship stops with an error
instead.

The Godot version must match the `major.minor` version in `project.godot`, and the
export templates for that exact version must be installed, unless a preset sets a
custom release template. Install templates from the Godot editor's **Editor > Manage
Export Templates** dialog.

### User config

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

On Windows, point gdship at the `_console.exe` that comes with Godot. The plain `.exe`
works too, and gdship still captures its output, but the console build is the one Godot
provides for command-line use.

## Push to itch.io

### First-time setup

1.  Log butler in to your itch.io account. This opens your browser:

    ```sh
    gdship login
    ```

    butler keeps its credentials; gdship never stores them.

2.  Push once, then on the game's itch.io edit page, set **Kind of project** to
    **HTML** and mark the `html5` file as **This file will be played in the browser**.
    itch.io remembers this for later pushes to the channel.

### Release a version

Tag the commit and push it:

```sh
gdship push --tag v0.3.0
```

gdship checks that the tree is clean, creates the annotated tag, exports and verifies
every channel, pushes each one with `--userversion 0.3.0`, and then pushes the tag to
`origin`. If an export fails, gdship deletes the tag again.

If `HEAD` is already tagged, run `gdship push` with no flags. A leading `v` before a
digit is dropped for the itch.io version, so tag `v0.3.0` becomes version `0.3.0`.

Other flags:

| Flag | Effect |
|---|---|
| `--only <channel>` | Only this channel. Repeat it for more. |
| `--version <v>` | Use this itch.io version instead of a tag. |
| `--allow-dirty` | Push with uncommitted changes. The version is then `git describe --tags --always --dirty`, unless `--version` is given. |
| `--dry-run` | Run every check, then print the Godot and butler commands without running them. |
| `-v` | Show Godot's output as it runs. It is always saved in `.gdship/logs/`. |

`gdship status` shows butler's status for each channel.

### butler

gdship uses butler from `GDSHIP_BUTLER` or your `PATH`. Otherwise it downloads the
butler version pinned in gdship, checks its sha256 hash, and caches it per user (set
`GDSHIP_CACHE_DIR` to move the cache).

## GitHub Actions

This job pushes a release whenever you push a `v*` tag:

```yaml
name: Release

on:
  push:
    tags: ["v*"]

jobs:
  itch:
    runs-on: ubuntu-latest
    env:
      GODOT_VERSION: 4.7.2
    steps:
      - uses: actions/checkout@v7
        with:
          fetch-depth: 0

      - name: Install Godot and export templates
        run: |
          base="https://github.com/godotengine/godot/releases/download/${GODOT_VERSION}-stable"
          curl -fsSL -o "$RUNNER_TEMP/godot.zip" "$base/Godot_v${GODOT_VERSION}-stable_linux.x86_64.zip"
          curl -fsSL -o "$RUNNER_TEMP/templates.tpz" "$base/Godot_v${GODOT_VERSION}-stable_export_templates.tpz"
          unzip -q "$RUNNER_TEMP/godot.zip" -d "$RUNNER_TEMP"
          unzip -q "$RUNNER_TEMP/templates.tpz" -d "$RUNNER_TEMP"
          templates="$HOME/.local/share/godot/export_templates"
          mkdir -p "$templates"
          mv "$RUNNER_TEMP/templates" "$templates/${GODOT_VERSION}.stable"
          echo "GDSHIP_GODOT=$RUNNER_TEMP/Godot_v${GODOT_VERSION}-stable_linux.x86_64" >> "$GITHUB_ENV"

      - name: Install gdship
        run: |
          curl -fsSL https://github.com/DevPrice/gdship/releases/latest/download/gdship-x86_64-unknown-linux-musl.tar.gz \
            | tar -xz -C "$RUNNER_TEMP"
          echo "$RUNNER_TEMP" >> "$GITHUB_PATH"

      - name: Push to itch.io
        run: gdship push
        env:
          BUTLER_API_KEY: ${{ secrets.BUTLER_API_KEY }}
```

- Create an API key on itch.io under **Settings > API keys** and save it as the
  `BUTLER_API_KEY` repository secret. butler reads it from the environment in place of
  a login.
- The checkout is at the tag, so `gdship push` finds the version without flags. The
  checkout needs `fetch-depth: 0` or `fetch-tags: true` to see the tag.
- Download tools outside the checkout, as above. Files left in it count as uncommitted
  changes, and gdship refuses to push.
- gdship downloads butler itself.
- For a Godot patch release of 0, such as 4.7, the templates folder is `4.7.stable`,
  not `4.7.0.stable`.
- Under GitHub Actions, gdship reports warnings and errors as workflow annotations.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. |
| 1 | Any failure, including a push that succeeded but whose tag could not be pushed. |
| 2 | A usage error, such as an unknown flag or an `--only` channel the project doesn't have. |

## Build from source

Before you begin, install Rust 1.89 or later from [rustup.rs](https://rustup.rs).

1.  Clone the repository:

    ```sh
    git clone https://github.com/DevPrice/gdship.git
    cd gdship
    ```

2.  Build a release binary:

    ```sh
    cargo build --release
    ```

    The binary is `target/release/gdship` (`gdship.exe` on Windows).

3.  Run the tests and lints that CI runs:

    ```sh
    cargo test
    cargo clippy --all-targets -- -D warnings
    cargo fmt --check
    ```

    The tests run gdship against fake `godot`, `butler` and `gdget` programs and a
    local HTTP server, and don't need network access or a Godot install.

## License

gdship is licensed under the [MIT License](LICENSE).
