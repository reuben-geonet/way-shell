# Way-Shell

A GNOME-inspired desktop shell for Sway and Niri, written in Rust.

## Keyboard shortcuts

Use the panel’s **Keyboard shortcuts** button or `way-sh shortcuts toggle` to
view saved Sway/Niri shortcuts and Way-Shell navigation.

Way-Shell requires a Wayland session and its selected Sway or Niri compositor.
A session D-Bus enables notification, tray and media integrations. These desktop
services provide the corresponding controls:

- Logind for session actions and inhibitors
- NetworkManager for networking
- BlueZ for Bluetooth
- WirePlumber/PipeWire for audio
- UPower for battery state
- power-profiles-daemon or tuned-ppd for power profiles

Unavailable optional services disable their controls. The controls recover when
the services return.

Way-Shell is in its very early stages of development so expect some crashes and
bugs. However, it is my daily driver and I'm rather quick to fix show stopper
issues.

The demo above is using [SwayFX](https://github.com/WillPower3309/swayfx) which I really enjoy.

## Installing and Running Way-Shell 

A [copr](https://copr.fedorainfracloud.org/coprs/ldelossa/Way-Shell/) exists for installing Way-Shell on Fedora. 

For Nix development, an installable native Nix package, and locally built
Fedora RPMs, see [the Nix commands below](#nix-commands). Run `nix flake show`
to discover available targets.

An [AUR package](https://aur.archlinux.org/packages/way-shell) also exists for Arch-based distros, which can be installed with any AUR helper, or with `makepkg` if you're feeling lucky.

To build from source, use the pinned Nix development environment:

```sh
nix develop
cargo build --workspace --locked
cargo test --workspace --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
target/debug/way-shell --help
```

The workspace uses Rust 2024 and requires Rust 1.90 or newer. Its four crates are
`way-shell-core` (protocols and shared logic), `way-shell` (services and GTK UI),
`way-shell-shortcuts` (compositor shortcut parsing), and `way-sh` (the command-line
client, without GTK). `Cargo.lock` pins Rust
dependencies; the Nix environment supplies the compiler and native libraries.
See [the Nix commands below](#nix-commands) for packaging and optional developer
checks.

The old Fedora 40 toolbox build and Makefiles have been retired. Cargo is the
application build tool; `scripts/install.sh` stages already-built files for Nix
and RPM packaging.

Contributions are very welcome if you're using Way-Shell on a distro other then Fedora!

## Video Demonstration

[![IMAGE ALT TEXT HERE](https://img.youtube.com/vi/sOooD4Q3mYU/0.jpg)](https://www.youtube.com/watch?v=sOooD4Q3mYU)

## Configuring Sway

Sway is the ultimate controller of keybinds and must be configured to
specifically work with Way-Shell.

Sway integrates with Way-Shell via its CLI `way-sh`.

Below is the expected Sway config which Way-Shell supports.

```shell
set $mod Mod4
# way-shell configuration
bindsym XF86AudioRaiseVolume exec way-sh volume up
bindsym XF86AudioMute exec way-sh volume mute
bindsym XF86AudioLowerVolume exec way-sh volume down
bindsym XF86MonBrightnessDown exec way-sh brightness down
bindsym XF86MonBrightnessUp exec way-sh brightness up
bindcode --release 133 exec way-sh activities toggle
bindsym $mod+Tab exec way-sh app-switcher toggle
bindsym $mod+o exec way-sh output-switcher toggle
bindsym $mod+w exec way-sh workspace-switcher toggle
bindsym $mod+a exec way-sh workspace-app-switcher toggle
bindsym $mod+r exec way-sh rename-switcher toggle
```

## Using Way-Shell

Way-Shell aims to feel like a "natural" desktop environment.

The below table lists useful keymaps to keep in mind.

| Keybinding | Action |
|------------|--------|
| Super + Tab | Open the app switcher (release Super to close) |
| Super + g  | (In app switcher) move to next instance of an application if it exists |
| Super + Shift + g  | (In app switcher) move to previous instance of an application if it exists |
| Super + w | Open the workspace switcher (repeat to toggle close) |
| Ctrl + Return | Use the exact search text as the workspace target, even when it partially matches an existing name. Workspace creation depends on the compositor. |
| Super + a | Open the workspace app switcher (repeat to toggle close) |
| Super + o | Open the output switcher (repeat to toggle close) |
| Super + r | Open the rename workspace switcher (repeat to toggle close) |

The workspace, workspace-app and output switchers support these search/list keys.

| Keybinding | Action |
|------------|--------|
| Tab | next item |
| Shift + Tab  | previous item |
| Ctrl + n | next item |
| Ctrl + p  | previous item |
| DownArrow | next item |
| UpArrow  | previous item |
| Esc | Clear search, close widget if search is empty |

The app switcher instead uses Super+Tab and Super+Shift+Tab to select applications,
Super+g or Super+grave to select instances, and Shift to reverse instance selection.
Super+Escape cancels; releasing Super activates the selection.

`way-sh --help` and subcommand help work without a running shell. Commands use
`$XDG_RUNTIME_DIR/way-shell.sock` in the current user session. A command exits
with status 0 on success, 1 on an operational failure, or 2 for invalid arguments,
and waits at most two seconds for a response. For example, `way-sh volume set 0.5`
sets the output volume to 50%; the value must be finite and between 0 and 1.

## Configuring Niri

Way-Shell uses the same `way-sh` commands under Niri. Start it inside the Niri
session, where `NIRI_SOCKET` and `WAYLAND_DISPLAY` identify that compositor.
Select the backend explicitly with:

```sh
gsettings set org.ldelossa.way-shell.window-manager backend 'niri'
```

Restart Way-Shell after changing the backend. Set it to `'sway'` when selecting
Sway explicitly. A saved backend choice takes precedence over session detection.
The included user service is still named `way-shell.service` and retains its
`sway-session.target` installation target; a Niri session should launch the shell
through its own startup configuration or manage the service with its session.

## Settings and themes

Existing `org.ldelossa.way-shell` GSettings schemas and saved settings are retained.
Use `gsettings list-recursively org.ldelossa.way-shell.system` to inspect system
settings, and the `.panel`, `.notifications` and `.window-manager` schemas for
the other controls. The installed package compiles its schemas; `nix develop`
provides a local schema directory for source builds.

Custom themes live in `$XDG_CONFIG_HOME/way-shell` (normally
`~/.config/way-shell`). `way-sh theme dump-dark` and `way-sh theme dump-light`
write the bundled CSS to `way-shell-dark.css` and `way-shell-light.css` there,
replacing an existing file of the same name. Edit those files, then run
`way-sh theme dark` or `way-sh theme light` to load them; repeating the command
reloads the CSS. Missing custom CSS falls back to the embedded theme.

If `on_theme_changed.sh` exists in that directory, Way-Shell runs it with Bash
and one argument, `dark` or `light`. The script need not be executable.

### Bluetooth

**Bluetooth Settings** opens Blueman for discovery, pairing, and device removal.
Install Blueman separately or choose another manager:

```sh
gsettings set org.ldelossa.way-shell.system bluetooth-settings-command 'your-manager --option'
gsettings reset org.ldelossa.way-shell.system bluetooth-settings-command
```

The command accepts quoted arguments without shell expansion. Changes apply on
the next click, and launch failures appear in the menu. Power and device actions
work independently of the external manager.

### Integrating with SwayFX

The [SwayFX](https://github.com/WillPower3309/swayfx) project provides some extra eye candy for Sway.

Way-Shell components can integrate with SwayFX settings for some added visual effects.

For instance:

![image](./contrib/images/swayfx-way-shell.png)

```
corner_radius 6
shadows enable
default_dim_inactive 0.05
layer_effects "way-shell-message-tray-underlay" blur enable; shadows enable;
layer_effects "way-shell-message-tray" shadows enable; corner_radius 20
layer_effects "way-shell-quick-settings-underlay" blur enable; shadows enable;
layer_effects "way-shell-quick-settings" shadows enable; corner_radius 20
layer_effects "way-shell-switcher" shadows enable; corner_radius 20
layer_effects "way-shell-quick-switcher" shadows enable; corner_radius 20
layer_effects "way-shell-dialog" blur enable; shadows enable;
```

## Nix commands

Run these from the repository root.

| Command | Description |
| --- | --- |
| `nix develop` | Enter the development environment with Rust tools, native libraries and local schemas. |
| `cargo fmt` (inside `nix develop`) | Apply Rust formatting to the working files. |
| `nix build` | Build the native package, linking it at `result`. |
| `nix build .#way-shell` | Build the native package explicitly. |
| `nix profile add .#way-shell` | Install the native package into your Nix profile. |
| `nix flake check` | Run Rust formatting and the lightweight native package check. |
| `nix build .#check-format-rs` | Check Rust formatting without editing files. |
| `nix build .#clippy-rs` | Run Clippy across the Rust workspace and all targets with warnings treated as errors. |
| `nix build .#native-package` | Check installed executable startup, licenses, service paths and schemas. |
| `nix build .#cargo-vendor` | Fetch the locked Rust dependencies into a Cargo vendor directory. |
| `nix build .#rpms -L` | Build RPMs for every configured Fedora release. |
| `nix build .#rpm-fedora-VERSION -L` | Build RPMs for the selected Fedora release. |
| `nix build .#test-install -L` | Test package installation and removal on every supported system. |
| `nix build .#test-install-fedora -L` | Test all supported Fedora releases. |
| `nix build .#test-install-fedora-VERSION -L` | Test one Fedora release. |
| `nix build .#test-install-nixos -L` | Test installation into a user's Nix profile inside NixOS. |
| `nix flake show` | Discover available targets. |
| `nix run .#update-fedora-locks` | Regenerate and validate the configured Fedora dependency locks. |

Native package builds use two Nix outputs: `cargo-deps-native` compiles a Crane
stub workspace, and `way-shell` imports those artifacts before compiling the real
workspace. Both use the pinned Nix compiler and libraries, release profile,
workspace binaries, frozen dependencies and one job. Vendored registry and Git
sources and license collection are shared with RPM packaging. Clippy and the
development shell retain their separate workflows.

Every configured Fedora release has the same split through
`cargo-deps-fedora-VERSION` and `rpm-fedora-VERSION`.
Each stage boots a fresh VM from the same locked Fedora image. Both use the same
RPM build paths, `%cargo_prep` and `%cargo_build`, including Fedora's `rpm`
profile and `target/release` symlink. The first stage stops after `%build`, cleans
workspace artifacts with Cargo, and exports the remaining target tree using the
pinned Crane hooks. The application stage unpacks a writable copy after `%prep`.
Fedora uses Crane's
directory export followed by a tar archive that preserves timestamps: the default
compressed exporter resets them to epoch 1, which predates Fedora headers tracked
by bindgen. The adapter does not edit Cargo fingerprints.
The optional `way_shell_artifacts_import` and `way_shell_artifacts_export` RPM
macros are supplied only by Nix; normal RPM/SRPM builds and published application
source archives do not require a cache.

Compiled artifacts are separate for native Nix and every Fedora release and
architecture, since their compilers and native libraries differ.

Application code, build-script contents, resources, documentation and installation
tests do not invalidate dependency artifacts. Dependency declarations, Cargo
features/profiles, the lockfile, compiler, native environment and compilation
settings do. Fedora also conservatively invalidates on any spec edit. Crane
preserves the dependency-relevant manifest fields; adding or removing Cargo
targets can also invalidate the stub. These are immutable Nix outputs, not VM
snapshots or persistent incremental compilation directories. Workspace crates
are expected to rebuild. CI retains explicit dependency output roots until the
existing Nix cache save step; the installation-test matrix is unchanged.

Useful diagnostics:

```sh
nix eval --option allow-import-from-derivation false .#cargo-deps-native.drvPath
nix build .#cargo-deps-native --out-link result-cargo-deps -L
nix path-info --closure-size ./result-cargo-deps
python3 nix/check-dependency-inputs.py
python3 nix/check-dependency-inputs.py --build
python3 nix/check-dependency-inputs.py --fedora 43 --fedora 44
nix log .#way-shell  # Expect Fresh gtk4/glib/wireplumber and Compiling way-shell.
nix build .#cargo-deps-fedora-43 --out-link result-cargo-deps-fedora-43 -L
nix build .#rpm-fedora-43 -L
nix build .#test-install-fedora-43 -L
```

The regression script evaluates isolated source edits with import-from-derivation
disabled; `--build` additionally builds the native package before and after an
application edit and checks reuse and real binary help output. `--fedora VERSION` checks the same
reuse contract using RPM build logs in fresh VMs. On a KVM host,
repeat the Fedora build after an application-only edit: the dependency derivation
path must stay fixed, third-party crates (including GTK) must be `Fresh`, and real
workspace code and the shell build script must compile. Check schemas/resources
and run the existing installation tests. Also build the application SRPM without
the optional macros. Compare initial and follow-up CI runs for dependency cache
restoration, compilation skipped, elapsed build time and closure size; local
build reuse alone does not establish CI cache persistence.

Local validation on 2026-09-18 passed flake checks with IFD disabled, native
and Fedora 43 application-edit reuse, NixOS profile installation and Fedora 43
installation. All external crates were fresh in application builds, including GTK
and WirePlumber; only the four workspace crates compiled. Measured Cargo phase
times and dependency output sizes were:

| Environment | Cold dependencies | Application | After source edit | Artifact NAR | Closure |
| --- | --- | --- | --- | --- | --- |
| Native Nix | 5m10s | 1m27s | 1m44s | 197.1 MiB | 421.0 MiB |
| Fedora 43 | 6m37s | 1m35s | 1m32s | 387.7 MiB | 620.9 MiB |

An ordinary Fedora 43 build without artifact macros also passed (10m29s in
Cargo), with matching RPM requirements, provides, scriptlets, installed-file
metadata and all 641 license paths/checksums. Its SRPM contains the standalone
spec and vendored configuration. VM validation ran sequentially to fit the host's
available disk space. These are local measurements; initial/follow-up GitHub
Actions cache restoration and timings still need to be compared after CI runs.
