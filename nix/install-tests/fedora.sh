# shellcheck shell=bash
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
mkdir -p /tmp/way-shell-repos
cat > /tmp/way-shell-repos/installation.repo <<EOF
[installation]
name=Pinned installation dependencies
baseurl=file://$INSTALL_REPOSITORY
enabled=1
gpgcheck=0
EOF
# The local test RPM is unsigned; dependency downloads are SHA-256 pinned by Nix.
dnf=(dnf5 --assumeyes --setopt=reposdir=/tmp/way-shell-repos)
"${dnf[@]}" install "${application[0]}"
/usr/bin/way-shell --help
/usr/bin/way-sh --help
gsettings list-recursively org.ldelossa.way-shell.system > "$out/logs/settings.txt"

"${dnf[@]}" remove way-shell
if rpm -q way-shell; then
    echo 'Way-Shell remains installed after removal' >&2
    exit 1
fi
echo 'Fedora installation, executable startup, schemas and removal passed.'
