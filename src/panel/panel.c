// Temporary startup adapter; the panel windows and widgets are owned by Rust.
#include "panel.h"
#include "message_tray/message_tray.h"
#include "quick_settings/quick_settings.h"

static PanelMediator *mediator;
extern int way_shell_panels_init(void (*message_tray)(void),
                                 void (*quick_settings)(void));

static void toggle_message_tray(void) {
    MessageTray *tray = message_tray_get_global();
    if (tray) message_tray_toggle(tray);
}
static void toggle_quick_settings(void) {
    QuickSettings *settings = quick_settings_get_global();
    if (settings) quick_settings_toggle(settings);
}
void panel_activate(AdwApplication *app, gpointer user_data) {
    if (way_shell_panels_init(toggle_message_tray, toggle_quick_settings) != 0)
        g_error("Failed to initialize Rust panels");
    mediator = g_object_new(PANEL_MEDIATOR_TYPE, NULL);
}
PanelMediator *panel_get_global_mediator(void) { return mediator; }
