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

gschemas.compiled: ../data/org.ldelossa.way-shell.gschema.xml
	glib-compile-schemas --strict --targetdir=. ../data

clean:
	rm -f core-test ipc-test theme-test clock-test sway-transport-test niri-test
	rm -f gschemas.compiled
	rm -f network-test network-operations-test
	rm -f media-widgets-test
	rm -f media-presentation-test
	rm -f notification-presentation-test
	rm -f notification-replacement-test

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
