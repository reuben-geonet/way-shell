#pragma once
#include "../../../rust_bridge.h"

/* Compatibility adapter for C callers; calculations and table are in Rust. */
static inline int colorramp_fill(uint16_t *red, uint16_t *green, uint16_t *blue,
                                int size, int temperature) {
    if (size <= 0) return -1;
    return way_shell_gamma_fill(red, green, blue, (size_t)size, temperature);
}
