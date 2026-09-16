# Nix development and packages

This flake supports `x86_64-linux`. Install Nix with flakes enabled
(`experimental-features = nix-command flakes` in `nix.conf`). A multi-user
installation must have a running daemon; on a systemd host with the packaged
socket unit, enable it with `pkexec systemctl enable --now nix-daemon.socket`.
Check ordinary-user access with `nix store info --store daemon`.

```sh
nix develop                  # Tools, dependencies, GDB and local schemas
cargo build --workspace --locked
cargo test --workspace --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
target/debug/way-shell --help
nix build                    # Native Nix package, linked at result
nix build .#way-shell         # Explicit native package
nix profile add .#way-shell   # Install in your profile
```

The Rust 2024 workspace has three crates: `way-shell-core`, `way-shell` and
`way-sh`. The minimum Rust version is 1.90, matching Fedora 43's release compiler;
`flake.lock` pins the Nix development toolchain. Workspace dependencies share one
committed `Cargo.lock`. The CLI and core logic build without GTK.

Native dependencies are GTK4, libadwaita, gtk4-layer-shell, GLib/GIO, libnm,
WirePlumber 0.5, PipeWire, PulseAudio's client and GLib libraries, and Wayland.
The WirePlumber bindings use `saivert/wireplumber.rs` revision
`04fbbc6ea157cdd7061f6a5f2a6b22e102fccbcc` with `v0_5` and `futures`, alongside
GLib/GIO 0.21, GTK4 0.10 and libadwaita 0.8. The system WirePlumber daemon remains
in charge of audio policy. GCC and Clang are still build dependencies because
the Rust PipeWire bindings generate bindings and compile small native helpers.

Cargo's build script compiles and embeds GResources. Rust Wayland crates provide
the protocol bindings; no application C scanner or D-Bus generator is required.
GLib's resource and schema tools remain necessary. The old Fedora 40 toolbox
recipe has been removed; use this development shell or the Fedora RPM builds.

The development shell compiles schemas into `.cache/nix/schemas` and sets
`GSETTINGS_SCHEMA_DIR`; re-enter after changing the schema XML. No system schema
installation is needed. The installed Nix executable has its own GTK/GLib
wrapper, schemas and dconf backend. Its systemd user unit invokes that wrapper.
To use the unit from a profile, link its `lib/systemd/user/way-shell.service`
into your user unit directory, then run `systemctl --user daemon-reload`.

Start a debugger from `nix develop` so it inherits the native libraries,
schemas and current session variables, for example `gdb target/debug/way-shell`.
Local editor configuration belongs in the ignored `.vscode/` directory.
Do not commit user IDs, machine socket paths or remote debugger addresses.

Way-Shell requires a Wayland session and its selected Sway or Niri compositor.
A session D-Bus enables notification, tray and media integrations. Logind,
NetworkManager, PipeWire/WirePlumber, UPower and a power-profile provider supply
the corresponding optional controls. Unavailable services leave those controls
disabled until they recover. Nix packaging supplies the application's libraries;
the host manages its desktop services.
Persistent settings also require the dconf D-Bus service, normally supplied
by the host desktop.
Use the Sway configuration in the main README, including `way-sh` on PATH.
Niri selects its backend through the existing window-manager setting and
`NIRI_SOCKET` environment variable. Both compositors are available in `nix develop`.
Other desktops such as GNOME may already own the notification and status
notifier D-Bus names Way-Shell uses. A successful `--help` check does not test
desktop startup or guarantee compatibility with those sessions.

`scripts/install.sh` copies already-built release executables, schema XML,
the user unit and licenses. It defaults to `target/release` and `/usr`, and accepts
`DESTDIR`, `PREFIX`, `BINDIR`, `SCHEMADIR` and `USERUNITDIR`. Use
`CARGO_ARTIFACT_DIR` for a different binary directory, `LICENSEDIR` for a different
license destination, and `DEPENDENCY_LICENSES` for a directory of dependency
licenses. For example, to inspect a staged installation:

```sh
cargo build --workspace --release --locked
DESTDIR="$PWD/.cache/staging" PREFIX=/usr sh scripts/install.sh
```

The script leaves schema compilation to the packaging hooks. Nix wraps the
installed executable and adjusts the unit's `ExecStart`; Fedora keeps the
standard `/usr/bin/way-shell` path. The unit's existing identity and
`sway-session.target` installation target are preserved.

## Fedora RPMs

```sh
nix run .#rpm -- --list          # Authoritative supported-release list
nix run .#rpm                    # Build and verify every configured release
nix run .#rpm -- --fedora 44      # Build and verify only the selected release
nix run .#rpm -- --help
nix build .#rpms                 # Aggregate output directly
```

Individual outputs are named `rpm-fedora-VERSION`. Every public RPM output
depends on its installation check. Both the individual and aggregate outputs
use `result/rpms/fedora-VERSION-x86_64/`, containing the application RPM, SRPM
and debug RPMs. `result/logs/` contains build, console and installation logs.
Install the application RPM with `pkexec /usr/bin/dnf install /absolute/path/to/way-shell-VERSION-RELEASE.x86_64.rpm`,
selecting the file without `debuginfo` or `debugsource` in its name. The native
RPM has no Nix runtime dependency. The CLI builds the invoked flake's source
snapshot, including tracked local edits, regardless of the caller's directory.
Add new files to Git so Nix includes them in a local flake.

The RPM accepts either `power-profiles-daemon` or `tuned-ppd`. The installation
matrix uses the former on Fedora 43 and the latter on Fedora 44, and checks that
installation and removal preserve the selected provider.

The source archive contains manifests, lockfile, resources, fixtures, vendored
registry and Git dependencies, and their licenses. Vendoring dereferences Nix
store symlinks and installs portable Cargo source replacement configuration.
Fedora builds and tests use `--frozen` with guest networking disabled. The native
package uses `rustPlatform.buildRustPackage` and the same pinned Cargo inputs.

VM builds require usable `/dev/kvm` and `kvm` in Nix's `system-features`.
Allow both your user and the daemon's build users to use KVM through your
distribution's device access rules. Verify read/write access to `/dev/kvm`
and check `nix config show | grep system-features`. Native builds do not
need KVM. Images have a 16 GiB sparse virtual disk; creation and checks use 2 GiB RAM,
and compilation uses 3 GiB. Budget additional host RAM and disk for Nix's
dependencies. `nix build .#rpms --max-jobs 1 --cores 2` limits local VM build
resource use. The RPM app uses Nix's configured build limits.
RPM compilation uses `/var/tmp/way-shell-rpm` on the guest disk; vmTools'
RAM-backed `/tmp` is too small for the GTK Rust build artifacts.

## Checks and logs

```sh
nix flake check --no-update-lock-file \
  --option allow-import-from-derivation false \
  --max-jobs 1 --cores 2 --keep-going --print-build-logs
```

`native-package` checks executables, license, compiled schemas, the wrapped
systemd command and `--help`. `native-schemas` uses the installed wrapper's
environment, isolated data directories and GLib's memory settings backend to
discover and read every project schema.
`native-application-sway` and `native-application-niri` exercise the installed
executables and schemas in private compositor sessions. The Rust `schema-probe`
and `application-smoke` helpers live in the native package's separate `testHelpers`
output. Each Fedora build produces its own helpers for the fresh installation VM.
Both helpers are absent from the application's installed files and RPM payload.

The native build also runs Rust protocol fixtures and real compositor smoke
tests for both Sway and Niri. Sway runs headless; Niri runs as a nested compositor
inside that private Sway with Mesa software rendering. Niri probes connect to
Niri's own sockets and exercise literal workspace names, stable identifiers,
focus, GTK window movement and layer-shell surfaces. Neither test imports
environment variables into the desktop session or replaces the desktop shell.

To run these checks locally:

```sh
nix develop
cargo build -p way-shell --examples --locked
sh tests/component-smoke.sh target/debug/examples  # Complete component matrix
sh tests/wayland-component.sh target/debug/examples/sway-smoke sway
sh tests/wayland-component.sh target/debug/examples/niri-smoke niri
sh tests/wayland-component.sh target/debug/examples/theme-smoke niri
```

The application smoke test starts the supplied packaged executables in a private
session and checks startup, layer surfaces that actually receive committed
buffers, representative CLI commands, notification/tray D-Bus registration,
theme exports, duplicate activation, shutdown and restart. It uses the supplied
installed schemas. To run it against the native package:

```sh
nix develop
nix build .#way-shell
cargo build -p way-shell --example application-smoke --locked
sh tests/application-smoke.sh target/debug/examples/application-smoke \
  result/bin/way-shell result/bin/way-sh \
  result/share/gsettings-schemas/*/glib-2.0/schemas sway .cache/smoke-sway
sh tests/application-smoke.sh target/debug/examples/application-smoke \
  result/bin/way-shell result/bin/way-sh \
  result/share/gsettings-schemas/*/glib-2.0/schemas niri .cache/smoke-niri
```

The harness creates private D-Bus, runtime, configuration, data and cache
directories, uses memory settings, and isolates optional host services. Logs
remain in the final directory argument. These checks exercise the packaged shell
with unavailable optional services; component fixtures separately exercise audio,
network and other service behavior. Real hardware checks remain necessary for
Wi-Fi, audio devices, hotplug and suspend/resume.

The development shell and native check supply `WAY_SHELL_TEST_EGL_VENDOR` to
select the pinned Mesa EGL provider. If Niri reports `Egl(DisplayNotSupported)`,
run the probe through this environment. These software-rendered checks cover
compositor integration; real multi-output, input and hardware testing remains
part of the migration acceptance checks.

Each generated `rpm-fedora-VERSION` check builds an RPM with Fedora tools,
installs it in a fresh copy of the cached image with dependency checks and
scriptlets enabled, checks all schemas and executable loaders/libraries,
and runs the installed application smoke test under both Sway and Niri as an
ordinary guest user. It then removes the package and checks that its registration
and owned files disappeared. The Fedora-built schema and smoke helpers stay
outside the RPM payload, and the checks retain compositor and application logs.
Fedora tools use Fedora's standard data directories, without Nix schema
overrides. Guests have no repository network access. Builds and checks use
separate disposable copy-on-write disks.

These are Fedora userspaces under a Nix-provided VM kernel. The images include
build dependencies; minimal-desktop dependency completeness, a full Fedora
boot, physical graphics/input hardware and interactive desktop sessions are not
tested. Inputs and
environments are pinned; bit-for-bit reproducible RPMs remain future work.

Nix retains build logs, including failures; use `--print-build-logs` or
`nix log /nix/store/…drv`. To keep a failed VM for debugging, add `--keep-failed`;
the retained build directory includes `guest-console.log` and vmTools'
`run-vm` script. Successful output logs exclude the VM disks.

## Dependency maintenance

`fedora/config.nix` is the only supported-release configuration. The JSON
files under `fedora/locks/` record the Nixpkgs revision, requested packages,
repository metadata and every resolved RPM URL and SHA-256 hash.
Normal evaluation reads these files without repository resolution or
import-from-derivation. Nix's hashed fetchers download RPMs before guest work.

```sh
nix run .#update-fedora-locks
nix flake check --option allow-import-from-derivation false --max-jobs 1 --cores 2
```

Run the updater from the repository root. It obtains release repository
metadata and runs the pinned Nixpkgs RPM closure generator, writes temporary
manifests, and validates every image before replacing any committed manifest.
Unchanged configuration and repository metadata produce unchanged manifests.
Repository RPM dependency validation during image creation also catches
requirements the closure generator cannot resolve itself. Commit the reviewed
manifests and `flake.lock`; CI never refreshes either. Release repositories
are used without the rolling updates repository. If Fedora archives a release,
update its base URL and regenerate its lock, or remove support.

To add a release, add its configuration, run the updater, add the generated
manifest to Git, and pass the checks. To remove it, remove its configuration
and manifest. CLI choices, images, package outputs, checks and CI artifact
collection follow that configuration automatically.

## GitHub checks and downloads

The `Nix` workflow runs `nix-checks` on pushes and pull requests for `main`
and `nix`, or by manual dispatch. It uses an Ubuntu x86_64 runner with KVM,
one concurrent Nix build, two build cores and a 90-minute timeout. It needs no
repository secrets and uses read-only repository permissions, so fork pull
requests can run it subject to GitHub's workflow approval settings.

Open the repository's **Actions → Nix → run → Artifacts** to download
`way-shell-rpms` (organized by Fedora release) and `way-shell-check-logs`.
Artifacts expire after 14 days. Logs are uploaded even when checks fail;
RPMs are uploaded after all checks pass. VM images and the Nix store are
excluded. Available dependencies come from the standard Nix binary cache.
Persistent caching of project outputs, NixOS modules, full desktop tests and
branch protection are follow-up work. This workflow does not publish releases
or integrate with COPR.

The native recipe follows the packaging approach in
[Nixpkgs PR #343051](https://github.com/NixOS/nixpkgs/pull/343051/files), using
this repository's source and serial compilation.

## Troubleshooting

- If schemas cannot be found in a source build, re-enter `nix develop` after
  changing the XML. For installed Nix builds, run the wrapped `bin/way-shell`.
- If the wrong compositor is selected, inspect
  `gsettings get org.ldelossa.way-shell.window-manager backend` and the saved
  backend choice. Set it to `'sway'` or `'niri'`, then restart in that session.
  Required compositor and Wayland connection failures are reported at startup.
- Missing audio devices, a backlight, or an optional daemon disable the affected
  controls. Inspect the shell's startup logs and the corresponding host service;
  service and device updates restore available controls.
- If `way-sh` cannot connect, confirm that the shell is running in the same user
  session with the same `XDG_RUNTIME_DIR`. Its socket remains `way-shell.sock`,
  command replies have a two-second deadline, and help works without the shell.
- To diagnose a user-service launch, use
  `journalctl --user -u way-shell.service -b`. A compositor-launched instance logs
  through that compositor's launch environment. Stop the existing instance
  before launching a separate debug build.
