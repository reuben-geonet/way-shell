CC ?= gcc
DEPS := libadwaita-1 wireplumber-0.5 json-glib-1.0 gtk4-layer-shell-0 upower-glib libnm
CFLAGS += -g -Wall -ffunction-sections -fdata-sections $(shell pkg-config --cflags $(DEPS))
LDFLAGS += -Wl,--gc-sections
LDLIBS += $(shell pkg-config --libs $(DEPS)) -lm
CORE_SOURCES := core.c ../lib/cmd_tree/cmd_tree.c \
  ../src/services/window_manager_service/sway/sway_client.c

.PHONY: check clean
check: core-test ipc-test
	cd .. && tests/core-test
	cd .. && tests/ipc-test
	cd .. && python3 tests/cli.py

core-test: $(CORE_SOURCES) ../src/services/wireplumber_service.h \
  ../src/services/wayland/gamma_control_service/colorramp.h
	$(CC) $(CPPFLAGS) $(CFLAGS) $(CORE_SOURCES) $(LDFLAGS) $(LDLIBS) -o $@

ipc-test: ipc.c ../src/services/ipc_service/ipc_service.c ../src/services/ipc_service/ipc_protocol.h
	$(CC) $(CPPFLAGS) $(CFLAGS) ipc.c $(LDFLAGS) $(LDLIBS) -o $@

clean:
	rm -f core-test ipc-test
