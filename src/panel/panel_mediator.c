#include "panel_mediator.h"

#include "../activities/activities.h"
#include "../output_switcher/output_switcher.h"
#include "../workspace_switcher/workspace_switcher.h"
#include "./../osd/osd.h"
#include "./message_tray/message_tray.h"
#include "panel.h"
#include "quick_settings/quick_settings.h"

enum signals { signals_n };

struct _PanelMediator {
    GObject parent_instance;
};
static guint signals[signals_n] = {0};
G_DEFINE_TYPE(PanelMediator, panel_mediator, G_TYPE_OBJECT);

static void panel_mediator_dispose(GObject *gobject) {
    PanelMediator *self = PANEL_MEDIATOR(gobject);
    // Chain-up
    G_OBJECT_CLASS(panel_mediator_parent_class)->dispose(gobject);
};

static void panel_mediator_finalize(GObject *gobject) {
    // Chain-up
    G_OBJECT_CLASS(panel_mediator_parent_class)->finalize(gobject);
};

static void panel_mediator_class_init(PanelMediatorClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->dispose = panel_mediator_dispose;
    object_class->finalize = panel_mediator_finalize;
};

static void panel_mediator_init(PanelMediator *self) {};

// hides any component which does not match the argument.
// this is the common case when a decoupled overlay component is opened, we want
// to hide all others.
static void hide_intelligent(gpointer component) {
    QuickSettings *qs = quick_settings_get_global();
    MessageTray *mt = message_tray_get_global();
    WorkspaceSwitcher *ws = workspace_switcher_get_global();
    OutputSwitcher *os = output_switcher_get_global();
    Activities *a = activities_get_global();

    if (component != qs) quick_settings_set_hidden(qs);
    if (component != mt) message_tray_set_hidden(mt);
    if (component != ws) workspace_switcher_hide(ws);
    if (component != os) output_switcher_hide(os);
    if (component != a) activities_hide(a);

    // hide osds in any case
    osd_set_hidden(osd_get_global());
}

static void on_message_tray_visible(MessageTray *tray) {
    panel_set_message_tray_visible(TRUE);
    hide_intelligent(tray);
}
static void on_message_tray_hidden(MessageTray *tray) {
    panel_set_message_tray_visible(FALSE);
}
static void on_quick_settings_visible(QuickSettings *qs) {
    panel_set_quick_settings_visible(TRUE);
    hide_intelligent(qs);
}
static void on_quick_settings_hidden(QuickSettings *qs) {
    panel_set_quick_settings_visible(FALSE);
}
static void on_overlay_visible(Activities *a, PanelMediator *self) {
    panel_set_activities_visible(TRUE);
    hide_intelligent(a);
}
static void on_overlay_hidden(Activities *a, PanelMediator *self) {
    panel_set_activities_visible(FALSE);
}

void panel_mediator_connect(PanelMediator *mediator) {
    MessageTray *mt = message_tray_get_global();
    g_signal_connect(mt, "message-tray-visible",
                     G_CALLBACK(on_message_tray_visible), mediator);
    g_signal_connect(mt, "message-tray-hidden",
                     G_CALLBACK(on_message_tray_hidden), mediator);

    QuickSettings *qs = quick_settings_get_global();
    g_signal_connect(qs, "quick-settings-visible",
                     G_CALLBACK(on_quick_settings_visible), mediator);
    g_signal_connect(qs, "quick-settings-hidden",
                     G_CALLBACK(on_quick_settings_hidden), mediator);

    Activities *a = activities_get_global();
    g_signal_connect(a, "activities-will-show", G_CALLBACK(on_overlay_visible),
                     mediator);
    g_signal_connect(a, "activities-will-hide", G_CALLBACK(on_overlay_hidden),
                     mediator);
};
