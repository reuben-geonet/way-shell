#include "./quick_settings_grid_idle_inhibitor.h"

#include <adwaita.h>

#include "../../../services/logind_service/logind_service.h"
#include "./quick_settings_grid_button.h"

static void on_toggle_button_clicked(
    GtkButton *button, QuickSettingsGridIdleInhibitorButton *self) {
    g_debug(
        "quick_settings_grid_idle_inhibitor.c: on_toggle_button_clicked(): "
        "toggling idle inhibitor");

    LogindService *logind = self->service;

    gboolean inhibit = logind_service_get_idle_inhibit(logind);
    if (!inhibit)
        logind_service_set_idle_inhibit(logind, true);
    else
        logind_service_set_idle_inhibit(logind, false);
}

static void on_idle_inhibit_changed(
    LogindService *logind_service, gboolean inhibited,
    QuickSettingsGridIdleInhibitorButton *self) {
    g_debug(
        "quick_settings_grid_idle_inhibitor.c: on_idle_inhibit_changed(): "
        "idle inhibitor changed: %d",
        inhibited);

    if (inhibited)
        quick_settings_grid_button_set_toggled(&self->button, true);
    else
        quick_settings_grid_button_set_toggled(&self->button, false);
}

static void on_availability_changed(LogindService *service, gboolean available,
                                    QuickSettingsGridIdleInhibitorButton *self) {
    gtk_widget_set_sensitive(GTK_WIDGET(self->button.toggle), available);
}

QuickSettingsGridIdleInhibitorButton *
quick_settings_grid_idle_inhibitor_button_init() {
    QuickSettingsGridIdleInhibitorButton *self =
        g_malloc0(sizeof(QuickSettingsGridIdleInhibitorButton));

    quick_settings_grid_button_init(
        &self->button, QUICK_SETTINGS_BUTTON_IDLE_INHIBITOR, "Idle Inhibitor",
        NULL, "radio-mixed-symbolic", NULL, NULL);

    g_signal_connect(self->button.toggle, "clicked",
                     G_CALLBACK(on_toggle_button_clicked), self);

    // Retain the service while callbacks reference this button.
    self->service = g_object_ref(logind_service_get_global());
    LogindService *logind = self->service;
    gboolean inhibit = logind_service_get_idle_inhibit(logind);

    on_idle_inhibit_changed(logind, inhibit, self);

    // listen for idle_inhibitor_changed
    g_signal_connect(logind, "idle-inhibitor-changed",
                     G_CALLBACK(on_idle_inhibit_changed), self);
    g_signal_connect(logind, "availability-changed",
                     G_CALLBACK(on_availability_changed), self);
    on_availability_changed(logind, logind_service_get_enabled(logind), self);

    return self;
}

void quick_settings_grid_inhibitor_button_free(
    QuickSettingsGridIdleInhibitorButton *self) {
    // unref button's container
    g_object_unref(self->button.container);

    // kill signals
    LogindService *logind = self->service;
    g_signal_handlers_disconnect_by_data(logind, self);
    g_clear_object(&self->service);

    // free ourselves
    g_free(self);
}
