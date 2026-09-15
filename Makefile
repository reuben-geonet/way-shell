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
# Match Cargo linking: discard unused Rust sections in the temporary static bridge.
LDFLAGS += -Wl,--gc-sections

.DEFAULT_GOAL := all
DBUS_BASES := src/services/dbus_dbus \
  src/services/status_notifier_service/dbusmenu_dbus \
  src/services/status_notifier_service/status_notifier_host_dbus \
  src/services/status_notifier_service/status_notifier_item_dbus \
  src/services/status_notifier_service/status_notifier_watcher_dbus
GENERATED_SOURCES := $(addsuffix .c,$(DBUS_BASES))
GENERATED_HEADERS := $(addsuffix .h,$(DBUS_BASES))
SOURCES := $(sort $(shell find src -type f -name '*.c') $(GENERATED_SOURCES))
OBJS := $(SOURCES:.c=.o)
BRIDGE := target/release/libway_shell_bridge.a

.PHONY: all check clean install install-gschema dbus-codegen
all: way-shell cli
check: all
	$(CARGO) test --workspace $(CARGO_BUILD_FLAGS)
	$(MAKE) -C tests check

way-shell: $(OBJS) $(BRIDGE)
	$(CC) $(CFLAGS) $(LDFLAGS) -o $@ $(OBJS) $(BRIDGE) $(LIBS) -ldl -lpthread

.PHONY: bridge
bridge:
	$(CARGO) build -p way-shell-bridge --release $(CARGO_BUILD_FLAGS)
$(BRIDGE): bridge

# Bootstrapping dependencies cover the first build; .d files provide precise
# header dependencies thereafter. Generated headers precede every C consumer.
$(OBJS): | $(GENERATED_HEADERS)
$(GENERATED_SOURCES:.c=.o): %.o: %.c %.h

# GNU Make grouped targets regenerate both outputs if either one is absent.
define dbus_binding
$(1).c $(1).h &: data/dbus-interfaces/$(2).xml
	gdbus-codegen --generate-c-code $(notdir $(1)) --c-namespace Dbus \
	  --interface-prefix $(3) --output-directory $(dir $(1)) $$<
endef
$(eval $(call dbus_binding,src/services/dbus_dbus,org.freedesktop.DBus,org.freedesktop.))
$(eval $(call dbus_binding,src/services/status_notifier_service/dbusmenu_dbus,com.canonical.dbusmenu,com.canonical.))
$(eval $(call dbus_binding,src/services/status_notifier_service/status_notifier_host_dbus,org.kde.StatusNotifierHost,org.kde.))
$(eval $(call dbus_binding,src/services/status_notifier_service/status_notifier_item_dbus,org.kde.StatusNotifierItem,org.kde.))
$(eval $(call dbus_binding,src/services/status_notifier_service/status_notifier_watcher_dbus,org.kde.StatusNotifierWatcher,org.kde.))

dbus-codegen: $(addsuffix .c,$(DBUS_BASES)) $(addsuffix .h,$(DBUS_BASES))

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
