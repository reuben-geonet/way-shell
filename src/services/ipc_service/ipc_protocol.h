#pragma once

#include <math.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include "ipc_commands.h"

/* Explicit x86_64 wire encoding, independent of struct alignment. */
static inline uint32_t ipc_read_u32(const uint8_t *bytes) {
    return (uint32_t)bytes[0] | (uint32_t)bytes[1] << 8 |
           (uint32_t)bytes[2] << 16 | (uint32_t)bytes[3] << 24;
}

static inline void ipc_write_u32(uint8_t *bytes, uint32_t value) {
    for (unsigned i = 0; i < 4; i++) bytes[i] = value >> (i * 8);
}

static inline bool ipc_decode_request(const uint8_t *bytes, size_t size,
                                      IPCVolumeSet *request) {
    if (size < 4) return false;
    uint32_t opcode = ipc_read_u32(bytes);
    if (opcode > IPC_CMD_RENAME_SWITCHER_TOGGLE) return false;
    size_t expected = opcode == IPC_CMD_VOLUME_SET ? 8 : 4;
    if (size != expected) return false;
    request->header.type = opcode;
    request->volume = 0;
    if (opcode == IPC_CMD_VOLUME_SET) {
        uint32_t bits = ipc_read_u32(bytes + 4);
        _Static_assert(sizeof(float) == sizeof(bits), "IPC requires a 32-bit float");
        memcpy(&request->volume, &bits, sizeof(bits));
        if (!isfinite(request->volume) || request->volume < 0 || request->volume > 1)
            return false;
    }
    return true;
}
