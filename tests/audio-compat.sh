#!/bin/sh
# Run the binding probe against a private daemon; no desktop audio is changed.
set -eu
probe=$(realpath "${1:?pass the compiled audio-compat example}")
fixture=$(realpath tests/fixtures/pipewire.conf)
runtime=$(mktemp -d /tmp/way-shell-audio.XXXXXX)
daemon_pid=
cleanup() {
    if [ -n "$daemon_pid" ]; then
        kill "$daemon_pid" 2>/dev/null || :
        wait "$daemon_pid" 2>/dev/null || :
    fi
    rm -rf "$runtime"
}
trap cleanup EXIT HUP INT TERM
export XDG_RUNTIME_DIR="$runtime"
pipewire -c "$fixture" > "$runtime/pipewire.log" 2>&1 &
daemon_pid=$!
attempt=0
until [ -S "$runtime/pipewire-0" ]; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 50 ] || ! kill -0 "$daemon_pid" 2>/dev/null; then
        cat "$runtime/pipewire.log" >&2
        exit 1
    fi
    sleep 0.1
done
"$probe"
