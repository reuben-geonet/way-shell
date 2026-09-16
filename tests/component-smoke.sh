#!/bin/sh
# Run the GTK/Wayland component matrix from the repository root.
# Audio and full application lifecycle checks have separate harnesses.
set -eu
examples=$(realpath "${1:?pass the compiled examples directory}")

run_probe() (
    probe=$1
    backend=$2
    renderer=$3
    warnings=$4
    if [ "$renderer" = cairo ]; then
        export GSK_RENDERER=cairo
    fi
    if [ "$warnings" = fatal ]; then
        export G_DEBUG=fatal-warnings
    fi
    printf 'Running %s on %s\n' "$probe" "$backend"
    sh tests/wayland-component.sh "$examples/$probe" "$backend"
)

# "default" retains the caller's environment. Keep each probe's existing
# renderer and warning policy; process isolation prevents settings leaking.
while read -r probe backends renderer warnings; do
    if [ "$backends" = both ]; then
        backends='sway niri'
    fi
    for backend in $backends; do
        run_probe "$probe" "$backend" "$renderer" "$warnings"
    done
done <<'PROBES'
theme-smoke both default default
shortcuts-smoke both cairo fatal
sway-smoke sway default default
notification-ui-smoke both cairo fatal
message-tray-media-smoke both cairo fatal
message-tray-window-smoke both cairo fatal
message-tray-smoke both cairo fatal
tray-ui-smoke both cairo fatal
panel-status-smoke both cairo default
panel-smoke both cairo default
niri-smoke niri default default
wayland-smoke both cairo default
window-smoke both cairo default
popup-click-away-smoke sway cairo fatal
switcher-smoke both cairo default
workspace-switchers-smoke both cairo default
activities-smoke both cairo fatal
quick-settings-window-smoke both cairo fatal
quick-settings-system-smoke both cairo fatal
quick-settings-bluetooth-smoke both cairo fatal
quick-settings-network-smoke both cairo fatal
dialog-smoke both cairo fatal
osd-smoke both cairo fatal
quick-settings-audio-smoke both cairo fatal
quick-settings-smoke both cairo fatal
PROBES
