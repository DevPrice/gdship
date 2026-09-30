# Command reference

Except for `login`, run commands from your Godot project folder or any folder inside
it. gdship uses the nearest folder above that contains `project.godot`.

## gdship init

Creates `gdship.toml`, asking which itch.io game the project pushes to. Create the game
on itch.io first; butler can't create games. Answer with the game's address, such as
`https://username.itch.io/my-game`, or type `username/my-game`:

```toml
itch = "username/my-game"
```

If the project has a `.gitignore`, `init` also adds `/.gdship/` to it. Otherwise, create
a `.gitignore` with that line. gdship writes builds and logs to `.gdship/`, and `push`
refuses to run until it is ignored, because the builds would otherwise make the next
push see uncommitted files.

## gdship export

Exports every channel into `.gdship/build/CHANNEL/` and checks each build, without git,
tags or butler. `export` works without `gdship.toml`.

| Flag | Effect |
|---|---|
| `--only CHANNEL` | Only this channel. Repeat it for more. |
| `--godot PATH` | Use this Godot binary. See [Find Godot](configuration.md#find-godot). |
| `-v`, `--verbose` | Show Godot's output as it runs. It is always saved in `.gdship/logs/`. |

If your project has gdget's `addons.toml`, gdship first runs `gdget sync --check` to
check that `addons/` matches it. If gdget isn't on `PATH`, gdship warns and carries on.

## gdship push

Checks that the work tree is clean, exports and verifies every channel, and then pushes
each build. If an export fails, nothing is pushed.

The itch.io version names the commit, as `git describe --tags` does:

| HEAD | Version |
|---|---|
| tagged `v0.3.0` | `0.3.0` |
| four commits after `v0.3.0` | `0.3.0-4-gabc1234` |
| in a repository with no tags | `abc1234` |

A leading `v` before a digit is dropped. itch.io orders builds by upload time, so the
version only has to identify the build.

Outside a git repository, or before the first commit, gdship warns that it can't check
for uncommitted files and lets itch.io number the builds.

The first time you push a Web build, open the game's itch.io edit page, set **Kind of
project** to **HTML**, and mark the `html5` file as **This file will be played in the
browser**. itch.io remembers this for later pushes to the channel.

`push` takes the flags of `export`, and these:

| Flag | Effect |
|---|---|
| `--tag TAG` | Create an annotated tag on HEAD, use it as the version, and push it to `origin` once every channel is pushed. If nothing reaches itch.io, gdship deletes the tag again. |
| `--version V` | Use this itch.io version, exactly as given. Skips the tag lookup. |
| `--allow-dirty` | Push with uncommitted changes. The version then ends in `-dirty`. |
| `--dry-run` | Run every check, then print the Godot and butler commands without running them. |

## gdship login

Logs butler in to your itch.io account, opening your browser if needed. butler keeps
its credentials; gdship never stores them.

## gdship status

Shows butler's status for each channel.

## Environment variables

| Variable | Effect |
|---|---|
| `GDSHIP_GODOT` | The Godot binary. See [Find Godot](configuration.md#find-godot). |
| `GDSHIP_CONFIG` | The user config file. See [User config](configuration.md#user-config). |
| `GDSHIP_BUTLER` | The butler binary. See [butler](configuration.md#butler). |
| `GDSHIP_CACHE_DIR` | Where gdship caches the butler it downloads. |
| `BUTLER_API_KEY` | An itch.io API key, which butler uses in place of a login. gdship passes it only to butler, never to Godot or gdget, which run project and addon code. |

Under GitHub Actions, gdship reports warnings and errors as workflow annotations.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. |
| 1 | Any failure, including a push that succeeded but whose tag could not be pushed. |
| 2 | A usage error, such as an unknown flag or an `--only` channel the project doesn't have. |
