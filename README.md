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

For Nix development, an installable native Nix package, and locally verified
Fedora RPMs, see [the Nix commands below](#nix-commands). Run `nix run .#rpm -- --list`
for the supported Fedora releases.

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
| `nix build` | Build the native package and run its Cargo tests, linking the installed package at `result`. |
| `nix build .#way-shell` | Build the native package explicitly, including its Cargo tests. |
| `nix profile add .#way-shell` | Install the native package into your Nix profile. |
| `nix flake check` | Run Rust formatting, Clippy, native package and schema checks, including the package's Cargo tests. |
| `nix build .#check-format-rs` | Check Rust formatting without editing files. |
| `nix build .#clippy-rs` | Run Clippy across the Rust workspace and all targets with warnings treated as errors. |
| `nix build .#native-package` | Check installed binaries, licenses, schemas, service paths and help output. |
| `nix build .#native-schemas` | Check schema discovery through the installed application's wrapper environment. |
| `nix build .#native-components` | Run the audio, GTK component and Sway/Niri runtime test suite. |
| `nix build .#native-application-sway` | Test the installed application in an isolated Sway session. |
| `nix build .#native-application-niri` | Test the installed application in an isolated Niri session. |
| `nix build .#check-integration` | Run all three native integration targets. |
| `nix build .#cargo-vendor` | Fetch the locked Rust dependencies into a Cargo vendor directory. |
| `nix run .#rpm` | Build and verify RPMs for every configured Fedora release. |
| `nix run .#rpm -- --fedora VERSION` | Build and verify RPMs for the selected Fedora release. |
| `nix run .#rpm -- --list` | List the supported Fedora releases. |
| `nix run .#rpm -- --help` | Show the RPM command's usage. |
| `nix build .#rpms` | Build the aggregate verified RPM output directly. |
| `nix build .#rpm-fedora-VERSION` | Build the verified RPM output for the selected Fedora release directly. |
| `nix run .#update-fedora-locks` | Regenerate and validate the configured Fedora dependency locks. |
