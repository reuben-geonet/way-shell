#pragma once

#include <gio/gio.h>

/* Parse an executable and arguments, without shell expansion. */
char **bluetooth_settings_parse_command(const char *command, GError **error);
gboolean bluetooth_settings_launch(const char *command, GError **error);
