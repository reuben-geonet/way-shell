CC ?= gcc
DEPS := libadwaita-1 wireplumber-0.5 json-glib-1.0 gtk4-layer-shell-0 upower-glib libnm
CFLAGS += -g -Wall -ffunction-sections -fdata-sections $(shell pkg-config --cflags $(DEPS))
LDFLAGS += -Wl,--gc-sections
LDLIBS += ../target/release/libway_shell_bridge.a $(shell pkg-config --libs $(DEPS)) -lm -ldl -lpthread
CORE_SOURCES := core.c

.PHONY: check clean
check: core-test ipc-test power-profiles-test wayland-state-test power-boundary-test
	cd .. && tests/core-test
	cd .. && tests/ipc-test
	cd .. && tests/power-profiles-test
	cd .. && tests/wayland-state-test
	cd .. && tests/power-boundary-test

core-test: $(CORE_SOURCES) ../target/release/libway_shell_bridge.a ../src/services/wireplumber_service.h \
  ../src/services/wayland/gamma_control_service/colorramp.h
	$(CC) $(CPPFLAGS) $(CFLAGS) $(CORE_SOURCES) $(LDFLAGS) $(LDLIBS) -o $@

ipc-test: ipc.c ../src/services/ipc_service/ipc_service.c ../src/services/ipc_service/ipc_protocol.h
	$(CC) $(CPPFLAGS) $(CFLAGS) ipc.c $(LDFLAGS) $(LDLIBS) -o $@

power-profiles-test: power-profiles.c ../src/services/power_profiles_service/power_profiles_service.c \
  ../src/services/power_profiles_service/power_profiles_dbus.c ../src/services/power_profiles_service/power_profiles_dbus.h
	$(CC) $(CPPFLAGS) $(CFLAGS) power-profiles.c ../src/services/power_profiles_service/power_profiles_dbus.c $(LDFLAGS) $(shell pkg-config --libs libadwaita-1) -o $@

wayland-state-test: wayland-state.c ../src/services/wayland/foreign_toplevel_service/foreign_toplevel.c \
  ../src/services/wayland/wlr-foreign-toplevel-management-unstable-v1.h
	$(CC) $(CPPFLAGS) $(CFLAGS) wayland-state.c $(LDFLAGS) $(shell pkg-config --libs libadwaita-1 wayland-client) -o $@

power-boundary-test: power-boundary.c ../src/services/upower_service.c
	$(CC) $(CPPFLAGS) $(CFLAGS) $^ $(LDFLAGS) $(shell pkg-config --libs libadwaita-1 upower-glib) -o $@

clean:
	rm -f core-test ipc-test theme-test clock-test sway-transport-test niri-test power-profiles-test wayland-state-test power-boundary-test
