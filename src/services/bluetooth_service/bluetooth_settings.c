#include "bluetooth_settings.h"

char **bluetooth_settings_parse_command(const char *command, GError **error) {
    char **argv = NULL;
    if (!command || !g_shell_parse_argv(command, NULL, &argv, error)) {
        if (!command)
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "No Bluetooth settings command configured");
        return NULL;
    }
    g_autofree char *executable = g_find_program_in_path(argv[0]);
    if (!executable) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                    "Cannot find %s. Install it or change the Bluetooth "
                    "settings command.", argv[0]);
        g_strfreev(argv);
        return NULL;
    }
    g_free(argv[0]);
    argv[0] = g_steal_pointer(&executable);
    return argv;
}

gboolean bluetooth_settings_launch(const char *command, GError **error) {
    g_auto(GStrv) argv = bluetooth_settings_parse_command(command, error);
    if (!argv) return FALSE;
    return g_spawn_async(NULL, argv, NULL, G_SPAWN_DEFAULT, NULL, NULL, NULL,
                         error);
}
