# Sourced by vmTools.runInLinuxImage, with Fedora tools on PATH.
set -euo pipefail
: "${out:?vmTools must provide the output directory}"
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
mkdir -p "$out/logs"
exec 3>&1 4>&2
exec > >(tee "$out/logs/check.log") 2>&1
log_pid=$!
report_status() {
    local status=$?
    trap - EXIT
    set +x
    echo "Fedora installation check exit status: $status"
    exec 1>&3 2>&4 3>&- 4>&-
    wait "$log_pid"
    exit "$status"
}
trap report_status EXIT
set -x

unset GSETTINGS_SCHEMA_DIR GIO_EXTRA_MODULES GIO_MODULE_DIR LD_LIBRARY_PATH LD_PRELOAD
export XDG_DATA_DIRS=/usr/local/share:/usr/share
export HOME=/tmp/check-home XDG_DATA_HOME=/tmp/check-data GSETTINGS_BACKEND=memory
mkdir -p "$HOME" "$XDG_DATA_HOME"

application=()
for artifact in "$RPM_DIRECTORY"/*.rpm; do
    case "$artifact" in *.src.rpm|*.nosrc.rpm) continue ;; esac
    if [ "$(rpm -qp --qf '%{NAME}' "$artifact")" = way-shell ] &&
        [ "$(rpm -qp --qf '%{ARCH}' "$artifact")" = x86_64 ]; then
        application+=("$artifact")
    fi
done
test "${#application[@]}" -eq 1
rpm -q --qf '%{NEVRA}\n' "$POWER_PROVIDER" > "$out/logs/power-provider-before.txt"
case "$POWER_PROVIDER" in
    power-profiles-daemon) other_provider=tuned-ppd ;;
    tuned-ppd) other_provider=power-profiles-daemon ;;
    *) echo "Unknown power provider: $POWER_PROVIDER" >&2; exit 1 ;;
esac
if rpm -q "$other_provider"; then exit 1; fi
rpm -qp --requires "${application[0]}" | grep -Fx '(power-profiles-daemon or tuned-ppd)'
rpm -Uvh "${application[0]}"
rpm -q --qf '%{NEVRA}\n' "$POWER_PROVIDER" > "$out/logs/power-provider-after.txt"
cmp "$out/logs/power-provider-before.txt" "$out/logs/power-provider-after.txt"
if rpm -q "$other_provider"; then exit 1; fi
rpm -q way-shell
# RPMs also co-own shared directories such as /usr/lib/.build-id. Check all
# files and symlinks; those shared directories may legitimately remain.
rpm -q --qf '[%{FILEMODES:perms} %{FILENAMES}\n]' way-shell \
    | sed '/^d/d;s/^[^ ]* //' > "$out/logs/owned-files.txt"
test -x /usr/bin/way-shell
test -x /usr/bin/way-sh
grep -Fx 'ExecStart=/usr/bin/way-shell' /usr/lib/systemd/user/way-shell.service
test -s /usr/share/licenses/way-shell/LICENSE
test -s /usr/share/glib-2.0/schemas/gschemas.compiled
/usr/bin/way-shell --help

# This artifact was compiled by the same Fedora compiler as the package.
# Keep test helpers outside the installed application payload.
test -x "$SCHEMA_PROBE"
test -x "$APPLICATION_SMOKE_HELPER"
test -r "$APPLICATION_SMOKE_SCRIPT"
if grep -E '/(schema-probe|application-smoke)$' "$out/logs/owned-files.txt"; then exit 1; fi
# Word splitting is intentional: the IDs are derived from our schema XML.
# shellcheck disable=SC2086
"$SCHEMA_PROBE" $EXPECTED_SCHEMAS
gsettings list-schemas > "$out/logs/schemas.txt"

for executable in /usr/bin/way-shell /usr/bin/way-sh "$SCHEMA_PROBE" "$APPLICATION_SMOKE_HELPER"; do
    executable_name=${executable##*/}
    readelf -l -d "$executable" > "$out/logs/$executable_name-elf.txt"
    grep -F 'Requesting program interpreter: /lib64/ld-linux-x86-64.so.2' "$out/logs/$executable_name-elf.txt"
    ldd "$executable" > "$out/logs/$executable_name-ldd.txt"
    if grep -E '/nix/store|not found' "$out/logs/$executable_name-elf.txt" "$out/logs/$executable_name-ldd.txt"; then
        exit 1
    fi
done

# Compositors run as an ordinary guest user. No PAM/logind session or root
# compositor exception is needed for private headless Sway and nested Niri.
test "$(id -u)" -eq 0
smoke_user=way-shell-smoke
smoke_root=$(mktemp -d /var/tmp/way-shell-installed.XXXXXX)
smoke_home="$smoke_root/home"
smoke_logs="$smoke_root/logs"
useradd --system --user-group --home-dir "$smoke_home" --no-create-home \
    --shell /bin/sh "$smoke_user"
mkdir -p "$smoke_home" "$smoke_logs"
chown -R "$smoke_user:$smoke_user" "$smoke_root"
test -s /etc/fonts/fonts.conf
test -s /usr/share/glvnd/egl_vendor.d/50_mesa.json
for backend in sway niri; do
    smoke_status=0
    if (
        cd "$smoke_home" || exit 1
        # env -i removes Nix's compiler, loader, schema, EGL and data overrides.
        # Retain only Fedora's installed data and fonts; the harness creates its
        # own memory settings, buses, runtime, configuration and writable data.
        exec /usr/bin/setpriv --reuid "$smoke_user" --regid "$smoke_user" --clear-groups \
            /usr/bin/env -i \
            PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
            HOME="$smoke_home" USER="$smoke_user" LOGNAME="$smoke_user" \
            LANG=C.UTF-8 FONTCONFIG_FILE=/etc/fonts/fonts.conf FONTCONFIG_PATH=/etc/fonts \
            WAY_SHELL_TEST_DATA_DIRS=/usr/local/share:/usr/share \
            WAY_SHELL_TEST_EGL_VENDOR=/usr/share/glvnd/egl_vendor.d/50_mesa.json \
            /bin/sh "$APPLICATION_SMOKE_SCRIPT" "$APPLICATION_SMOKE_HELPER" \
            /usr/bin/way-shell /usr/bin/way-sh /usr/share/glib-2.0/schemas \
            "$backend" "$smoke_logs/$backend"
    ); then
        :
    else
        smoke_status=$?
    fi
    # Copy as root without preserving the guest user's ownership, including on
    # failure so the verified-output logs retain the actual compositor trace.
    mkdir -p "$out/logs/application-$backend"
    if [ -d "$smoke_logs/$backend" ]; then
        cp -R "$smoke_logs/$backend/." "$out/logs/application-$backend/"
    fi
    if [ "$smoke_status" -ne 0 ]; then exit "$smoke_status"; fi
done
userdel "$smoke_user"
rm -rf "$smoke_root"

rpm -e way-shell
rpm -q --qf '%{NEVRA}\n' "$POWER_PROVIDER" > "$out/logs/power-provider-uninstalled.txt"
cmp "$out/logs/power-provider-before.txt" "$out/logs/power-provider-uninstalled.txt"
if rpm -q way-shell; then exit 1; fi
while IFS= read -r owned; do
    test ! -e "$owned"
    test ! -L "$owned"
done < "$out/logs/owned-files.txt"
echo 'Installation, schemas, libraries, Sway/Niri application smoke and removal passed.'
