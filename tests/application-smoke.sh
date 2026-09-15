#!/bin/sh
# Usage: application-smoke.sh HELPER WAY_SHELL WAY_SH INSTALLED_SCHEMA_DIR [sway|niri] [LOG_DIR]
# Uses the supplied packaged binaries and schemas, never build-tree payloads.
set -eu
helper=$(realpath "${1:?pass the compiled Rust smoke helper}")
application=$(realpath "${2:?pass the packaged way-shell executable}")
cli=$(realpath "${3:?pass the packaged way-sh executable}")
schemas=$(realpath "${4:?pass the installed GSettings schema directory}")
backend=${5:-sway}
case "$backend" in
    sway|niri) ;;
    *) echo "Unknown test compositor: $backend" >&2; exit 2 ;;
esac
for executable in "$helper" "$application" "$cli"; do
    if [ ! -x "$executable" ]; then
        echo "Missing executable: $executable" >&2
        exit 1
    fi
done
logs=${6:-$(mktemp -d /tmp/way-shell-smoke-logs.XXXXXX)}
mkdir -p "$logs"
logs=$(realpath "$logs")
test -f "$schemas/gschemas.compiled"
runtime=$(mktemp -d /tmp/way-shell-smoke.XXXXXX)
compositor_pid=
niri_pid=
bus_pid=
cleanup() {
    trap - EXIT HUP INT TERM
    for pid in "$niri_pid" "$compositor_pid" "$bus_pid"; do
        if [ -n "$pid" ]; then
            kill "$pid" 2>/dev/null || :
        fi
    done
    for pid in "$niri_pid" "$compositor_pid" "$bus_pid"; do
        if [ -n "$pid" ]; then
            wait "$pid" 2>/dev/null || :
        fi
    done
    rm -rf "$runtime"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
export XDG_RUNTIME_DIR="$runtime"
export XDG_CONFIG_HOME="$runtime/config" XDG_CACHE_HOME="$runtime/cache"
export XDG_DATA_HOME="$runtime/data"
# Package checks can retain installed loader/icon data without inheriting the
# caller's data paths. Configuration, writable data and buses remain private.
export XDG_DATA_DIRS="$runtime/data${WAY_SHELL_TEST_DATA_DIRS:+:$WAY_SHELL_TEST_DATA_DIRS}"
export GSETTINGS_SCHEMA_DIR="$schemas" GSETTINGS_BACKEND=memory
export GTK_A11Y=none GSK_RENDERER=cairo GDK_BACKEND=wayland
export PIPEWIRE_RUNTIME_DIR="$runtime" PIPEWIRE_REMOTE="$runtime/no-pipewire"
export PULSE_SERVER="unix:$runtime/no-pulse"
export XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP="$backend"
export WAY_SHELL_TEST_APPLICATION="$application" WAY_SHELL_TEST_CLI="$cli"
export WAY_SHELL_TEST_LOG_DIR="$logs" WAY_SHELL_TEST_BACKEND="$backend"
export WAY_SHELL_TEST_PRIVATE_RUNTIME="$runtime"
unset G_MESSAGES_DEBUG G_DEBUG SWAYSOCK NIRI_SOCKET WAYLAND_DISPLAY DISPLAY
unset DBUS_SESSION_BUS_ADDRESS DBUS_SYSTEM_BUS_ADDRESS
mkdir -p "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME"
# No activation directories: host optional services cannot be autostarted.
cat > "$runtime/bus.conf" <<EOF_BUS
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:path=$runtime/bus</listen><auth>EXTERNAL</auth>
<policy context="default"><allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/></policy></busconfig>
EOF_BUS
dbus-daemon --nofork --print-address=1 --config-file="$runtime/bus.conf" > "$runtime/bus-address" 2> "$logs/dbus.log" &
bus_pid=$!
attempt=0
until [ -s "$runtime/bus-address" ]; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 50 ] || ! kill -0 "$bus_pid" 2>/dev/null; then cat "$logs/dbus.log" >&2; exit 1; fi
    sleep 0.1
done
DBUS_SESSION_BUS_ADDRESS=$(head -n 1 "$runtime/bus-address")
export DBUS_SESSION_BUS_ADDRESS
export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS"
cat > "$runtime/sway.conf" <<'EOF_SWAY'
output * resolution 1280x800
seat seat0 fallback true
set $mod Mod4
EOF_SWAY
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
    sway -c "$runtime/sway.conf" > "$logs/sway.log" 2>&1 &
compositor_pid=$!
attempt=0
until [ -n "${WAYLAND_DISPLAY:-}" ] && [ -n "${SWAYSOCK:-}" ]; do
    for socket in "$runtime"/wayland-*; do
        if [ -S "$socket" ]; then WAYLAND_DISPLAY=$(basename "$socket"); break; fi
    done
    for socket in "$runtime"/sway-ipc.*.sock; do
        if [ -S "$socket" ]; then SWAYSOCK=$socket; break; fi
    done
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 100 ] || ! kill -0 "$compositor_pid" 2>/dev/null; then cat "$logs/sway.log" >&2; exit 1; fi
    sleep 0.1
done
export WAYLAND_DISPLAY SWAYSOCK
case "$backend" in
    sway) ;;
    niri)
        parent_display=$WAYLAND_DISPLAY
        if [ -n "${WAY_SHELL_TEST_EGL_VENDOR:-}" ]; then export __EGL_VENDOR_LIBRARY_FILENAMES="$WAY_SHELL_TEST_EGL_VENDOR"; fi
        cat > "$runtime/niri.kdl" <<'EOF_NIRI'
input { keyboard { xkb { layout "us"; }; }; }
animations { off; }
hotkey-overlay { skip-at-startup; }
xwayland-satellite { off; }
workspace "first"
workspace "second"
EOF_NIRI
        LIBGL_ALWAYS_SOFTWARE=1 niri -c "$runtime/niri.kdl" > "$logs/niri.log" 2>&1 &
        niri_pid=$!
        attempt=0
        until [ -n "${NIRI_SOCKET:-}" ] && [ "$WAYLAND_DISPLAY" != "$parent_display" ]; do
            for socket in "$runtime"/niri.*.sock; do
                if [ -S "$socket" ]; then NIRI_SOCKET=$socket; break; fi
            done
            for socket in "$runtime"/wayland-*; do
                if [ -S "$socket" ] && [ "$(basename "$socket")" != "$parent_display" ]; then WAYLAND_DISPLAY=$(basename "$socket"); break; fi
            done
            attempt=$((attempt + 1))
            if [ "$attempt" -ge 200 ] || ! kill -0 "$niri_pid" 2>/dev/null; then cat "$logs/niri.log" >&2; exit 1; fi
            sleep 0.1
        done
        export NIRI_SOCKET WAYLAND_DISPLAY
        ;;
esac
if "$helper" > "$logs/smoke.log" 2>&1; then
    cat "$logs/smoke.log"
else
    cat "$logs/smoke.log" >&2
    for log in "$logs"/application-*.log; do
        if [ -f "$log" ]; then tail -n 120 "$log" >&2; fi
    done
    echo "Application smoke logs: $logs" >&2
    exit 1
fi
