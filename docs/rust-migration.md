# Rust migration contracts and acceptance

The migration starts at `nix` commit `a9c7a1e1978bf1ce8453fce6e51349b5f2257c2b`.
Only the RPM power-provider change from
`05ab8e1b889ffda2e68cce6891d2ecb64c704b82` is carried over from Bluetooth work.
Keep the C shell working until each replacement passes its behavioral checks.

## Public contracts

- Keep `way-shell`, `way-sh`, `way-shell.service`, application ID
  `org.ldelossa.way-shell`, existing installation paths, and all schema IDs,
  keys and defaults in `data/org.ldelossa.way-shell.gschema.xml`.
- Preserve Sway and Niri operations, existing keybindings, focus, keyboard
  navigation, popup coordination, panel, quick settings, notifications, tray,
  calendar, media, switchers, application launching, OSD and dialogs.
- Preserve GTK4, libadwaita, gtk4-layer-shell, CSS resource paths and user
  theme files/hooks. Native daemons continue running independently.
- IPC uses a Unix datagram socket at `$XDG_RUNTIME_DIR/way-shell.sock`.
  Opcodes are the stable integers 0–33 in `ipc_commands.h`. Encode the
  existing x86_64 wire layout explicitly: little-endian `u32`, followed by
  an IEEE-754 little-endian `f32` only for volume-set (opcode 4). Replies are
  exactly four bytes, little-endian 0 or 1. Do not serialize Rust/C structs.
- Valid CLI groups: message-tray, volume, brightness (including keyboard),
  theme, activities, app-switcher, workspace-switcher, workspace-app-switcher,
  output-switcher, bluelight-filter and rename-switcher. Preserve subcommands.
- Intended fixes: validate datagram sizes and values; correct volume-set
  success; reject malformed/nonfinite/out-of-range volume arguments; provide
  help without a shell; use unique client addresses; validate socket paths;
  impose a two-second deadline. Exit codes: success 0, operational error 1,
  invalid arguments 2. Each change requires a regression test.

Baseline execution exposed another CLI bug: `IPC_RECV_MSG` read a pointer's
size into a one-byte C bool. With Nix hardening every reply aborted with
`buffer overflow detected`. Repair this before accepting CLI characterization:
receive a four-byte integer, check its size/value, then convert to bool.

Theme characterization also exposed a hook-path quoting failure: a configuration
directory containing spaces or an apostrophe could not run `on_theme_changed.sh`.
Pass the script and theme as separate Bash arguments. The regression fixture
uses a directory containing both characters.

Clock characterization exposed a skipped first minute: synchronization scheduled
the following minute without emitting the boundary it had just reached. Emit
that update immediately and release the temporary timestamp. The deterministic
test advances from 12:34:59 to 12:35:00 without waiting on real time.

The Rust theme service keeps `changed::light-theme` as a one-way observation:
one setting change produces one `theme-changed` signal and one hook invocation.
The C observer wrote the same setting through its setter, duplicating updates.
Repeating a theme command still reloads the CSS. Memory-backed settings tests
cover external changes and callback cleanup. Embedded CSS remains byte-for-byte
identical to the baseline, including its existing GTK parser diagnostics.

## Architecture and order

Rust 2024, minimum Rust 1.90, one Cargo.lock, shared workspace dependencies.
Permanent crates: `way-shell-core` (no GTK), `way-shell` (services/ui/platform),
`way-sh` (CLI, no GTK). A temporary static `way-shell-bridge` serves C callers.
Keep adapters outside permanent crates; own snapshots/handles explicitly,
disconnect callbacks before releasing owners, and never unwind over C ABI.
Keep a single GLib main loop and use asynchronous service operations and weak
widget references. Services expose application-owned types.

Audio uses saivert/wireplumber.rs with `v0_5`, initially revision
`04fbbc6ea157cdd7061f6a5f2a6b22e102fccbcc`. Prove its compiler, plugin,
connection and cleanup compatibility in Nix and both Fedora environments
before replacing C audio. Retain PulseAudio stream routing. NetworkManager
uses narrowly scoped libnm declarations with safe ownership wrappers.
Wayland uses wayland-client and wayland-protocols-wlr; migrate the separate
connection and all its proxies together. GDK retains shortcut inhibition.

Port CLI, calculations, settings/theme, clock, Sway, Niri, Wayland, power,
logind, power profiles, brightness, network, audio, media, notifications and
tray services before their UI consumers. Port windows/panel, switchers,
activities, quick settings, message tray, overlays, IPC server and schema
probe next. Move startup/shutdown last, then remove the bridge, project C,
generated C and Make. Use scoped commits, independently passing checks.
Generated-file cleanup and local editor configuration are separate commits.

## Verification gates

1. Establish baseline native package/schema checks and Fedora 43/44 offline
   build/install/uninstall checks. Add GLib core tests, CLI socket tests,
   volume/channel tests, gamma golden values, and Sway fixtures before ports.
2. Run affected tests per commit and the full package matrix before accepting
   a port. Test malformed/partial IPC, concurrent clients, timeouts, compositor
   reconnects, daemon restarts, removed hardware, failed operations, canceled
   callbacks and descriptor cleanup. Transfer fixtures to Rust as ports land.
3. Clean builds regenerate required bindings/resources and do not change
   tracked files. Cargo eventually owns all application compilation and tests;
   Nix, POSIX installation/packaging scripts and the Python lock updater stay.
4. Native Nix uses rustPlatform, bindgenHook when needed, wrapGAppsHook4 and
   installed-schema/dconf checks. Offline RPM sources contain dereferenced
   vendored dependencies, portable source replacement and dependency licenses.
   Fedora runs Cargo --frozen. Preserve layer-shell library load order.
5. Fedora 43 installs with power-profiles-daemon; Fedora 44 with tuned-ppd.
   Check the selected provider's exact version before/after install/removal.
   Check ELF loaders, missing libraries, Nix runtime leakage and owned files.
6. Run headless Wayland coverage and real Sway/Niri tests with audio, Wi-Fi,
   notifications, tray, multiple outputs, hotplug, suspend/resume and keyboard
   switchers. Report unavailable environments as unverified, never as passes.

Final commands:

```sh
nix develop
cargo build --workspace --locked
cargo test --workspace --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
nix flake check --no-update-lock-file --option allow-import-from-derivation false
nix build .#rpms --no-update-lock-file --option allow-import-from-derivation false --print-build-logs
```

Completion requires committed final source, both verified RPMs, an RPM release
newer than the laptop's installed package (initial candidate 0.0.10-3), and
installation of the Fedora 44 x86_64 binary RPM through
`pkexec /usr/bin/dnf --assumeyes install "$rpm"`. Recheck host version, artifact
metadata and provider first. Activate through the existing launch method,
avoid duplicate instances, inspect startup logs and verify installed behavior.
Record commit, artifacts, installed version and actual test results. Do not
install an intermediate migration package as the completed migration.
