#pragma once
#include <gio/gio.h>

/* Rust owns and registers this immutable resource. The returned pointer is
 * borrowed for the lifetime of the application thread. */
GResource *way_shell_get_resource(void);
void way_shell_rust_shutdown(void);
