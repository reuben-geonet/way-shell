#pragma once

#include <adwaita.h>

G_BEGIN_DECLS
// Opaque identity owned by the Rust startup adapter until shutdown.
typedef struct _AppSwitcher AppSwitcher;
void app_switcher_activate(AdwApplication *app, gpointer user_data);
AppSwitcher *app_switcher_get_global(void);
void app_switcher_show(AppSwitcher *self);
void app_switcher_hide(AppSwitcher *self);
void app_switcher_toggle(AppSwitcher *self);
G_END_DECLS
