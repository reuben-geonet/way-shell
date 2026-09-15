#pragma once
#include <adwaita.h>

G_BEGIN_DECLS
// Opaque identity owned by the Rust startup adapter until shutdown.
typedef struct _OutputSwitcher OutputSwitcher;
void output_switcher_activate(AdwApplication *app, gpointer user_data);
OutputSwitcher *output_switcher_get_global(void);
void output_switcher_show(OutputSwitcher *self);
void output_switcher_hide(OutputSwitcher *self);
void output_switcher_toggle(OutputSwitcher *self);
G_END_DECLS
