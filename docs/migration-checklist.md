# Rust migration checklist

Checked implementation items have passed their local checks. The package and
deployment gates below are tracked separately; an implementation check is not
a claim that the complete migration is ready to install.

- [x] 1. Accept tuned-ppd as a power profile provider.
- [x] 2. Verify both RPM power profile providers.
- [x] 3. Record migration contracts and acceptance checks.
- [x] 4. Characterize core and CLI behavior.
- [x] 5. Run baseline tests in Nix and Fedora.
- [x] 6. Repair generation and untrack the 24 generated protocol files.
- [x] 7. Keep machine-specific editor configuration local.
- [x] 8. Validate IPC requests and correct command results.
- [x] 9. Make CLI argument and socket handling reliable.
- [x] 10. Introduce Cargo in both packaging paths.
- [x] 11. Verify the WirePlumber 0.5 Rust binding candidate.
- [x] 12. Replace the CLI with Rust.
- [x] 13. Bridge Rust volume and gamma calculations to C.
- [x] 14. Migrate settings and theme handling to Rust.
- [x] 15. Migrate clock scheduling to Rust.
- [x] 16. Migrate the Sway backend to Rust.
- [x] 17. Migrate the Niri backend to Rust.
- [x] 18. Migrate Wayland connection and protocol ownership to Rust.
- [x] 19. Migrate UPower integration to Rust.
- [x] 20. Migrate logind session actions and inhibitors to Rust.
- [x] 21. Migrate power profile management to Rust.
- [x] 22. Migrate display and keyboard brightness to Rust.
- [x] 23. Migrate network inventory and radio state to Rust.
- [x] 24. Migrate network connection management to Rust.
- [x] 25. Migrate WirePlumber object tracking to Rust.
- [x] 26. Migrate volume and stream routing to Rust.
- [x] 27. Migrate MPRIS integration to Rust.
- [x] 28. Migrate the notification service to Rust.
- [x] 29. Migrate tray watcher and item tracking to Rust.
- [x] 30. Migrate D-Bus menu handling to Rust.
- [x] 31. Introduce Rust window and visibility controllers.
- [x] 32. Migrate panel indicators and workspaces to Rust.
- [x] 33. Migrate shared and application switchers to Rust.
- [x] 34. Migrate workspace, output, and rename switchers to Rust.
- [x] 35. Migrate application discovery and launching to Rust.
- [x] 36. Migrate quick settings layout and system controls to Rust.
- [x] 37. Migrate network controls to Rust.
- [x] 38. Migrate mixer controls to Rust.
- [x] 39. Migrate notification presentation to Rust.
- [x] 40. Migrate calendar and media presentation to Rust.
- [x] 41. Migrate OSD and dialog overlays to Rust.
- [x] 42. Replace the shell IPC server with Rust.
- [x] 43. Replace the packaging schema probe with Rust.
- [x] 44. Move startup and shutdown into Rust.
- [x] 45. Make Cargo the sole application build tool; remove C and the bridge.
- [x] 46. Remove obsolete build dependencies.
- [x] 47. Add packaged Wayland smoke coverage.
- [ ] 48. Complete Rust development and migration documentation.
- [ ] 49. Advance the RPM release over the installed package.

## Package and deployment gates

- [x] Baseline native package/schema checks and both Fedora RPMs.
- [x] Full matrix through the Rust CLI and calculation bridge (`706e68e`).
      Both Fedora compilers also passed the WirePlumber connection/plugin probe.
- [x] Settings/theme tests and mapped layer-window smoke test (`1955558`).
- [x] Clock timing, cancellation, and C integration checks (`95149f5`).
- [x] Sway framing, reconnection, literal names, focus, output hotplug,
      workspace movement, ownership, C integration and Clippy checks.
- [x] Theme package matrix covered by `709d126`, including the private D-Bus configuration fix from `a034a65`.
- [x] Full matrix covering clock/Sway (`709d126`): native application/schema checks and both Fedora RPM build/install/uninstall checks passed after resuming from the reboot. Artifacts: `../artifacts/sway-resumed-rpms` and `sway-resumed-rpms-1` in the durable migration cache.
- [x] Niri development environment, private socket fixtures, real workspace/window actions and layer-window smoke checks.
- [x] Wayland protocol fixtures, descriptor transfer, backpressure, repeated ownership and real Sway/Niri component checks (`fa909b9`).
- [x] UPower, logind, power-profile and brightness service fixtures, adapter ownership, GTK controls, full workspace tests, Clippy and combined C/Rust build through `767efca`.
- [x] Fedora 43/44 dependency images validate the private D-Bus test dependency (`c64a75f`).
- [x] Native package/schema check for the services through `767efca`, built from `2f40bf7`; artifact `../artifacts/power-services-native-schemas`.
- [x] Fedora 43/44 build/install/uninstall matrix for those services (`2f40bf7`); artifact `../artifacts/power-services-rpms` -> `/nix/store/chygaa5mn4sm3lk83wq9c4snz61dgzwb-way-shell-rpms`.
- [x] Network inventory/radio private D-Bus fixtures using real libnm, C facade recovery, ownership, full local build/workspace checks and Clippy.
- [x] Clean native application/schema package checks for network inventory (`3c5ac97`), including existing Sway/Niri component coverage; artifact `../artifacts/network-inventory-native-schemas`.
- [x] Fedora package matrix for the network inventory port, covered by `0cb7076`.
- [x] Rust Wi-Fi, saved profiles, VPN/WireGuard actions, concurrent requests, cancellation, raw SSIDs, removal/restart, C argument ownership, workspace tests, Clippy and linked application checks.
- [x] Clean native application/schema package checks for network connections (`0cb7076`); artifact `../artifacts/network-connections-native-schemas` -> `/nix/store/ccmr6s3h5w0q65h2kjs0a70xiijbfq8l-way-shell-native-schemas-check`.
- [x] Fedora 43/44 package matrix for network connections (`0cb7076`): build/install/uninstall and selected power providers pass; artifact `.cache/rust-migration/artifacts/network-connections-rpms` -> `/nix/store/h0b9569m0w2kmfxynh6pvmxnsdl5g29h-way-shell-rpms`.
- [x] Rust audio tracking: private PipeWire devices, streams, ports, links, defaults, mixer notifications, microphone activity, hotplug/restart, owned snapshots, descriptor cleanup, C ABI and adapter recovery; workspace tests, Clippy and linked application pass.
- [x] Native/Fedora package matrix for Rust audio tracking, covered by `ff6a042`.
- [x] Rust volume/routing controls, confirmed playback/capture moves, concurrent requests, cancellation, restart, deadlines, C ABI, amplified-volume compatibility, full workspace tests, formatting, Clippy and linked build.
- [x] Fedora 43/44 dependency images validate the private PulseAudio routing fixtures; both locks add only `pipewire-pulseaudio`.
- [x] Native/Fedora package matrix for the completed Rust audio service, covered by `ff6a042`.
- [x] Clean native audio application/schema checks at `6571864`, including Sway/Niri components; artifact `.cache/rust-migration/artifacts/audio-controls-native-schemas` -> `/nix/store/nkdsb5q80kgkyc71i95pwbvqyqbszz69-way-shell-native-schemas-check`.
- [x] Reproduce and fix Fedora 43 audio fixture negotiation (`55b95d8`): the offline diagnostic reproduces the old bridge timeout and passes both corrected fixtures; native checks also pass.
- [x] Full Fedora audio matrix after the verified fixture correction, covered by `ff6a042`.
- [x] Rust MPRIS discovery, metadata, commands, owner replacement, late property export, session-bus restart, C ownership, Sway/Niri tray reconstruction, workspace tests, formatting, Clippy and linked application checks.
- [x] Native/Fedora package matrix for Rust MPRIS, covered by `ff6a042`.
- [x] Clean native MPRIS application/schema checks at `06ac6aa`, including Sway/Niri components and media artwork ownership; artifact `.cache/rust-migration/artifacts/media-native-schemas` -> `/nix/store/9s70xg1iq4p2im8pp984q1f7394ibz9k-way-shell-native-schemas-check`.
- [x] Rust notification IDs, validation, D-Bus ownership/recovery, expiration, actions, C interoperability and replacement presentation; real Sway/Niri widgets, controller AddressSanitizer tests, workspace tests, formatting, Clippy and linked application checks.
- [x] Native/Fedora package matrix for Rust notifications (`ff6a042`): both offline builds, installation, schemas, loading, libraries and uninstall checks pass. Artifact `.cache/rust-migration/artifacts/notifications-rpms` -> `/nix/store/cblvnd4jj2wjxq03kv48h9k8qlgmybs2-way-shell-rpms`; Fedora 43 retains PPD and Fedora 44 retains TuneD throughout installation and removal.
- [x] Clean native notification application/schema checks at `ff6a042`, including Sway/Niri components; artifact `.cache/rust-migration/artifacts/notifications-native-schemas` -> `/nix/store/spb53v4cfmn57wzqy8gbpmwgg59lhq35-way-shell-native-schemas-check`.
- [x] Rust tray/menu services and adapter: owned item/menu snapshots, unique registration, malformed images/layouts, cancellation, restart, deadlines, queued callbacks and disposal; full workspace tests, formatting, Clippy, clean linked build and existing tray widgets on Sway/Niri pass.
- [x] Native/Fedora package matrix for the Rust tray and menu cutover, covered by the panel checkpoint below.
- [x] Clean native tray application/schema checks at `6b79c1d`, including Sway/Niri components; artifact `.cache/rust-migration/artifacts/tray-native-schemas` -> `/nix/store/l9380vg0grsd4al26b8y3pgmv9z08b88-way-shell-native-schemas-check`.
- [x] Reproduce and correct Fedora guest image-loader configuration discovery: inherited Nix data paths hid installed Glycin loaders; a Rust probe in the exact Fedora 43 image fails before and decodes/encodes PNG after restoring Fedora data paths. Full RPM rerun remains pending.
- [x] Rust window foundation: transition/popup-policy tests, interrupted animations, underlay ownership, repeated destruction, GTK focus chain and theme cycles on Sway/Niri; real Sway output removal and GDK invalidation on both.
- [x] Rust panels: clock/DND, workspace identity, status updates, tray menus, output removal, nested notifications and retained-widget cleanup pass on Sway/Niri; workspace tests, formatting, Clippy and a clean linked application build pass.
- [x] Native/Fedora package matrix for the Rust panel cutover: both `b297300` offline Fedora builds and fresh-VM installation/schema/library/uninstall checks pass. Artifact `.cache/rust-migration/artifacts/panel-rpms` -> `/nix/store/gkqd4fkmygrn79fdmq1x3hr7d27h9m79-way-shell-rpms`; Fedora 43 retains PPD and Fedora 44 retains TuneD. Native acceptance passes through `bb4f612` below.
- [x] Reproduce native GTK schema-source failure with empty data paths; supplying the private compiled schema source passes the Rust tray probe on Sway and Niri with fatal warnings enabled. Native checks now also provide a font configuration.
- [x] Rust shared/application switchers: selection, filtering, activation, shortcut ownership, nested updates, reopening and disposal pass on Sway/Niri; full workspace, formatting, Clippy and clean linked application checks pass.
- [x] Rust workspace/output/rename views: full-width identifiers, literal names, raw move-window mode, failed dispatch, reordering and owned destruction pass; affected Rust/bridge tests, Clippy, clean application build and real Sway/Niri probes pass.
- [x] Native/Fedora package matrix for the Rust switcher cutover, covered by `4c9bc21`.
- [x] Rust activities: private desktop discovery, search/navigation, actual launching, file icons, inventory changes, failed launches, interrupted animations and disposal pass on Sway/Niri; Rust adapter tests, formatting, Clippy, clean linked build and existing checks pass.
- [x] Native/Fedora package matrix for the Rust activities cutover, covered by `4c9bc21`.
- [x] Reproduce and fix stopping audio from a native volume callback: direct Rust regression, inventory/restart and routing checks pass; refreshes are delivered after native dispatch returns.
- [x] Permanent Rust quick-settings system widgets and window lifecycle: focused tests, strict Clippy and private-service GTK probes pass on Sway/Niri. Runtime composition with network/audio remains pending.
- [x] Permanent Rust network controls: three focused tests, strict workspace Clippy and private libnm fixtures on Sway/Niri pass, including radio state, saved Wi-Fi, VPN/WireGuard, cancellation/restart and password-log checks. Runtime composition remains pending.
- [x] Clean native package/schema checks through activities (`bb4f612`), covering the panel, all switchers and the corrected schema/font environment. Artifacts `.cache/rust-migration/artifacts/activities-native` -> `/nix/store/3cvw4yhjbmllbk63a3lg9gc01mw680pl-way-shell-native-package-check` and `activities-native-1` -> `/nix/store/8m5p34m110pb36g1mild8g0nyvn124my-way-shell-native-schemas-check`.
- [x] Permanent Rust confirmation dialog: strict Clippy and real Sway/Niri ownership/cancellation/reentrancy checks pass. Runtime cutover joins quick settings; level OSD work remains in step 41.
- [x] Quick-settings runtime cutover: system, network, audio and confirmation dialog now run in Rust. Full workspace tests, strict Clippy, clean linked build, existing tests and composed Sway/Niri probes pass; 56 replaced C sources/headers and five obsolete C widget tests are removed.
- [x] Clean native package/schema checks for quick settings and the confirmation dialog (`4c9bc21`). Artifacts `.cache/rust-migration/artifacts/quick-settings-native` -> `/nix/store/1s7q3ci267vvnxln7dc1j2fv14lljhd8-way-shell-native-package-check` and `quick-settings-native-1` -> `/nix/store/z3bjhn6lmis94f7x3gmzdvy8fl3r1kv4-way-shell-native-schemas-check`.
- [x] Fedora package matrix through quick settings (`4c9bc21`): both offline builds and fresh-VM installation/schema/library/uninstall checks pass. Artifact `.cache/rust-migration/artifacts/quick-settings-rpms` -> `/nix/store/nyx32r3wssqicvg4mkdd5b2yszmli2mv-way-shell-rpms`; both selected power providers are retained.
- [x] Rust message tray: notification history/actions/DND, popup replacement/expiry, calendar and media cards, fade/underlay mediation and retained-widget cleanup pass on Sway/Niri. Full workspace, strict Clippy and clean linked build pass; eleven replaced C sources/headers and four obsolete C tests are removed.
- [ ] Native/Fedora package matrix for the Rust message-tray cutover.
- [x] Native package/schema checks through the message tray, level OSD and Rust schema helper (`5de7eee`). Artifacts `.cache/rust-migration/artifacts/message-tray-native` -> `/nix/store/dplkyphrg46sb6lj7k2zbqzzqx7n8323-way-shell-native-package-check` and `message-tray-native-1` -> `/nix/store/1yjmgjssc7i2d33rjxhp409ghck60f8l-way-shell-native-schemas-check`.
- [x] Rust level OSD is active with the existing confirmation dialog. Audio/brightness updates, default-device suppression, animation/timer cancellation, removed devices, reentrancy and cleanup pass on Sway/Niri; affected Rust tests, strict Clippy and linked build pass.
- [x] Permanent Rust IPC server: five real-socket tests cover all 33 actions, malformed datagrams, replies, deadlines, concurrency, socket ownership and immediate cancellation. Normal workspace integration and strict Clippy pass; application activation joins step 44.
- [x] Rust schema helper: all application schemas/keys, missing IDs, wrong backend and exit statuses pass. Native and Fedora checks now consume separately compiled helper artifacts; native derivations evaluate and packaging syntax passes.
- [ ] Native/Fedora installed-environment checks using the new Rust schema artifacts.
- [x] Reproduce and repair bundled GTK CSS parsing and negative panel-scrollbar sizing. The Rust theme parser regression and complete runtime checks pass on both compositors with fatal warnings enabled.
- [x] Rust application and IPC activation: normal debug/release binaries, explicit backend selection, real volume acknowledgement, network snapshot lifetime, strict Clippy and full runtime checks pass. Actual release executables pass private Sway/Niri smoke and installation staging; the C entry point/server and obsolete IPC test are removed.
- [ ] Native/Fedora package matrix for the Rust application entry point.
- [x] Cargo-only workspace build, full Rust tests, formatting and strict Clippy pass. The three permanent crates replace the temporary bridge; all remaining project C, C tests and Makefiles are removed. The shared POSIX installer passes staging checks. The combined tests also reproduced and fixed an audio fixture directory collision between separately included helper modules.
- [x] Trimmed native dependencies compile and pass the affected Rust/GTK checks. Regenerated Fedora 43/44 locks validate both complete images with Sway/Niri and ordinary-user smoke tools; conditional image requirements remain explicit. Evidence: `final-fedora-lock-update.log`.
- [x] Retained compositor handles stop immediately during shutdown, queued events cannot restart them, and closed Wayland peers fail startup. Regression, reconnection, strict Clippy, full runtime and application smoke checks pass on both compositors (`a09a4ab`).
- [x] Installed-package Sway/Niri smoke checks are wired into native Nix and both Fedora installation VMs. Rust helpers stay outside the application payload; Fedora uses an ordinary guest user and preserves logs. Parser tests, actual binary smoke, shell checks and derivation evaluation pass; execution of the final installed artifacts remains in the matrix below.
- [ ] Final clean Cargo build, tests, formatting, and Clippy.
- [ ] Final native Nix and offline Fedora 43/44 build/install/uninstall checks.
- [ ] Sway and Niri runtime checks, including real hardware and hotplug.
- [ ] Verify final Fedora 44 x86_64 RPM metadata and tuned-ppd compatibility.
- [ ] Install the completed migration RPM through polkit.
- [ ] Activate and verify the installed shell without a duplicate instance.

The laptop package has not been changed. Intermediate RPMs remain release 1 and
are verification artifacts, not the final upgrade.

## Current work

The active checkout is the main repository at
`/home/reubena/Documents/github/way-shell` on `rust-migration`, moved at the
user's request so VS Code shows the edits and branch history. The Bluetooth
branch is preserved. The former cache worktree is detached and inactive.

Rust owns Wayland, power, networking and the complete audio service, including
volume and PulseAudio stream routing. MPRIS and notifications now run in Rust;
the tray watcher, items, menus and all panel widgets also run in Rust.
All switchers, activities, quick settings, the confirmation dialog and the
complete message tray, level OSD, command server and application entry point now
run in Rust. Cargo owns application compilation and tests; the temporary bridge,
project C and Makefiles are removed. Final packaging checks are underway.
Package and laptop acceptance remain separate gates.

Following the user's updated direction, new migration tests are written in
Rust. The final obsolete C checks are removed with the unused adapters after
their Rust replacements pass. The tray and menu services switched together
after their Rust integration checks, avoiding a temporary C menu adapter.
