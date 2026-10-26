## Design Guidance

Build only what the current requirement needs. Prefer standard tools and straightforward code; introduce abstractions, dependencies and configuration only for a concrete benefit. Treat every helper and test as an ongoing maintenance cost.

For loading feedback, retain useful content and stable controls during background work. Delay optional progress by 300 ms; operations that finish sooner show no indicator. Once shown, keep progress visible for at least 500 ms. Successful presentation changes may wait for that minimum, but underlying work, input guards, cancellation and errors follow actual operation state immediately. Errors bypass presentation holds. Reserve space so progress does not move labels, buttons or headings. Closing or dismissing a surface must clear its presentation timers, and stale callbacks must never reopen it or change a newer operation. Keep recording/privacy indicators, actual availability changes, direct volume/brightness feedback and notifications immediate. Verify both fast and slow operations, minimum visibility, errors, cancellation, overlapping operations and timer cleanup on dismissal and teardown.

## Project structure

- `crates/shell/`: GTK desktop shell and desktop service integrations.
- `crates/cli/`: The `way-sh` command-line client.
- `crates/core/`: Shared protocols, types and application logic.
- `crates/shortcuts/`: Sway and Niri shortcut parsing.
- `data/` and `gresources.xml`: Settings schemas, themes and bundled resources.
- `scripts/install.sh`: Stages built binaries and resources for packaging.
- `nix/`: Development environment, package builds and checks; Fedora packaging and VM tooling live in `nix/packaging/fedora/`, OS installation tests in `nix/install-tests/` with Fedora tests in `nix/install-tests/fedora/`.
- `way-shell.spec`: Fedora RPM definition.
- `contrib/systemd/`: User service definition.
- `tests/`: Shared test fixtures; Rust integration tests live in `crates/*/tests/`.

## Development

- For CSS edits, verify live against the installed shell: export with `way-sh theme dump-dark` / `dump-light`, edit `$XDG_CONFIG_HOME/way-shell/way-shell-{dark,light}.css` (normally `~/.config/way-shell/`), then reload with `way-sh theme dark` / `light`. Repeating the same command still reloads. Do not rebuild, reinstall, or restart the user service (`systemctl --user`) for CSS-only verification. Copy the result back to `data/theme/` only when finished.
- For building and validation, use the Nix environment (`nix develop`, `nix build`, `nix flake check`, `nix run ...`). Do not build manually with plain `cargo`/toolbox/make.
- If a Nix command fails, stop. Fix it only when the fix is simple and report the Nix change to the user; otherwise stop and discuss with the user before proceeding.

## Commits

Use [Scoped Commits](https://scopedcommits.com/): `<scope>: <description>`.
Choose the affected subsystem as the scope, such as `nix`, and keep descriptions concise.

## Documentation

Do not add or create additional documentation unless explicitly requested. Update existing documentation references only when the changed surface requires it.

## Known Issues

When a bug appears specific to an upstream dependency or version, investigate whether upstream already resolves it. Prefer retaining an otherwise correct implementation when the upstream fix meets deployment requirements.

Also consider whether a proportionate implementation change would resolve the issue and provide concrete project benefits beyond avoiding the upstream bug. Assess whether those benefits justify the added complexity, maintenance cost and behaviour changes.

Discuss the evidence, options and trade-offs with the user, and obtain explicit agreement before implementing a local change. Keep compatibility workarounds narrowly scoped and document the affected versions and removal conditions.

- Sway 1.11 can leave stale monitor information after Quick Settings moves between displays, causing the screenshot toolbar to reopen on a previously used monitor. Sway 1.12 resolves this issue; retain the current implementation rather than adding a shell workaround.
- Fedora 43/44 build images can warn that `wireplumber.service` is added as a dependency to a non-existent `pipewire.service`. These images include WirePlumber and PipeWire development libraries without the PipeWire daemon. Image construction succeeds, and build jobs do not need audio services; do not add the daemon solely to silence this warning.
- Fedora 44 `systemd-udev` has an incorrect preset reference to the non-existent `systemd-tmpfiles-clear.service`, confirmed in `259.5-1.fc44` and `259.10-1.fc44`. The actual cleanup unit is `systemd-tmpfiles-clean.service`. The RPM tolerates the preset failure and image construction succeeds; leave Fedora's units and installation scripts unchanged, and recheck the reference when updating systemd.
