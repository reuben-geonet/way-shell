#pragma once
#include <adwaita.h>

G_BEGIN_DECLS
// Opaque identity owned by the Rust startup adapter until shutdown.
typedef struct _WorkspaceSwitcher WorkspaceSwitcher;
void workspace_switcher_activate(AdwApplication *app, gpointer user_data);
WorkspaceSwitcher *workspace_switcher_get_global(void);
void workspace_switcher_show(WorkspaceSwitcher *self);
void workspace_switcher_hide(WorkspaceSwitcher *self);
void workspace_switcher_toggle(WorkspaceSwitcher *self);
void workspace_switcher_show_app_mode(WorkspaceSwitcher *self);
void workspace_switcher_toggle_app_mode(WorkspaceSwitcher *self);
G_END_DECLS
