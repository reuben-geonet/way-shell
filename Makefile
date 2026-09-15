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
SOURCES := $(sort $(shell find src -type f -name '*.c'))
OBJS := $(SOURCES:.c=.o)
BRIDGE := target/release/libway_shell_bridge.a

.PHONY: all check clean install install-gschema way-shell
all: way-shell cli
check: all $(BRIDGE)
	$(CARGO) test --workspace $(CARGO_BUILD_FLAGS)
	$(MAKE) -C tests check

way-shell:
	$(CARGO) build -p way-shell --bin way-shell --release $(CARGO_BUILD_FLAGS)
	install -m755 target/release/way-shell $@

.PHONY: bridge
bridge:
	$(CARGO) build -p way-shell-bridge --release $(CARGO_BUILD_FLAGS)
$(BRIDGE): bridge

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
	rm -f $(OBJS) $(OBJS:.o=.d)
	rm -f way-shell gresources.c gresources.h
	$(MAKE) -C tests clean

-include $(OBJS:.o=.d)
