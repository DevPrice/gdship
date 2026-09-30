# GitHub Actions

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
      GDSHIP_VERSION: 0.1.0
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
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          archive="$RUNNER_TEMP/gdship.tar.gz"
          curl -fsSL -o "$archive" "https://github.com/DevPrice/gdship/releases/download/v${GDSHIP_VERSION}/gdship-x86_64-unknown-linux-musl.tar.gz"
          gh attestation verify "$archive" --repo DevPrice/gdship
          tar -xzf "$archive" -C "$RUNNER_TEMP"
          echo "$RUNNER_TEMP" >> "$GITHUB_PATH"

      - name: Push to itch.io
        run: gdship push
        env:
          BUTLER_API_KEY: ${{ secrets.BUTLER_API_KEY }}
```

- Create an API key on itch.io under **Settings > API keys** and save it as the
  `BUTLER_API_KEY` repository secret. butler reads it from the environment in place of
  a login. gdship passes it only to butler, never to Godot or gdget, which run project
  and addon code.
- The checkout is at the tag, so `gdship push` uses it as the version. The checkout
  needs `fetch-depth: 0` or `fetch-tags: true` to see the tag. Without either, the
  version is the commit hash, and gdship warns that the clone is shallow.
- Download tools outside the checkout, as above. Files left in it count as uncommitted
  changes, and gdship refuses to push.
- gdship downloads butler itself.
- `gh attestation verify` checks that the gdship archive was built by gdship's release
  workflow. To keep an action from changing under you, pin it to a commit SHA rather
  than a tag such as `v7`.
- For a Godot patch release of 0, such as 4.7, the templates folder is `4.7.stable`,
  not `4.7.0.stable`.
- Under GitHub Actions, gdship reports warnings and errors as workflow annotations.
