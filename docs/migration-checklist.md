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
- [ ] 21. Migrate power profile management to Rust.
- [ ] 22. Migrate display and keyboard brightness to Rust.
- [ ] 23. Migrate network inventory and radio state to Rust.
- [ ] 24. Migrate network connection management to Rust.
- [ ] 25. Migrate WirePlumber object tracking to Rust.
- [ ] 26. Migrate volume and stream routing to Rust.
- [ ] 27. Migrate MPRIS integration to Rust.
- [ ] 28. Migrate the notification service to Rust.
- [ ] 29. Migrate tray watcher and item tracking to Rust.
- [ ] 30. Migrate D-Bus menu handling to Rust.
- [ ] 31. Introduce Rust window and visibility controllers.
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
- [ ] Native and Fedora package matrix for the theme port (`1955558`): native
      smoke test exposed a missing private D-Bus configuration; fixed in
      `a034a65`. Rerun with the Sway port to cover subsequent changes.
- [ ] Full matrix covering clock/Sway (`709d126`): interrupted for the requested reboot during Fedora compilation. Native application and Sway/theme smoke checks passed; the separate schema check and RPM matrix must rerun.
- [x] Niri development environment, private socket fixtures, real workspace/window actions and layer-window smoke checks.
- [ ] Final clean Cargo build, tests, formatting, and Clippy.
- [ ] Final native Nix and offline Fedora 43/44 build/install/uninstall checks.
- [ ] Sway and Niri runtime checks, including real hardware and hotplug.
- [ ] Verify final Fedora 44 x86_64 RPM metadata and tuned-ppd compatibility.
- [ ] Install the completed migration RPM through polkit.
- [ ] Activate and verify the installed shell without a duplicate instance.

The laptop package has not been changed. Intermediate RPMs remain release 1 and
are verification artifacts, not the final upgrade.

## Current work

Resumed from the durable checkpoint. Niri's C backend is replaced with the Rust
service and a thin C vtable adapter. The separate Wayland state-array, battery
90% icon and power-profile notification bugs are regression-tested commits.
Wayland, UPower and power-profile service integration is underway; logind and
brightness are being developed against fixtures. The full RPM matrix remains
a required acceptance gate, and the installed laptop shell remains unchanged.
