#pragma once
#include "../lib/cmd_tree/include/cmd_tree.h"
#include "../src/services/ipc_service/ipc_protocol.h"
#include <errno.h>
#include <poll.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <time.h>

typedef struct _ctx {
    char *server_socket_path;
    int client_sock;
    int64_t deadline_ms;
} way_sh_ctx;

static inline int64_t way_sh_now_ms(void) {
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) return -1;
    return (int64_t)now.tv_sec * 1000 + now.tv_nsec / 1000000;
}

static inline bool way_sh_parse_volume(const char *text, float *volume) {
    char *end;
    errno = 0;
    *volume = strtof(text, &end);
    return text != end && *end == '\0' && errno != ERANGE &&
           isfinite(*volume) && *volume >= 0 && *volume <= 1;
}

static inline int way_sh_send(way_sh_ctx *ctx, const void *message, size_t size) {
    uint8_t bytes[8];
    uint32_t opcode;
    _Static_assert(sizeof(IPCHeader) == 4 && sizeof(IPCVolumeSet) == 8,
                   "unexpected C IPC layout");
    memcpy(&opcode, message, sizeof(opcode));
    ipc_write_u32(bytes, opcode);
    if (size == 8) {
        uint32_t bits;
        memcpy(&bits, (const uint8_t *)message + 4, sizeof(bits));
        ipc_write_u32(bytes + 4, bits);
    }
    int64_t now = way_sh_now_ms();
    if (now < 0) return -1;
    ctx->deadline_ms = now + 2000;
    return send(ctx->client_sock, bytes, size, MSG_NOSIGNAL);
}

static inline int way_sh_receive(way_sh_ctx *ctx, bool *response) {
    *response = false;
    for (;;) {
        int64_t now = way_sh_now_ms();
        if (now < 0) break;
        int64_t remaining = ctx->deadline_ms - now;
        if (remaining <= 0) { errno = ETIMEDOUT; break; }
        struct pollfd fd = {.fd = ctx->client_sock, .events = POLLIN};
        int ready = poll(&fd, 1, (int)remaining);
        if (ready < 0 && errno == EINTR) continue;
        if (ready < 0) break;
        if (ready == 0) { errno = ETIMEDOUT; break; }
        uint8_t bytes[4];
        ssize_t size = recv(ctx->client_sock, bytes, sizeof(bytes), MSG_TRUNC);
        if (size < 0 && (errno == EINTR || errno == EAGAIN)) continue;
        if (size < 0) break;
        if (size != sizeof(bytes) || ipc_read_u32(bytes) > 1) {
            errno = EPROTO;
            break;
        }
        *response = ipc_read_u32(bytes) == 1;
        return 0;
    }
    perror("way-sh: failed to receive acknowledgement");
    return -1;
}

#define IPC_SEND_MSG(way_ctx, msg) ret = way_sh_send(way_ctx, &(msg), sizeof(msg))
#define IPC_RECV_MSG(way_ctx, addr, response) way_sh_receive(way_ctx, response)

// The root command node.
//
// It is exec'd when no command is provided to way-sh and provides a short
// description of way-sh's usage.
//
// Defined in ./root.c
extern cmd_tree_node_t root_cmd;

// The message_tray comand root.
//
// Subcommands off this node deal with showing or manipulating the Message Tray
// UI component.
cmd_tree_node_t *message_tray_cmd();

// The volume command root.
//
// Subcommands off this node deal with adjusting the default audio sink's
// volume.
cmd_tree_node_t *volume_cmd();

// The Brightness command
//
// Subcommands off this node deal with adjusting the default display's
// brightness.
cmd_tree_node_t *brightness_cmd();

// The Theme command
//
// Subcommands off this node deal with adjusting Way-Shell' theme.
cmd_tree_node_t *theme_cmd();

// The Activities command
//
// Subcommands off this node deal with showing and hiding the Activities widget.
cmd_tree_node_t *activities_cmd();

// The App Switcher command
//
// Subcommands off this node deal with showing and hiding the App Switcher
// widget.
cmd_tree_node_t *app_switcher_cmd();

// The Workspace Switcher command
//
// Subcommands off this node deal with showing and hiding the Workspace Switcher
// widget.
cmd_tree_node_t *workspace_switcher_cmd();

// The Output Switcher command
//
// Subcommands off this node deal with showing and hiding the Output Switcher
// widget.
cmd_tree_node_t *output_switcher_cmd();

// The Workspace App Switcher command
//
// Subcommands off this node deal with showing and hiding the Workspace App
// Switcher widget.
cmd_tree_node_t *workspace_app_switcher_cmd();

// The Night Light command
//
// Subcommands off this node deal with enabling and disabling the Night Light
cmd_tree_node_t *bluelight_filter_cmd();

// The Rename Switcher command
//
// Subcommands off this node deal with showing and hiding the Rename Switcher
cmd_tree_node_t *rename_switcher_cmd();
