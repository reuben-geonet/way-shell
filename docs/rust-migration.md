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

The Sway stream fixture reproduced incorrect partial reads and writes: the C
loops advanced a byte count instead of the buffer pointer, and could spin on
EOF. Advance the buffer by the completed bytes, retry interrupted calls, and
stop on EOF or a zero-length write. Deterministic two-byte operations cover
both directions before transferring this contract to Rust.

The Rust Sway backend uses bounded, nonblocking GIO streams, preserves partial
reads/writes and 64-bit workspace identifiers, and clears stale state before
reconnecting. An idle established socket has no connection deadline. Private
socket fixtures cover those contracts, settings updates and owner destruction.
Headless Sway verifies focus, rename, output hotplug and workspace movement.
Literal names preserve quotes, backslashes, dollar signs and command separators;
Sway IPC quote removal differs from shell escaping. Urgency clearing no longer
requests focus. C retains only its temporary signal/vtable adapter and owned
snapshots until the widgets migrate.

Niri fixtures reproduced three independent baseline problems: named workspaces
lost their index and swapped focus/visibility, output discovery treated the
`Ok` response envelope as an output, and names starting with digits or containing
quotes produced invalid action JSON. The C fixes preserve every workspace's
index, map active to visible and focused to focused, unwrap `Ok.Outputs`, and
encode strings with json-glib. Only complete integers in 1..255 become index
references. Rust transfers the same fixtures and adds stable-ID references.

The power-profile baseline fixture reproduces a missing profile-list update:
`notify::profiles` called the active-profile handler. Correct the callback,
retain the proxy's borrowed variant, own each profile string once, and disconnect
the proxy before releasing the service. The regression also verifies that a
released service receives no subsequent property callbacks.

Profile-list changes also replaced the menu's outer container while its
revealer retained the original, leaving the visible menu stale. A GTK fixture
reproduces the changed container identity; update its children in place, release
old option data and disconnect the service on disposal. Native headless checks
cover inventory replacement, click routing, disappearance and repopulation.

The Wayland baseline's foreign-toplevel state loop used a byte length as an
element count. A single activated-state event reproduced a heap-buffer overflow
under AddressSanitizer. Bound iteration by the number of complete 32-bit states
and reject malformed lengths before replacing this protocol implementation.

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

## Niri migration verification

The Rust backend owns separate asynchronous command/event streams, preserves
64-bit workspace IDs, applies workspace events atomically and refreshes output
inventory on topology changes. Rejected actions leave the connection usable;
malformed replies close both streams and reconnect. C widgets retain owned
snapshots through the shared adapter. The old Niri client and C fixture are
removed after their Rust fixtures pass.

The private Niri compositor check verifies literal names, focus, stable IDs,
regular GTK window movement and cleanup. Layer-window/theme cycles also pass
on Niri. Both native checks and nix develop provide Niri and the explicit Mesa
EGL provider needed for software rendering. Real hardware and final package
verification remain separate gates.

## Wayland and power service verification

The Rust Wayland connection owns the registry, outputs, seats, foreign-toplevel
and gamma objects together. Raw protocol fixtures verify capped versions,
fragmentation, malformed state arrays, 30,000 queued actions under backpressure,
and exact gamma data transferred through a descriptor. Sway and Niri exercise
window discovery, activation, title/filter changes, shortcut inhibition and
repeated destruction; Sway also exercises output removal. Gamma controls become
unavailable when the protocol is absent or every output rejects it. Cargo's
protocol crates replace the last C Wayland generator consumers.

UPower uses owned application snapshots and a temporary UpDevice mirror for C
widgets. Tests cover non-battery devices, invalid readings, hotplug, failed
property requests, service restart and cancellation. The 90-percent icon
boundary is an independent baseline regression fix. Battery and power-menu
construction exposed missing GObject headers; separate GTK regressions cover
construction, updates and callback disconnection before the Rust service ports.

Logind validates the current session's UID, owns inhibitor descriptors, cancels
outstanding requests when the service/session changes and waits for actual
operation results. Baseline regressions cover selecting the user's session,
applying the initial inhibitor setting, denied requests, invalid descriptor
indices and descriptor cleanup. Power controls observe capability changes.

Power-profile fixtures cover both provider contracts, inventory/property changes,
failed writes, restart and cancellation. C menu updates preserve the displayed
container. Optional services leave subscribed controls disabled until recovery.

Brightness reads observed sysfs values, queues asynchronous logind writes, and
refreshes the device after completion. It never reports an unconfirmed value as
observed state. Fixtures cover validation, write failure, cancellation, device
removal/replacement, settings changes and callback cleanup. Separate C fixes
preserve readings after a failed write and advance at least one unit on a small
numeric range. GTK checks cover unavailable keyboard controls, changing ranges,
failed-write rollback and widgets outliving their controller. The final Rust IPC
server must await these operation results; the temporary C brightness commands
retain their existing immediate acknowledgement until step 42.

All of these local checks pass through `767efca`. Package-matrix and real hardware
acceptance are tracked independently in migration-checklist.md.

## Network inventory verification

Rust owns libnm initialization, device inventory, the primary device, radio
state and networking enable requests. The service publishes owned snapshots
without exposing native pointers to permanent application interfaces. The
remaining C connection facade receives retained objects through the temporary
bridge. It clears stale VPN records when the daemon disappears and reconnects
its subscriptions when the Rust inventory becomes available again.

The private D-Bus fixture exercises the real libnm cache with device state
changes, an empty primary-device list, removal, radio changes, rejected writes,
caller cancellation, a two-second response deadline, daemon restart, closed
connections and repeated initialization/destruction. Device fixtures emit both
State and StateReason, matching NetworkManager's paired state properties.
Separate bridge and C facade checks cover retained array entries, callback
teardown, delayed startup and VPN removal signal lifetimes. Local Cargo tests,
Clippy, the linked application and remaining C tests pass. Offline Fedora and
native package checks remain recorded separately in migration-checklist.md.

A C baseline regression reproduced Wi-Fi passwords appearing in debug output,
including calls rejected for missing devices. Both password-bearing messages
are removed before the connection port; diagnostics must never include supplied
credentials. The fixture uses a dummy value and captures log messages locally.

The connection baseline also reproduced device mix-ups between concurrent Wi-Fi
joins, an incorrect async finish function for saved connections, and uncancelled
requests after owner destruction. Each C request now retains its own client and
device, holds a weak service reference, and is cancelled on daemon or owner loss.
Completion calls the matching libnm finish function and releases returned objects.
Fixtures cover both new and saved joins, denied writes, concurrent devices,
credential-free diagnostics, cancellation and result ownership before the Rust
replacement takes over this behavior.

## Network connection verification

Connection operations now run through Rust using libnm's owning D-Bus connection.
Owned snapshots include access points, raw SSID bytes, saved profiles and active
connections. Requests copy settings before applying a supplied password. An
unchanged saved profile activates without an unnecessary settings write. A single
cancellation handle covers settings persistence and activation; failed writes do
not activate the profile or change the observed cache. Expected reply signatures
are checked and each D-Bus call has a two-second deadline.

The private real-libnm fixture covers new and saved Wi-Fi, scanning, disconnects,
non-UTF-8 SSIDs, rejected updates, concurrent devices, cancellation at each stage,
VPN/WireGuard, duplicate display names, live property changes, removal, daemon
restart and service destruction. A bridge fixture drops the C caller's strings
before processing the request to verify that the Rust operation owns its inputs.
The earlier C operation fixture is retired; C facade ownership tests remain until
the corresponding widgets move to Rust. C widgets still identify VPN profiles by
display name, preserving their existing behavior; permanent Rust APIs identify
profiles by object path. The network UI port must use those stable identifiers.

The workspace tests, bridge tests, Clippy and linked application pass locally.
Native and Fedora package checks for each checkpoint are recorded separately.
## Audio inventory baseline corrections

Before replacing object tracking, the C fixture reproduced a removed capture
device remaining in the database when no playback device existed, a dangling
default-device pointer, and incomplete cleanup of copied node/stream names.
The source-array guard and default references are corrected, all copied names
are released, and transient WirePlumber iterators, lookup references, mixer
variants and the pruning set use scoped ownership. Three regression cases and
the linked application with the remaining C suite pass. Evidence is recorded in
`audio-inventory-before.log`, `audio-default-before.log`, `audio-names-before.log`
and `audio-inventory-after.log` under the migration cache's `evidence` directory.
These checks use an empty object manager and do not touch desktop audio.

The mixer baseline regression also reproduces a numeric `step` being read with
GVariant's boolean format. It now reads a double, and missing mixer data produces
safe defaults while node identity and names remain inventoried. The regression
fails with the original format and passes with the correction; the linked build
and C suite also pass (`audio-mixer-before.log`, `audio-mixer-after.log`).

Audio control baseline fixtures reproduce an invalid widget access during
construction, initialization from the wrong default device, retained pointers to
removed devices, and callbacks surviving their controller. The controls now read
the correct initial source, clear and disable unavailable devices, recover when
they return, and use weak GObject callback ownership. The regression passes under
both Sway and Niri, including widgets retained after controller destruction.
Native package checks now include this fixture. Evidence is in the
`audio-widgets-*.log` files; the main checkout's linked build and C checks pass in
`audio-static-bridge-build.log`.

A stream mute canary test reproduces the C control writing `last_volume` beyond
an application stream's shorter record. Mute and unmute now send only the mixer
mute flag; WirePlumber retains the volume independently. The C canary regression,
Rust adapter ownership tests, private PipeWire volume/mute inventory and workspace
Clippy pass (`audio-stream-mute-before.log`, `audio-stream-and-adapter-checks.log`).


## Rust audio inventory verification

WirePlumber connection setup, plugin activation, object inventory, default-node
tracking, mixer notifications and reconnection now run in Rust. Widgets receive
owned device, stream, port, link and volume snapshots. Native proxy objects stay
inside the service. Callbacks use weak service references and disconnect before
connection disposal; pending initialization and retries are cancelled on stop.
The mixer scale is set through its actual enum type, and missing or invalid
values disable the affected volume control until valid data arrives.

The temporary adapter keeps stable C record addresses for existing objects and
retains removed records through synchronous notifications so C widgets can clear
their pointers. Rust and C checks agree on the record layout. Default changes,
volume OSD signals and microphone activity are distinct; removal does not produce
a volume OSD. The C layer now contains only volume and PulseAudio routing controls.
It obtains a scoped mixer reference and drops its routing context when audio
becomes unavailable. The remaining shared routing-request fields move to owned
Rust requests in step 26. Failed PulseAudio list queries also return safely.

A private PipeWire daemon exercises devices/streams, port/channel tracking,
metadata default changes, real volume/mute notifications, links, microphone
activity, removal, restart and repeated descriptor/ownership cleanup. A second
live fixture verifies the C-facing adapter through disconnection and recovery.
The C inventory regressions transfer to these Rust checks; the temporary C mute
and routing error regressions remain until their controls migrate. Workspace
tests, Clippy and the linked application pass. Evidence:
`audio-integrated-build.log` and `audio-inventory-final-checks.log` in
`.cache/rust-migration/evidence`. Package checks are tracked separately.

## Concurrent routing baseline fix

Two overlapping C playback routing requests previously shared their destination
and stream IDs, so only the second move happened. Each query now owns its IDs,
destination name and native operation. Completion releases that operation once;
owner loss or a failed/terminated PulseAudio connection cancels pending queries
before releasing their callback data. Missing property lists are ignored safely.
The regression covers concurrent destinations, normal completion, cancellation
and both terminal connection states. The linked application and baseline checks
pass; evidence is `audio-route-concurrency-before.log`,
`audio-route-concurrency-resumed.log` and `audio-route-concurrency-reviewed.log`.

The volume-up action also now caps its final five-percent increment at full
volume. A C mixer fixture reproduced a request for 103% from a 98% starting
level. Its regression checks ordinary increments, the final partial increment,
and no request when already full; all five C audio tests pass. Evidence:
`audio-volume-c-clamp-regression-before.log` and
`audio-volume-c-clamp-regression-after.log`. Rust control tests carry this case
forward alongside finite-value validation and mute/channel checks.

The clean native build of `e9a3a5f` exposed duplicate symbols between the IPC
fixture's audio replacements and the Rust static archive. Archive object grouping
varies with build settings, so the fixture now gives its replacements private
names before including the dispatcher. The IPC regressions pass with the linked
Rust bridge (`ipc-fixture-symbols.log`). The original native failure is recorded
in `audio-inventory-native-resumed.log`; that package gate awaits a corrected run.

The full Fedora 43/44 network connection matrix at `0cb7076` passed after
resumption, including offline builds, installation/uninstall, schema and loader
checks, and preservation of the selected power-profile providers. Artifact:
`.cache/rust-migration/artifacts/network-connections-rpms` resolves to
`/nix/store/h0b9569m0w2kmfxynh6pvmxnsdl5g29h-way-shell-rpms`.
Build and installation evidence is in `network-connections-rpm-resumed.log` and
the artifact's `logs` directory.

## Rust audio controls and routing

Volume, mute and stream routing now use application-owned IDs and results in
Rust. Mixer operations retain the plugin while releasing service state borrows
before emitting native signals. Mute preserves each channel's volume. Master
volume retains the existing WirePlumber channel-equalization behavior. Explicit
volume settings still accept finite values from zero to one; externally
amplified levels keep their existing incremental behavior (volume-up leaves
them alone, and volume-down decreases them by five percent).

PulseAudio routing uses the upstream `libpulse-sys` 1.23.0 and
`libpulse-mainloop-glib-sys` 1.22.0 declarations behind private owning wrappers.
They use GLib 0.21 on the existing application context. Each future owns its
query and move callback data, cancels native operations before freeing that
data, and reports success only after the server acknowledges the move. Requests
check immutable PipeWire serials and current endpoint identities, expire after
two seconds, cancel on owner loss, and recover after server restart or a stalled
handshake. The accepted WirePlumber binding and system daemon remain in use.

The temporary Rust adapter preserves the C API and record layout while rejecting
unowned record addresses without dereferencing them. The C audio implementation,
mixer-object escape hatch and C audio-control test helper are removed. Their
regressions now run in Rust. Private PipeWire, PulseAudio and hardware-disabled
WirePlumber policy instances verify playback/capture destinations, concurrent
requests, native errors, removal, cancellation, reconnection and deadlines.
Fixtures isolate configuration, state and D-Bus and reap their subprocesses.

The Fedora build closures now include `pipewire-pulseaudio` for these tests.
Both regenerated dependency images validate; each lock adds only that RPM.
Evidence is in `audio-controls-fedora-locks-resumed.log`,
`audio-controls-cutover.log`, `audio-routing-policy-verified.log` and
`audio-amplified-volume-{before,after}.log`. Full workspace tests, formatting,
Clippy and the linked application pass in `audio-controls-final-workspace.log`.
Clean native and Fedora package results are tracked separately in the checklist.

The first clean native audio-control run found the policy executable missing
from `PATH`: host library inputs do not supply build-time tools under Nix's
strict dependency handling. WirePlumber is now an explicit native check input
and RPM build requirement. The Fedora locked closures already contain it.
The failure is retained in `audio-controls-native-build.log`.

## MPRIS metadata baseline fixes

Metadata replacement now clears omitted fields, accepts empty artist arrays,
ignores fields with unexpected types and handles an absent or invalid metadata
container. Strings are copied into the player record and variant references are
released correctly. The property notification handler no longer unreferences
the generated getter's borrowed value. This fixes stale album/art/artist data,
malformed-metadata crashes and retained metadata storage before the Rust port.

Six GLib regressions cover valid multi-artist data, replacement, empty artists,
wrong types, missing metadata and ownership. They are part of the default test
suite. Evidence: `media-player-metadata-*-before.log`,
`media-player-metadata-after.log` and `media-metadata-integrated.log`.

MPRIS startup now discovers players that already own a bus name, deferring the
initial notifications until widgets have subscribed. Atomic owner replacement
removes the previous record and discovers its successor. Duplicate discovery and
unrelated name prefixes are ignored. Disposal cancels initial discovery,
unsubscribes from the bus and disconnects proxy callbacks; removed records stay
alive through their removal signal and are then released.

Three tests use real generated MPRIS proxies and skeletons on a private D-Bus to
verify startup discovery, replacement and destruction before/after discovery.
The six metadata tests remain green. Evidence is in
`media-discovery-{existing,replacement}-before.log`,
`media-discovery-after.log` and `media-discovery-integrated.log`.
