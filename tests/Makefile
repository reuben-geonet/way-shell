CC ?= gcc
DEPS := libadwaita-1 wireplumber-0.5 json-glib-1.0 gtk4-layer-shell-0 upower-glib libnm libpulse-mainloop-glib
CFLAGS += -g -Wall -ffunction-sections -fdata-sections $(shell pkg-config --cflags $(DEPS))
LDFLAGS += -Wl,--gc-sections
LDLIBS += ../target/release/libway_shell_bridge.a $(shell pkg-config --libs $(DEPS)) -lm -ldl -lpthread
CORE_SOURCES := core.c

.PHONY: check clean
check: core-test ipc-test network-test tray-baseline-test
	cd .. && tests/core-test
	cd .. && tests/ipc-test
	cd .. && tests/network-test
	cd .. && tests/tray-baseline-test

TRAY_BASES := dbusmenu status_notifier_item status_notifier_watcher status_notifier_host
TRAY_SOURCES := $(addprefix ../src/services/status_notifier_service/,$(addsuffix _dbus.c,$(TRAY_BASES)))
TRAY_HEADERS := $(TRAY_SOURCES:.c=.h)
TRAY_XML := ../data/dbus-interfaces/com.canonical.dbusmenu.xml \
  ../data/dbus-interfaces/org.kde.StatusNotifierItem.xml \
  ../data/dbus-interfaces/org.kde.StatusNotifierWatcher.xml \
  ../data/dbus-interfaces/org.kde.StatusNotifierHost.xml

$(TRAY_SOURCES) $(TRAY_HEADERS) &: $(TRAY_XML)
	$(MAKE) -C .. dbus-codegen

tray-baseline-test: tray-baseline.c $(TRAY_SOURCES) $(TRAY_HEADERS) \
  ../src/services/status_notifier_service/status_notifier_service.c \
  ../src/services/status_notifier_service/status_notifier_service.h \
  ../src/services/status_notifier_service/libdbusmenu.c \
  ../src/services/status_notifier_service/libdbusmenu.h \
  ../src/services/status_notifier_service/libdbus_menu_consts.h
	$(CC) $(CPPFLAGS) $(CFLAGS) tray-baseline.c $(TRAY_SOURCES) ../src/services/status_notifier_service/libdbusmenu.c $(LDFLAGS) $(shell pkg-config --libs libadwaita-1 gio-unix-2.0) -o $@

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
	rm -f media-presentation-test
	rm -f notification-presentation-test
	rm -f notification-replacement-test
	rm -f tray-baseline-test

audio-widgets-test: audio-widgets.c ../src/panel/quick_settings/quick_settings_scales/quick_settings_scales.c
	$(CC) $(CPPFLAGS) $(CFLAGS) audio-widgets.c $(LDFLAGS) $(shell pkg-config --libs libadwaita-1) -o $@

media-widgets-test: media-widgets.c gschemas.compiled ../src/panel/message_tray/notifications/notifications_list.c \
  ../src/services/media_player_service/media_player_service.h
	$(CC) $(CPPFLAGS) $(CFLAGS) media-widgets.c $(LDFLAGS) $(shell pkg-config --libs libadwaita-1) -o $@

media-presentation-test: media-presentation.c ../src/panel/message_tray/notifications/notification_widget.c
	$(CC) $(CPPFLAGS) $(CFLAGS) media-presentation.c $(LDFLAGS) $(shell pkg-config --libs libadwaita-1) -o $@

notification-presentation-test: notification-presentation.c ../src/panel/message_tray/notifications/notification_widget.c
	$(CC) $(CPPFLAGS) $(CFLAGS) notification-presentation.c $(LDFLAGS) $(shell pkg-config --libs libadwaita-1) -o $@

notification-replacement-test: notification-replacement.c gschemas.compiled \
  ../src/panel/message_tray/notifications/notification_widget.c \
  ../src/panel/message_tray/notifications/notification_widget.h \
  ../src/panel/message_tray/notifications/notification_group.c \
  ../src/panel/message_tray/notifications/notification_osd.c \
  ../src/panel/message_tray/notifications/notifications_list.c
	$(CC) $(CPPFLAGS) $(CFLAGS) notification-replacement.c ../src/panel/message_tray/notifications/notification_widget.c $(LDFLAGS) $(shell pkg-config --libs gtk4-layer-shell-0 libadwaita-1) -o $@
