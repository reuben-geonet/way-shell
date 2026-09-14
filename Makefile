PREFIX ?= /usr
BINDIR ?= $(PREFIX)/bin
SCHEMADIR ?= $(PREFIX)/share/glib-2.0/schemas
USERUNITDIR ?= $(PREFIX)/lib/systemd/user

CC := gcc
CARGO ?= cargo
CARGO_BUILD_FLAGS ?= --locked
# gtk4-layer-shell must precede GTK/Wayland in the dynamic loader's search.
DEPS := gtk4-layer-shell-0 libadwaita-1 upower-glib wireplumber-0.5 \
        json-glib-1.0 libnm libpipewire-0.3 libpulse libpulse-simple \
        libpulse-mainloop-glib wayland-client wayland-protocols gio-unix-2.0
CFLAGS += $(shell pkg-config --cflags $(DEPS)) -g3 -Wall -MMD -MP
LIBS := $(shell pkg-config --libs $(DEPS)) -lm

.DEFAULT_GOAL := all
DBUS_BASES := src/services/dbus_dbus \
  src/services/logind_service/logind_manager_dbus \
  src/services/logind_service/logind_session_dbus \
  src/services/media_player_service/media_player_dbus \
  src/services/notifications_service/notifications_dbus \
  src/services/power_profiles_service/power_profiles_dbus \
  src/services/status_notifier_service/dbusmenu_dbus \
  src/services/status_notifier_service/status_notifier_host_dbus \
  src/services/status_notifier_service/status_notifier_item_dbus \
  src/services/status_notifier_service/status_notifier_watcher_dbus
WAYLAND_BASES := src/services/wayland/wlr-foreign-toplevel-management-unstable-v1 \
  src/services/wayland/wlr-gamma-control-unstable-v1
GENERATED_SOURCES := $(addsuffix .c,$(DBUS_BASES) $(WAYLAND_BASES))
GENERATED_HEADERS := $(addsuffix .h,$(DBUS_BASES) $(WAYLAND_BASES))
SOURCES := $(sort $(shell find src -type f -name '*.c') $(GENERATED_SOURCES))
OBJS := $(SOURCES:.c=.o) gresources.o
BRIDGE := target/release/libway_shell_bridge.a
RESOURCE_FILES := $(shell glib-compile-resources --generate-dependencies gresources.xml)

.PHONY: all check clean install install-gschema dbus-codegen wlr-protocols gresources
all: way-shell cli
check: all
	$(CARGO) test -p way-shell-core -p way-sh $(CARGO_BUILD_FLAGS)
	$(MAKE) -C tests check

way-shell: $(OBJS) $(BRIDGE)
	$(CC) $(CFLAGS) $(LDFLAGS) -o $@ $(OBJS) $(BRIDGE) $(LIBS) -ldl -lpthread

.PHONY: bridge
bridge:
	$(CARGO) build -p way-shell-bridge --release $(CARGO_BUILD_FLAGS)
$(BRIDGE): bridge

# Bootstrapping dependencies cover the first build; .d files provide precise
# header dependencies thereafter. Generated headers precede every C consumer.
$(OBJS): | $(GENERATED_HEADERS) gresources.h
$(GENERATED_SOURCES:.c=.o): %.o: %.c %.h

# GNU Make grouped targets regenerate both outputs if either one is absent.
define dbus_binding
$(1).c $(1).h &: data/dbus-interfaces/$(2).xml
	gdbus-codegen --generate-c-code $(notdir $(1)) --c-namespace Dbus \
	  --interface-prefix $(3) --output-directory $(dir $(1)) $$<
endef
$(eval $(call dbus_binding,src/services/dbus_dbus,org.freedesktop.DBus,org.freedesktop.))
$(eval $(call dbus_binding,src/services/logind_service/logind_manager_dbus,org.freedesktop.login1.Manager,org.freedesktop.))
$(eval $(call dbus_binding,src/services/logind_service/logind_session_dbus,org.freedesktop.login1.Session,org.freedesktop.))
$(eval $(call dbus_binding,src/services/media_player_service/media_player_dbus,org.mpris.MediaPlayer2,org.mpris.))
$(eval $(call dbus_binding,src/services/notifications_service/notifications_dbus,org.freedesktop.Notifications,org.freedesktop.))
$(eval $(call dbus_binding,src/services/power_profiles_service/power_profiles_dbus,net.hadess.PowerProfiles,net.hadess.))
$(eval $(call dbus_binding,src/services/status_notifier_service/dbusmenu_dbus,com.canonical.dbusmenu,com.canonical.))
$(eval $(call dbus_binding,src/services/status_notifier_service/status_notifier_host_dbus,org.kde.StatusNotifierHost,org.kde.))
$(eval $(call dbus_binding,src/services/status_notifier_service/status_notifier_item_dbus,org.kde.StatusNotifierItem,org.kde.))
$(eval $(call dbus_binding,src/services/status_notifier_service/status_notifier_watcher_dbus,org.kde.StatusNotifierWatcher,org.kde.))

$(WAYLAND_BASES:%=%.h): src/services/wayland/%.h: data/wlr-protocols/unstable/%.xml
	wayland-scanner client-header $< $@
$(WAYLAND_BASES:%=%.c): src/services/wayland/%.c: data/wlr-protocols/unstable/%.xml
	wayland-scanner private-code $< $@

dbus-codegen: $(addsuffix .c,$(DBUS_BASES)) $(addsuffix .h,$(DBUS_BASES))
wlr-protocols: $(addsuffix .c,$(WAYLAND_BASES)) $(addsuffix .h,$(WAYLAND_BASES))
gresources: gresources.o gresources.h

gresources.c gresources.h &: gresources.xml $(RESOURCE_FILES)
	glib-compile-resources --generate-source --target gresources.c gresources.xml
	glib-compile-resources --generate-header --target gresources.h gresources.xml

.PHONY: cli
cli:
	$(CARGO) build -p way-sh --release $(CARGO_BUILD_FLAGS)

install-gschema:
	glib-compile-schemas $(DESTDIR)$(SCHEMADIR)
install:
	install -D ./way-shell $(DESTDIR)$(BINDIR)/way-shell
	install -D target/release/way-sh $(DESTDIR)$(BINDIR)/way-sh
	install -D data/org.ldelossa.way-shell.gschema.xml $(DESTDIR)$(SCHEMADIR)/org.ldelossa.way-shell.gschema.xml
	install -D -m 0644 contrib/systemd/way-shell.service $(DESTDIR)$(USERUNITDIR)/way-shell.service

clean:
	rm -f $(OBJS) $(OBJS:.o=.d) $(GENERATED_SOURCES) $(GENERATED_HEADERS)
	rm -f way-shell gresources.c gresources.h
	$(MAKE) -C tests clean

-include $(OBJS:.o=.d)
