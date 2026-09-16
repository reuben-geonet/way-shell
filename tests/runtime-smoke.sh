#!/bin/sh
set -eu
: "${WAY_SHELL_RUNTIME_TEST_BINARY:?set the compiled test binary}"
export WAY_SHELL_PRIVATE_RUNTIME_TEST=1
export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS"
export XDG_CONFIG_HOME="$XDG_RUNTIME_DIR/config"
export XDG_DATA_HOME="$XDG_RUNTIME_DIR/data"
export XDG_DATA_DIRS="$XDG_RUNTIME_DIR/no-system-data"
export PIPEWIRE_RUNTIME_DIR="$XDG_RUNTIME_DIR"
export PIPEWIRE_REMOTE="$XDG_RUNTIME_DIR/pipewire-0"
export PULSE_SERVER="unix:$XDG_RUNTIME_DIR/no-pulse"
unset PIPEWIRE_CONFIG_DIR PIPEWIRE_CONFIG_PREFIX PIPEWIRE_CONFIG_NAME
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_DATA_DIRS"
pipewire -c "$PWD/tests/fixtures/pipewire.conf" > "$XDG_RUNTIME_DIR/runtime-pipewire.log" 2>&1 &
pipewire_pid=$!
cleanup() {
    kill "$pipewire_pid" 2>/dev/null || :
    wait "$pipewire_pid" 2>/dev/null || :
}
trap cleanup EXIT HUP INT TERM
attempt=0
until [ -S "$PIPEWIRE_REMOTE" ]; do
    attempt=$((attempt + 1))
    if [ "$attempt" -gt 100 ] || ! kill -0 "$pipewire_pid" 2>/dev/null; then
        cat "$XDG_RUNTIME_DIR/runtime-pipewire.log" >&2
        exit 1
    fi
    sleep 0.05
done
"$WAY_SHELL_RUNTIME_TEST_BINARY" application::tests::gtk_runtime_commands_output_recovery_and_reentrant_shutdown --ignored --exact --nocapture --test-threads=1
