#include "panel_status_bar_power_button.h"

#include <adwaita.h>
#include <upower.h>

#include "../../services/upower_service.h"

struct _PanelStatusBarPowerButton {
    GObject parent_instance;
    UPowerService *service;
    GtkImage *icon;
};
G_DEFINE_TYPE(PanelStatusBarPowerButton, panel_status_bar_power_button,
              G_TYPE_OBJECT);

static void on_power_changed(UPowerService *service,
                             PanelStatusBarPowerButton *self) {
    UpDevice *device = upower_service_get_primary_device(service);
    gtk_image_set_from_icon_name(self->icon, upower_device_map_icon_name(device));
    gtk_widget_set_tooltip_text(GTK_WIDGET(self->icon),
                                device ? NULL : "Power information is unavailable");
}

// stub out dispose, finalize, class_init, and init methods
static void panel_status_bar_power_button_dispose(GObject *gobject) {
    PanelStatusBarPowerButton *self = PANEL_STATUS_BAR_POWER_BUTTON(gobject);

    if (self->service) {
        g_signal_handlers_disconnect_by_func(self->service, on_power_changed, self);
        g_clear_object(&self->service);
    }

    // Chain-up
    G_OBJECT_CLASS(panel_status_bar_power_button_parent_class)
        ->dispose(gobject);
};

static void panel_status_bar_power_button_finalize(GObject *gobject) {
    // Chain-up
    G_OBJECT_CLASS(panel_status_bar_power_button_parent_class)
        ->finalize(gobject);
};

static void panel_status_bar_power_button_class_init(
    PanelStatusBarPowerButtonClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->dispose = panel_status_bar_power_button_dispose;
    object_class->finalize = panel_status_bar_power_button_finalize;
};

static void panel_status_bar_power_button_init_layout(
    PanelStatusBarPowerButton *self) {
    self->service = g_object_ref(upower_service_get_global());
    self->icon = GTK_IMAGE(gtk_image_new());
    g_signal_connect(self->service, "changed", G_CALLBACK(on_power_changed), self);
    on_power_changed(self->service, self);

};

static void panel_status_bar_power_button_init(
    PanelStatusBarPowerButton *self) {
    panel_status_bar_power_button_init_layout(self);
}

GtkWidget *panel_status_bar_power_button_get_widget(
    PanelStatusBarPowerButton *self) {
    return GTK_WIDGET(self->icon);
}