## Design Guidance

Build only what the current requirement needs. Prefer standard tools and straightforward code; introduce abstractions, dependencies and configuration only for a concrete benefit. Treat every helper and test as an ongoing maintenance cost. Document temporary workarounds with the affected versions and removal conditions.

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
