#!/bin/sh
# Isolated component smoke test; accepts a compiled Rust example as its argument.
set -eu
probe=$(realpath "${1:?pass the compiled component probe}")
runtime=$(mktemp -d /tmp/way-shell-wayland.XXXXXX)
compositor_pid=
cleanup() {
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
unset SWAYSOCK WAYLAND_DISPLAY
cat > "$runtime/sway.conf" <<'EOF'
output * resolution 800x600
seat seat0 fallback true
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
"$probe"
