# Bluetooth Rust port

Reference: `bluetooth` at `017b61973888d5902f6a0b6d60b6f13d81c4b1b3`.
Destination: `rust-migration`, following `e97137f`.
The 22 original C scenarios passed on an isolated bus before implementation.
The reference branch is preserved; source and new tests are Rust.

The service uses asynchronous GIO BlueZ calls, owned rfkill descriptors, typed
operations and immutable device snapshots. Like the shell, the tests use the
default GLib context: GIO's asynchronous ObjectManager initializer uses a worker
whose signal subscriptions are attached to that context. The contract test runs
its scenarios on one thread, with a fresh private bus and radio for each case.

## Reference scenario mapping

All names below belong to `crates/shell/tests/bluetooth.rs`, run by
`bluetooth_reference_and_lifecycle_contract`.

| Original `/bluetooth/` scenario | Rust scenario |
| --- | --- |
| devices | devices |
| connection | connection |
| airplane | airplane |
| external-override | external_override |
| power-cycles | power_cycles |
| power-busy | power_busy |
| power-blocked | power_blocked |
| delayed-adapter | delayed_adapter |
| radio | radio_blocks |
| owner-and-removal | owner_and_removal |
| removed-operation | removed_operation |
| completed-operation-error | completed_operation_error |
| late-connect | late_connect |
| late-disconnect | late_disconnect |
| external-power | external_power |
| power-timeout | power_timeout |
| airplane-reversal | airplane_reversal |
| power-off-cancels-connection | power_off_cancels_connection |
| external-off-cancels-connection | external_off_cancels_connection |
| settings | settings |
| connected-without-profiles | connected_without_profiles |
| adapter-hotplug | adapter_hotplug |

Additional cases cover every transient power error, retry exhaustion, replacement
at identical object paths, mixed adapter restoration, rfkill write failure,
radio blocking during connection, pending-operation shutdown, retry cancellation,
shutdown from an error callback, and Airplane Mode before BlueZ initialization.

## UI and packaging acceptance

The `quick-settings-bluetooth-compat` Rust probe exercises both themes under
Sway and Niri. It checks subtitle changes, row identity and ordering, device
spinners, power icons and reversals, single-row sizing, scrolling bounds,
long-name/error wrapping, recovery, manager launch, daemon restart and hotplug.
The composed quick-settings probe also routes the Bluetooth tile to the shared
service and verifies that retained buttons stop acting after window destruction.

Native package checks run the new UI probe on both compositors. Fedora's offline
Cargo checks include the service contract, and installed-package checks exercise
the complete application under both compositors. RPM release is `0.0.10-10`.

Delivery is source and verified artifacts only. The laptop's installed shell and
radio state are unchanged. Isolated fixtures do not verify real-radio behavior.

Verified locally on 2026-09-16: all 32 Bluetooth scenarios, the workspace tests,
formatting and Clippy with warnings denied. Both Bluetooth and composed-popup
probes passed on Sway and Niri. Layout captures are in
`.cache/bluetooth/captures/`; logs use `.cache/bluetooth-*.log`.

The complete native and Fedora 43/44 package matrix passed. Artifact paths,
digests and installation-check results are recorded in the
[migration checklist](migration-checklist.md#bluetooth-follow-on).
