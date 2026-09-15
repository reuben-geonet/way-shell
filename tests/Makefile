CC ?= gcc
DEPS := libadwaita-1 wireplumber-0.5 json-glib-1.0 gtk4-layer-shell-0 upower-glib libnm libpulse-mainloop-glib
CFLAGS += -g -Wall -ffunction-sections -fdata-sections $(shell pkg-config --cflags $(DEPS))
LDFLAGS += -Wl,--gc-sections
LDLIBS += ../target/release/libway_shell_bridge.a $(shell pkg-config --libs $(DEPS)) -lm -ldl -lpthread
CORE_SOURCES := core.c

.PHONY: check clean
check: core-test ipc-test network-test
	cd .. && tests/core-test
	cd .. && tests/ipc-test
	cd .. && tests/network-test

network-test: network.c ../src/services/network_manager_service.c ../src/services/network_inventory.h
	$(CC) $(CPPFLAGS) $(CFLAGS) network.c $(LDFLAGS) $(shell pkg-config --libs libadwaita-1 libnm) -o $@

core-test: $(CORE_SOURCES) ../target/release/libway_shell_bridge.a ../src/services/wireplumber_service.h \
  ../src/services/wayland/gamma_control_service/colorramp.h
	$(CC) $(CPPFLAGS) $(CFLAGS) $(CORE_SOURCES) $(LDFLAGS) $(LDLIBS) -o $@

ipc-test: ipc.c ../src/services/ipc_service/ipc_service.c ../src/services/ipc_service/ipc_protocol.h
	$(CC) $(CPPFLAGS) $(CFLAGS) ipc.c $(LDFLAGS) $(LDLIBS) -o $@

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

brightness-widgets-test: brightness-widgets.c \
  ../src/panel/quick_settings/quick_settings_grid/quick_settings_keyboard_brightness/quick_settings_grid_keyboard_brightness_menu.c \
  ../src/panel/quick_settings/quick_settings_menu_widget.c
	$(CC) $(CPPFLAGS) $(CFLAGS) brightness-widgets.c ../src/panel/quick_settings/quick_settings_menu_widget.c $(LDFLAGS) $(shell pkg-config --libs libadwaita-1) -o $@

clean:
	rm -f core-test ipc-test theme-test clock-test sway-transport-test niri-test
	rm -f power-profiles-widgets-test
	rm -f gschemas.compiled
	rm -f battery-widgets-test
	rm -f power-widgets-test
	rm -f brightness-widgets-test
	rm -f network-test network-operations-test
	rm -f audio-widgets-test
	rm -f media-widgets-test

audio-widgets-test: audio-widgets.c ../src/panel/quick_settings/quick_settings_scales/quick_settings_scales.c
	$(CC) $(CPPFLAGS) $(CFLAGS) audio-widgets.c $(LDFLAGS) $(shell pkg-config --libs libadwaita-1) -o $@

media-widgets-test: media-widgets.c ../src/panel/message_tray/notifications/notifications_list.c \
  ../src/services/media_player_service/media_player_service.h
	$(CC) $(CPPFLAGS) $(CFLAGS) media-widgets.c $(LDFLAGS) $(shell pkg-config --libs libadwaita-1) -o $@
