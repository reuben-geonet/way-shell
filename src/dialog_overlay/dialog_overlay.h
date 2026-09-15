#pragma once

#include <adwaita.h>

/* Temporary startup entry point; the dialog and responses are owned by Rust. */
void dialog_overlay_activate(AdwApplication *app, gpointer user_data);
