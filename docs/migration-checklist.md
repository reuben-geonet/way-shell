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
- [ ] 32. Migrate panel indicators and workspaces to Rust.
- [ ] 33. Migrate shared and application switchers to Rust.
- [ ] 34. Migrate workspace, output, and rename switchers to Rust.
- [ ] 35. Migrate application discovery and launching to Rust.
- [ ] 36. Migrate quick settings layout and system controls to Rust.
- [ ] 37. Migrate network controls to Rust.
- [ ] 38. Migrate mixer controls to Rust.
- [ ] 39. Migrate notification presentation to Rust.
- [ ] 40. Migrate calendar and media presentation to Rust.
- [ ] 41. Migrate OSD and dialog overlays to Rust.
- [ ] 42. Replace the shell IPC server with Rust.
- [ ] 43. Replace the packaging schema probe with Rust.
- [ ] 44. Move startup and shutdown into Rust.
- [ ] 45. Make Cargo the sole application build tool; remove C and the bridge.
- [ ] 46. Remove obsolete build dependencies.
- [ ] 47. Add packaged Wayland smoke coverage.
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
- [ ] Native/Fedora package matrix for the Rust tray and menu cutover.
- [x] Rust window foundation: transition/popup-policy tests, interrupted animations, underlay ownership, repeated destruction, GTK focus chain and theme cycles on Sway/Niri; real Sway output removal and GDK invalidation on both.
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
volume and PulseAudio stream routing. The audio adapter preserves stable C
records for the remaining widgets. MPRIS and notifications now run in Rust;
the tray watcher, items and menus also run in Rust. Window and panel migration
is underway. Package and laptop acceptance remain separate gates.

Following the user's updated direction, new migration tests are written in
Rust. Existing C checks remain useful until their consumers migrate; no new
C test harnesses are added. The tray and menu services switched together
after their Rust integration checks, avoiding a temporary C menu adapter.
