#pragma once
#include <stddef.h>
#include <stdint.h>

/* Borrowed inputs, synchronous calls, no ownership transfer. */
double volume_from_linear(float volume, int32_t scale);
float volume_to_linear(double volume, int32_t scale);
int32_t way_shell_channel_index(const char *channel);
int32_t way_shell_gamma_supported(size_t size, int32_t temperature);
/* Three disjoint buffers, each containing size writable uint16_t values. */
int32_t way_shell_gamma_fill(uint16_t *red, uint16_t *green, uint16_t *blue,
                            size_t size, int32_t temperature);
