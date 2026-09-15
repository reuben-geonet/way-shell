#pragma once
#include <adwaita.h>
#include "panel_mediator.h"

// Temporary entry points while startup and popup mediation remain in C.
void panel_activate(AdwApplication *app, gpointer user_data);
PanelMediator *panel_get_global_mediator(void);
void panel_set_message_tray_visible(gboolean visible);
void panel_set_quick_settings_visible(gboolean visible);
void panel_set_activities_visible(gboolean visible);
