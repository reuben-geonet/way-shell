#!/bin/sh
# Isolated component smoke test; accepts a compiled Rust example as its argument.
set -eu
if [ "${WAY_SHELL_TEST_BUS:-}" != 1 ]; then
    exec dbus-run-session --config-file=tests/fixtures/session-bus.conf -- \
        env WAY_SHELL_TEST_BUS=1 sh "$0" "$@"
fi
probe=$(realpath "${1:?pass the compiled component probe}")
backend=${2:-sway}
runtime=$(mktemp -d /tmp/way-shell-wayland.XXXXXX)
compositor_pid=
niri_pid=
cleanup() {
    if [ -n "$niri_pid" ]; then
        kill "$niri_pid" 2>/dev/null || :
        wait "$niri_pid" 2>/dev/null || :
    fi
    if [ -n "$compositor_pid" ]; then
        kill "$compositor_pid" 2>/dev/null || :
        wait "$compositor_pid" 2>/dev/null || :
    fi
    rm -rf "$runtime"
}
trap cleanup EXIT HUP INT TERM
export XDG_RUNTIME_DIR="$runtime"
export GSETTINGS_BACKEND=memory
export GTK_A11Y=none
unset G_MESSAGES_DEBUG
unset SWAYSOCK NIRI_SOCKET WAYLAND_DISPLAY DISPLAY
cat > "$runtime/sway.conf" <<'EOF'
output * resolution 800x600
seat seat0 fallback true
set $mod Mod4
EOF
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
    sway -c "$runtime/sway.conf" > "$runtime/sway.log" 2>&1 &
compositor_pid=$!
attempt=0
until [ -n "${WAYLAND_DISPLAY:-}" ]; do
    for socket in "$runtime"/wayland-*; do
        if [ -S "$socket" ]; then WAYLAND_DISPLAY=$(basename "$socket"); break; fi
    done
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 50 ] || ! kill -0 "$compositor_pid" 2>/dev/null; then
        cat "$runtime/sway.log" >&2
        exit 1
    fi
    sleep 0.1
done
export WAYLAND_DISPLAY GDK_BACKEND=wayland
for socket in "$runtime"/sway-ipc.*.sock; do
    if [ -S "$socket" ]; then export SWAYSOCK="$socket"; break; fi
done
case "$backend" in
    sway) ;;
    niri)
        # Niri's supported nested backend runs inside the private headless
        # Sway. All commands and GTK surfaces below connect to Niri itself.
        parent_display=$WAYLAND_DISPLAY
        if [ -n "${WAY_SHELL_TEST_EGL_VENDOR:-}" ]; then
            export __EGL_VENDOR_LIBRARY_FILENAMES="$WAY_SHELL_TEST_EGL_VENDOR"
        fi
        cat > "$runtime/niri.kdl" <<'EOF'
input { keyboard { xkb { layout "us"; }; }; }
animations { off; }
hotkey-overlay { skip-at-startup; }
xwayland-satellite { off; }
workspace "first"
workspace "second"
EOF
        LIBGL_ALWAYS_SOFTWARE=1 niri -c "$runtime/niri.kdl" > "$runtime/niri.log" 2>&1 &
        niri_pid=$!
        attempt=0
        until [ -n "${NIRI_SOCKET:-}" ]; do
            for socket in "$runtime"/niri.*.sock; do
                if [ -S "$socket" ]; then NIRI_SOCKET=$socket; break; fi
            done
            attempt=$((attempt + 1))
            if [ "$attempt" -ge 100 ] || ! kill -0 "$niri_pid" 2>/dev/null; then
                cat "$runtime/niri.log" >&2
                exit 1
            fi
            sleep 0.1
        done
        for socket in "$runtime"/wayland-*; do
            if [ -S "$socket" ] && [ "$(basename "$socket")" != "$parent_display" ]; then
                WAYLAND_DISPLAY=$(basename "$socket")
                break
            fi
        done
        export NIRI_SOCKET WAYLAND_DISPLAY
        test "$WAYLAND_DISPLAY" != "$parent_display"
        ;;
    *) echo "Unknown test compositor: $backend" >&2; exit 2 ;;
esac
"$probe"
