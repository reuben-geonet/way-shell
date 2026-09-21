#!/bin/sh
# Install already-built artifacts. Packaging owns schema compilation and wrapping.
set -eu

source_dir=$(CDPATH= cd "$(dirname "$0")/.." && pwd)
prefix=${PREFIX:-/usr}
destination=${DESTDIR:-}
binary_dir=${CARGO_ARTIFACT_DIR:-"$source_dir/target/release"}
bin_dir=${BINDIR:-"$prefix/bin"}
schema_dir=${SCHEMADIR:-"$prefix/share/glib-2.0/schemas"}
unit_dir=${USERUNITDIR:-"$prefix/lib/systemd/user"}
license_dir=${LICENSEDIR:-"$prefix/share/licenses/way-shell"}

for binary in way-shell way-sh; do
    test -x "$binary_dir/$binary"
done
if [ -n "${DEPENDENCY_LICENSES:-}" ]; then
    test -d "$DEPENDENCY_LICENSES"
fi

install -d "$destination$bin_dir" "$destination$schema_dir" \
    "$destination$unit_dir" "$destination$license_dir"
install -m 755 "$binary_dir/way-shell" "$destination$bin_dir/way-shell"
install -m 755 "$binary_dir/way-sh" "$destination$bin_dir/way-sh"
install -m 644 "$source_dir/data/org.ldelossa.way-shell.gschema.xml" \
    "$destination$schema_dir/org.ldelossa.way-shell.gschema.xml"
install -m 644 "$source_dir/contrib/systemd/way-shell.service" \
    "$destination$unit_dir/way-shell.service"
install -m 644 "$source_dir/LICENSE" "$destination$license_dir/LICENSE"
if [ -n "${DEPENDENCY_LICENSES:-}" ]; then
    install -d "$destination$license_dir/dependencies"
    cp -RL "$DEPENDENCY_LICENSES/." "$destination$license_dir/dependencies/"
fi
