#pragma once
#include <adwaita.h>

G_BEGIN_DECLS
// Opaque identity owned by the Rust startup adapter until shutdown.
typedef struct _RenameSwitcher RenameSwitcher;
void rename_switcher_activate(AdwApplication *app, gpointer user_data);
RenameSwitcher *rename_switcher_get_global(void);
void rename_switcher_show(RenameSwitcher *self);
void rename_switcher_hide(RenameSwitcher *self);
void rename_switcher_toggle(RenameSwitcher *self);
G_END_DECLS
