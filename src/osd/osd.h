#pragma once

#include <adwaita.h>

// Opaque handle owned by the Rust level-overlay controller.
typedef struct _OSD OSD;

void osd_activate(AdwApplication *app, gpointer user_data);

OSD *osd_get_global(void);

void osd_set_hidden(OSD *self);
