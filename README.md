# gdship

gdship exports a Godot 4 project and pushes each build to an [itch.io](https://itch.io)
channel with [butler](https://itch.io/docs/butler/). It runs the same way on your
machine and in GitHub Actions.

gdship exports and checks every channel before it pushes any of them, so a failed export
never leaves your itch.io page half-updated. It names each build after its commit and
tag, refuses to ship uncommitted files, and downloads a pinned, verified copy of butler
if you don't have one.

## Install

Download the archive for your platform from the
[releases page](https://github.com/DevPrice/gdship/releases), extract `gdship`, and add
it to your `PATH`.

Alternatively, if you have Rust 1.89 or later, install from
[crates.io](https://crates.io/crates/gdship):

```sh
cargo install gdship --locked
```

## Quick start

Before you begin, create the game on itch.io, and install Godot and the export templates
for your project's Godot version.

1.  In your Godot project folder (the one that contains `project.godot`), run:

    ```sh
    gdship init
    ```

    When asked for the itch.io game, paste its address, such as
    `https://username.itch.io/my-game`. gdship writes it to `gdship.toml`.

    gdship keeps builds and logs in `.gdship/`, and `init` adds it to your
    `.gitignore`. If you don't have one, create a `.gitignore` with the line
    `/.gdship/`.

2.  Commit `gdship.toml` and `.gitignore`.

3.  Log butler in to your itch.io account. This opens your browser:

    ```sh
    gdship login
    ```

4.  Push:

    ```sh
    gdship push
    ```

    gdship exports one channel per preset in `export_presets.cfg`, such as `html5` and
    `windows`, and pushes each build, versioned by `git describe`. To tag a release and
    push it in one step, run `gdship push --tag v1.0.0`.

To build without pushing, run `gdship export`. The builds go to
`.gdship/build/CHANNEL/`.

## Documentation

- [Command reference](docs/commands.md): commands, flags, versions, environment
  variables, and exit codes.
- [Configuration](docs/configuration.md): channels, finding Godot, the user config, and
  butler.
- [GitHub Actions](docs/github-actions.md): push a release from CI.

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

    The tests use fake `godot`, `butler` and `gdget` programs and a local HTTP server,
    and don't need network access or a Godot install.

## License

gdship is licensed under the [MIT License](LICENSE).
