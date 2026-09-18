## Design Guidance

Build only what the current requirement needs. Prefer standard tools and straightforward code; introduce abstractions, dependencies and configuration only for a concrete benefit. Treat every helper and test as an ongoing maintenance cost. Document temporary workarounds with the affected versions and removal conditions.

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

## Commits

Use [Scoped Commits](https://scopedcommits.com/): `<scope>: <description>`.
Choose the affected subsystem as the scope, such as `nix`, and keep descriptions concise.
