# Nix development and packages

This flake supports `x86_64-linux`. Install Nix with flakes enabled
(`experimental-features = nix-command flakes` in `nix.conf`). A multi-user
installation must have a running daemon; on a systemd host with the packaged
socket unit, enable it with `pkexec systemctl enable --now nix-daemon.socket`.
Check ordinary-user access with `nix store info --store daemon`.

```sh
nix develop                  # Tools, dependencies, GDB and local schemas
make -j1                     # Generated files currently require serial builds
./way-shell --help
nix build                    # Native Nix package, linked at result
nix build .#way-shell         # Explicit native package
nix profile add .#way-shell   # Install in your profile
```

The development shell compiles schemas into `.cache/nix/schemas` and sets
`GSETTINGS_SCHEMA_DIR`; re-enter after changing the schema XML. No system schema
installation is needed. The installed Nix executable has its own GTK/GLib
wrapper, schemas and dconf backend. Its systemd user unit invokes that wrapper.
To use the unit from a profile, link its `lib/systemd/user/way-shell.service`
into your user unit directory, then run `systemctl --user daemon-reload`.

Start a debugger from `nix develop` so it inherits the native libraries,
schemas and current session variables, for example `gdb ./way-shell`.
Local editor configuration belongs in the ignored `.vscode/` directory.
Do not commit user IDs, machine socket paths or remote debugger addresses.

Way-Shell needs a Sway/Wayland session, a session D-Bus, logind,
NetworkManager, PipeWire/WirePlumber, UPower and power-profiles-daemon running
on the host. Nix packaging supplies libraries, not these host services.
Persistent settings also require the dconf D-Bus service, normally supplied
by the host desktop.
Use the Sway configuration in the main README, including `way-sh` on PATH.
Other desktops such as GNOME may already own the notification and status
notifier D-Bus names Way-Shell uses. A successful `--help` check does not test
desktop startup or guarantee compatibility with those sessions.

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
Install the application RPM with `sudo dnf install /path/to/way-shell-*.x86_64.rpm`,
selecting the file without `debuginfo` or `debugsource` in its name. The native
RPM has no Nix runtime dependency. The CLI builds the invoked flake's source
snapshot, including tracked local edits, regardless of the caller's directory.
Add new files to Git so Nix includes them in a local flake.

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

Each generated `rpm-fedora-VERSION` check builds an RPM with Fedora tools,
installs it in a fresh copy of the cached image with dependency checks and
scriptlets enabled, checks all schemas and both executable loaders/libraries,
then removes it and checks that its registration and owned files disappeared.
Fedora tools use Fedora's standard data directories, without Nix schema
overrides. Guests have no repository network access. Builds and checks use
separate disposable copy-on-write disks.

These are Fedora userspaces under a Nix-provided VM kernel. The images include
build dependencies; minimal-desktop dependency completeness, a full Fedora
boot, graphics and interactive Sway behavior are not tested. Inputs and
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
