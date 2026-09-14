#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/un.h>
#include <unistd.h>

#include "../lib/cmd_tree/include/cmd_tree.h"
#include "./commands.h"

#define SOCK_NAME "/way-shell.sock"

int client_socket_create(way_sh_ctx *ctx) {
    struct sockaddr_un address = {.sun_family = AF_UNIX};
    int fd = socket(AF_UNIX, SOCK_DGRAM | SOCK_CLOEXEC | SOCK_NONBLOCK, 0);
    if (fd < 0) return -1;
    // Linux autobind allocates an unused abstract address for each socket.
    if (bind(fd, (struct sockaddr *)&address, sizeof(address.sun_family)) < 0)
        goto fail;
    memcpy(address.sun_path, ctx->server_socket_path, strlen(ctx->server_socket_path) + 1);
    if (connect(fd, (struct sockaddr *)&address, sizeof(address)) < 0) goto fail;
    ctx->client_sock = fd;
    return 0;
fail:;
    int saved = errno;
    close(fd);
    errno = saved;
    return -1;
}

static void build_command_tree() {
    cmd_tree_node_t *message_tray = message_tray_cmd();
    cmd_tree_node_t *volume = volume_cmd();
    cmd_tree_node_t *brightness = brightness_cmd();
    cmd_tree_node_t *theme = theme_cmd();
    cmd_tree_node_t *activities = activities_cmd();
    cmd_tree_node_t *app_switcher = app_switcher_cmd();
    cmd_tree_node_t *workspace_switcher = workspace_switcher_cmd();
    cmd_tree_node_t *workspace_app_switcher = workspace_app_switcher_cmd();
    cmd_tree_node_t *output_switcher = output_switcher_cmd();
    cmd_tree_node_t *bluelight_filter = bluelight_filter_cmd();
	cmd_tree_node_t *rename_switcher = rename_switcher_cmd();

    cmd_tree_node_add_child(&root_cmd, message_tray);
    cmd_tree_node_add_child(&root_cmd, volume);
    cmd_tree_node_add_child(&root_cmd, brightness);
    cmd_tree_node_add_child(&root_cmd, theme);
    cmd_tree_node_add_child(&root_cmd, activities);
    cmd_tree_node_add_child(&root_cmd, app_switcher);
    cmd_tree_node_add_child(&root_cmd, workspace_switcher);
    cmd_tree_node_add_child(&root_cmd, workspace_app_switcher);
    cmd_tree_node_add_child(&root_cmd, output_switcher);
    cmd_tree_node_add_child(&root_cmd, bluelight_filter);
	cmd_tree_node_add_child(&root_cmd, rename_switcher);
}

static bool help_argument(const char *arg) {
    return strcmp(arg, "--help") == 0 || strcmp(arg, "-h") == 0;
}

int main(int argc, char **argv) {
    way_sh_ctx ctx = {.client_sock = -1};
    char socket_path[sizeof(((struct sockaddr_un *)0)->sun_path)];
    struct stat status;
    cmd_tree_node_t *cmd = NULL;
    build_command_tree();

    if (argc == 1 || (argc == 2 && help_argument(argv[1]))) {
        root_cmd.exec(NULL, 0, NULL);
        return 0;
    }
    if (argc > 256 || cmd_tree_search(&root_cmd, argc - 1, argv + 1, &cmd) != 1 ||
        !cmd || cmd == &root_cmd) {
        fprintf(stderr, "way-sh: unknown command or too many arguments\n");
        return 2;
    }
    if ((cmd->child && cmd->argc == 0) ||
        (cmd->argc == 1 && help_argument(cmd->argv[0]))) {
        if (cmd->child) cmd->exec(NULL, 0, NULL);
        else {
            printf("Usage: way-sh");
            for (int i = 1; i < argc - 1; i++) printf(" %s", argv[i]);
            puts(strcmp(cmd->name, "set") == 0 ? " <volume: 0.0-1.0>" : "");
        }
        return 0;
    }
    bool volume_set = strcmp(cmd->name, "set") == 0;
    float volume;
    if (cmd->child || cmd->argc != (volume_set ? 1 : 0) ||
        (volume_set && !way_sh_parse_volume(cmd->argv[0], &volume))) {
        fprintf(stderr, "way-sh: invalid arguments; use --help (volume must be finite, 0.0-1.0)\n");
        return 2;
    }

    const char *runtime = getenv("XDG_RUNTIME_DIR");
    if (!runtime || runtime[0] != '/' ||
        snprintf(socket_path, sizeof(socket_path), "%s%s", runtime, SOCK_NAME) >= sizeof(socket_path)) {
        fprintf(stderr, "way-sh: XDG_RUNTIME_DIR must be an absolute path that fits a Unix socket address\n");
        return 1;
    }
    if (stat(socket_path, &status) != 0) {
        perror("way-sh: cannot access shell socket");
        return 1;
    }
    if (!S_ISSOCK(status.st_mode)) {
        fprintf(stderr, "way-sh: path is not a socket: %s\n", socket_path);
        return 1;
    }
    ctx.server_socket_path = socket_path;
    if (client_socket_create(&ctx) < 0) {
        perror("way-sh: cannot connect to shell socket");
        return 1;
    }
    int result = cmd->exec(&ctx, cmd->argc, cmd->argv);
    close(ctx.client_sock);
    return result == 1 ? 0 : 1;
}
