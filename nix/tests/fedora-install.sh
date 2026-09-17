# Sourced in a disposable Fedora VM by vmTools.runInLinuxImage.
set -euo pipefail
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
mkdir -p "$out/logs"
exec > >(tee "$out/logs/check.log") 2>&1
set -x

# Check Fedora's installed schema paths without inheriting Nix overrides.
unset GSETTINGS_SCHEMA_DIR GIO_EXTRA_MODULES GIO_MODULE_DIR LD_LIBRARY_PATH LD_PRELOAD
export HOME=/tmp/way-shell-check XDG_DATA_HOME=/tmp/way-shell-check/data
export XDG_DATA_DIRS=/usr/local/share:/usr/share GSETTINGS_BACKEND=memory
mkdir -p "$HOME" "$XDG_DATA_HOME"

application=("$RPM_DIRECTORY"/way-shell-[0-9]*.x86_64.rpm)
test "${#application[@]}" -eq 1
rpm -Uvh "${application[0]}"
/usr/bin/way-shell --help
/usr/bin/way-sh --help
test -s /usr/lib/systemd/user/way-shell.service
test -s /usr/share/licenses/way-shell/LICENSE
gsettings list-recursively org.ldelossa.way-shell.system > "$out/logs/settings.txt"

rpm -e way-shell
for path in /usr/bin/way-shell /usr/bin/way-sh /usr/lib/systemd/user/way-shell.service \
    /usr/share/glib-2.0/schemas/org.ldelossa.way-shell.gschema.xml \
    /usr/share/licenses/way-shell; do
    test ! -e "$path"
done
gsettings list-schemas > "$out/logs/schemas-after-removal.txt"
if grep -E '^org\.ldelossa\.way-shell($|\.)' "$out/logs/schemas-after-removal.txt"; then
    echo 'Way-Shell schemas remain after removal' >&2
    exit 1
fi
echo 'Fedora installation, executable startup, schemas and removal passed.'
