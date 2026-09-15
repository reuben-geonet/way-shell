CC ?= gcc
DEPS := libadwaita-1 wireplumber-0.5 json-glib-1.0 gtk4-layer-shell-0 upower-glib libnm
CFLAGS += -g -Wall -ffunction-sections -fdata-sections $(shell pkg-config --cflags $(DEPS))
LDFLAGS += -Wl,--gc-sections
LDLIBS += ../target/release/libway_shell_bridge.a $(shell pkg-config --libs $(DEPS)) -lm -ldl -lpthread
CORE_SOURCES := core.c

.PHONY: check clean
check: core-test ipc-test power-profiles-test wayland-state-test power-boundary-test logind-test brightness-test
	cd .. && tests/core-test
	cd .. && tests/ipc-test
	cd .. && tests/power-profiles-test
	cd .. && tests/wayland-state-test
	cd .. && tests/power-boundary-test
	cd .. && GSETTINGS_SCHEMA_DIR=tests tests/logind-test
	cd .. && GSETTINGS_SCHEMA_DIR=tests GSETTINGS_BACKEND=memory tests/brightness-test

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

power-profiles-widgets-test: power-profiles-widgets.c \
  ../src/panel/quick_settings/quick_settings_grid/quick_settings_grid_power_profiles/quick_settings_grid_power_profiles_menu.c \
  ../src/panel/quick_settings/quick_settings_menu_widget.c
	$(CC) $(CPPFLAGS) $(CFLAGS) power-profiles-widgets.c ../src/panel/quick_settings/quick_settings_menu_widget.c $(LDFLAGS) $(shell pkg-config --libs libadwaita-1) -o $@

gschemas.compiled: ../data/org.ldelossa.way-shell.gschema.xml
	glib-compile-schemas --strict --targetdir=. ../data

battery-widgets-test: battery-widgets.c \
  ../src/panel/quick_settings/quick_settings_header/quick_settings_battery_menu.c \
  ../src/panel/quick_settings/quick_settings_menu_widget.c
	$(CC) $(CPPFLAGS) $(CFLAGS) battery-widgets.c ../src/panel/quick_settings/quick_settings_menu_widget.c $(LDFLAGS) $(shell pkg-config --libs libadwaita-1 upower-glib) -o $@

power-widgets-test: power-widgets.c \
  ../src/panel/quick_settings/quick_settings_header/quick_settings_power_menu.c \
  ../src/panel/quick_settings/quick_settings_menu_widget.c
	$(CC) $(CPPFLAGS) $(CFLAGS) power-widgets.c ../src/panel/quick_settings/quick_settings_menu_widget.c $(LDFLAGS) $(shell pkg-config --libs libadwaita-1) -o $@

logind-test: logind.c ../src/services/logind_service/logind_service.c \
  ../src/services/logind_service/logind_manager_dbus.c ../src/services/logind_service/logind_session_dbus.c gschemas.compiled
	$(CC) $(CPPFLAGS) $(CFLAGS) logind.c ../src/services/logind_service/logind_manager_dbus.c ../src/services/logind_service/logind_session_dbus.c $(LDFLAGS) $(shell pkg-config --libs libadwaita-1 gio-unix-2.0) -o $@

brightness-test: brightness.c ../src/services/brightness_service/brightness_service.c gschemas.compiled
	$(CC) $(CPPFLAGS) $(CFLAGS) brightness.c $(LDFLAGS) $(shell pkg-config --libs libadwaita-1 gio-unix-2.0) -o $@

clean:
	rm -f core-test ipc-test theme-test clock-test sway-transport-test niri-test power-profiles-test wayland-state-test power-boundary-test
	rm -f power-profiles-widgets-test
	rm -f logind-test gschemas.compiled
	rm -f brightness-test
	rm -f battery-widgets-test
	rm -f power-widgets-test
