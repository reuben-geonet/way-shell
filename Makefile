PREFIX ?= /usr
BINDIR ?= $(PREFIX)/bin
SCHEMADIR ?= $(PREFIX)/share/glib-2.0/schemas
USERUNITDIR ?= $(PREFIX)/lib/systemd/user

CC := gcc
# depedency order matters here.
# Gtk4LayerShell must be linked before any wayland libraries
DEPS := libadwaita-1 \
		gtk4-layer-shell-0 \
		upower-glib \
		wireplumber-0.5 \
		json-glib-1.0 \
		libnm \
		libpipewire-0.3 \
		libpulse \
		libpulse-simple \
		libpulse-mainloop-glib \
		wayland-client \
		wayland-protocols \
		gio-unix-2.0
CFLAGS += $(shell pkg-config --cflags $(DEPS)) -g3 -Wall
LIBS := $(LDFLAGS) "-lm"
LIBS += $(shell pkg-config --libs $(DEPS))
# Include generated sources even before the first invocation of wayland-scanner.
WAYLAND_SOURCES := src/services/wayland/wlr-foreign-toplevel-management-unstable-v1.c \
		src/services/wayland/wlr-gamma-control-unstable-v1.c
SOURCES := $(sort $(shell find src/ -type f -name "*.c") $(WAYLAND_SOURCES))
OBJS := $(patsubst %.c, %.o, $(SOURCES))
OBJS += lib/cmd_tree/cmd_tree.o

all: wlr-protocols gresources way-shell way-sh/way-sh

.PHONY: test-bluetooth
test-bluetooth:
	mkdir -p .cache/bluetooth-tests/schemas
	cp data/org.ldelossa.way-shell.gschema.xml .cache/bluetooth-tests/schemas/
	glib-compile-schemas --strict .cache/bluetooth-tests/schemas
	$(CC) -g -Wall -Isrc/services/bluetooth_service tests/bluetooth.c src/services/bluetooth_service/bluetooth_service.c src/services/bluetooth_service/bluetooth_settings.c -o .cache/bluetooth-tests/test $$(pkg-config --cflags --libs gio-2.0 gio-unix-2.0)
	GSETTINGS_SCHEMA_DIR=$(CURDIR)/.cache/bluetooth-tests/schemas GSETTINGS_BACKEND=memory .cache/bluetooth-tests/test

way-shell: $(OBJS)
	$(CC) $(CFLAGS) -o $@ $(OBJS) gresources.o $(LIBS)

lib/cmd_tree/cmd_tree.o: lib/cmd_tree/cmd_tree.c

lib/cmd_tree/cmd_tree.c:
	make -C lib/cmd_tree

.PHONY:
gresources:
	glib-compile-resources --generate-source --target gresources.c gresources.xml
	glib-compile-resources --generate-header --target gresources.h gresources.xml
	$(CC) $(CFLAGS) -c gresources.c -o gresources.o $(LIBS)

.PHONY:
wlr-protocols:
	wayland-scanner client-header < ./data/wlr-protocols/unstable/wlr-foreign-toplevel-management-unstable-v1.xml > ./src/services/wayland/wlr-foreign-toplevel-management-unstable-v1.h
	wayland-scanner private-code < ./data/wlr-protocols/unstable/wlr-foreign-toplevel-management-unstable-v1.xml > ./src/services/wayland/wlr-foreign-toplevel-management-unstable-v1.c
	wayland-scanner client-header ./data/wlr-protocols/unstable/wlr-gamma-control-unstable-v1.xml ./src/services/wayland/wlr-gamma-control-unstable-v1.h
	wayland-scanner private-code ./data/wlr-protocols/unstable/wlr-gamma-control-unstable-v1.xml ./src/services/wayland/wlr-gamma-control-unstable-v1.c

.PHONY:
way-sh/way-sh:
	make -C way-sh/

.PHONY:
dbus-codegen:
	# dbus
	gdbus-codegen --generate-c-code dbus_dbus \
	--c-namespace Dbus \
	--interface-prefix org.freedesktop. \
	--output-directory ./src/services/ \
	./data/dbus-interfaces/org.freedesktop.DBus.xml

	# logind
	gdbus-codegen --generate-c-code logind_manager_dbus \
	--c-namespace Dbus \
	--interface-prefix org.freedesktop. \
	--output-directory ./src/services/logind_service \
	./data/dbus-interfaces/org.freedesktop.login1.Manager.xml

	gdbus-codegen --generate-c-code logind_session_dbus \
	--c-namespace Dbus \
	--interface-prefix org.freedesktop. \
	--output-directory ./src/services/logind_service \
	./data/dbus-interfaces/org.freedesktop.login1.Session.xml

	# notifications
	gdbus-codegen --generate-c-code notifications_dbus \
	--c-namespace Dbus \
	--interface-prefix org.freedesktop. \
	--output-directory ./src/services/notifications_service \
	./data/dbus-interfaces/org.freedesktop.Notifications.xml

	# power profiles daemon
	gdbus-codegen --generate-c-code power_profiles_dbus \
	--c-namespace Dbus \
	--interface-prefix net.hadess. \
	--output-directory ./src/services/power_profiles_service \
	./data/dbus-interfaces/net.hadess.PowerProfiles.xml

	# media player
	gdbus-codegen --generate-c-code media_player_dbus \
	--c-namespace Dbus \
	--interface-prefix org.mpris. \
	--output-directory ./src/services/media_player_service \
	./data/dbus-interfaces/org.mpris.MediaPlayer2.xml

	# dbusmenu
	gdbus-codegen --generate-c-code dbusmenu_dbus \
	--c-namespace Dbus \
	--interface-prefix com.canonical. \
	--output-directory ./src/services/status_notifier_service/ \
	./data/dbus-interfaces/com.canonical.dbusmenu.xml

.PHONY:
install-gschema:
	glib-compile-schemas $(DESTDIR)$(SCHEMADIR)

.PHONY:
install:
	install -D ./way-shell $(DESTDIR)$(BINDIR)/way-shell
	install -D ./way-sh/way-sh $(DESTDIR)$(BINDIR)/way-sh
	install -D data/org.ldelossa.way-shell.gschema.xml $(DESTDIR)$(SCHEMADIR)/org.ldelossa.way-shell.gschema.xml
	install -D -m 0644 contrib/systemd/way-shell.service $(DESTDIR)$(USERUNITDIR)/way-shell.service

clean:
	find . -name "*.o" -type f -exec rm -f {} \;
	rm -rf way-shell
	rm -rf gresources.{h,c,o}
	make -C way-sh/ clean
