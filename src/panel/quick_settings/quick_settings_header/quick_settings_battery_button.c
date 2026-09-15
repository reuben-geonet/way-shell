#include "quick_settings_battery_button.h"

#include <adwaita.h>

#include "../../../services/notifications_service/notifications_service.h"
#include "../../../services/upower_service.h"

typedef struct _QuickSettingsBatteryButton {
    GObject parent_instance;
    GtkBox *container;
    GtkImage *icon;
    GtkLabel *percentage;
    GtkButton *button;
    UPowerService *service;
    // 20% battery power left warning notification
    gboolean warning_notification_sent;
    // 15% battery power left warning notification
    gboolean critical_notification_sent;
    // 5% battery power left fata notification
    gboolean fata_notification_sent;
} QuickSettingsBatteryButton;
G_DEFINE_TYPE(QuickSettingsBatteryButton, quick_settings_battery_button,
              G_TYPE_OBJECT);

static void on_power_changed(UPowerService *service,
                             QuickSettingsBatteryButton *self) {
    UpDevice *power_dev = upower_service_get_primary_device(service);
    gtk_image_set_from_icon_name(self->icon, upower_device_map_icon_name(power_dev));
    gtk_widget_set_sensitive(GTK_WIDGET(self->button), power_dev != NULL);
    gtk_widget_set_tooltip_text(GTK_WIDGET(self->button),
                                power_dev ? NULL : "Power information is unavailable");
    if (!power_dev) {
        gtk_label_set_text(self->percentage, "—");
        return;
    }
    if (!upower_service_primary_is_bat(service)) {
        gtk_label_set_text(self->percentage, "AC");
        return;
    }
    if (!upower_service_primary_has_percentage(service)) {
        gtk_label_set_text(self->percentage, "—");
        return;
    }
    const char *icon = NULL;
    double percent = 0;

    NotificationsService *notifs = notifications_service_get_global();

    g_debug("quick_settings_battery_button.c:on_power_dev_notify() called.");

    g_object_get(power_dev, "percentage", &percent, NULL);

    icon = upower_device_map_icon_name(power_dev);

    gtk_image_set_from_icon_name(self->icon, icon);

    g_debug("quick_settings_battery_button.c:on_power_dev_notify() icon: %s",
            icon);

    g_autofree gchar *percentage = g_strdup_printf("%.0f%%", percent);
    gtk_label_set_text(self->percentage, percentage);

    // reset notification sent values
    if (percent > 5) {
        self->fata_notification_sent = false;
    }
    if (percent > 15) {
        self->critical_notification_sent = false;
    }
    if (percent > 20) {
        self->warning_notification_sent = false;
    }

    // if device is charging don't bother sending events below
    guint state = 0;
    g_object_get(power_dev, "state", &state, NULL);
    gboolean charging = (state == UP_DEVICE_STATE_CHARGING ||
                         state == UP_DEVICE_STATE_FULLY_CHARGED ||
                         state == UP_DEVICE_STATE_PENDING_CHARGE);
    if (charging || !notifs) return;

    gchar *body = g_strdup_printf("Battery is at %.0f%% power.", percent);

    // send a notification if percent is in specific range, and notification
    // was not sent
    if (percent <= 5 && !self->fata_notification_sent) {
        Notification n = {
            .app_name = "Way-Shell",
            .app_icon = "battery-level-0-symbolic",
            .summary = "Battery is critically low",
            .body = body,
            .urgency = 2,
            .is_internal = true,
        };
        notifications_service_send_notification(notifs, &n);
        self->fata_notification_sent = true;
    } else if (percent <= 15 && !self->critical_notification_sent) {
        Notification n = {
            .app_name = "Way-Shell",
            .summary = "Battery is low",
            .app_icon = "battery-level-10-symbolic",
            .body = body,
            .urgency = 2,
            .is_internal = true,
        };
        notifications_service_send_notification(notifs, &n);
        self->critical_notification_sent = true;
    } else if (percent <= 20 && !self->warning_notification_sent) {
        Notification n = {
            .app_name = "Way-Shell",
            .app_icon = "battery-level-20-symbolic",
            .summary = "Battery is low",
            .body = body,
            .urgency = 0,
            .is_internal = true,
        };
        notifications_service_send_notification(notifs, &n);
        self->warning_notification_sent = true;
    }

    g_free(body);
}

// stub out dispose, finalize, class_init and init methods.
static void quick_settings_battery_button_dispose(GObject *gobject) {
    QuickSettingsBatteryButton *self = QUICK_SETTINGS_BATTERY_BUTTON(gobject);

    g_debug(
        "quick_settings_battery_button.c:quick_settings_battery_button_dispose("
        ") called.");

    if (self->service) {
        g_signal_handlers_disconnect_by_func(self->service, on_power_changed, self);
        g_clear_object(&self->service);
    }

    // Chain-up
    G_OBJECT_CLASS(quick_settings_battery_button_parent_class)
        ->dispose(gobject);
};

static void quick_settings_battery_button_finalize(GObject *gobject) {
    // Chain-up
    G_OBJECT_CLASS(quick_settings_battery_button_parent_class)
        ->finalize(gobject);
};

static void quick_settings_battery_button_class_init(
    QuickSettingsBatteryButtonClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->dispose = quick_settings_battery_button_dispose;
    object_class->finalize = quick_settings_battery_button_finalize;
};

static void quick_settings_battery_button_init_layout(
    QuickSettingsBatteryButton *self) {
    self->service = g_object_ref(upower_service_get_global());

    self->container = GTK_BOX(gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0));
    gtk_widget_set_name(GTK_WIDGET(self->container),
                        "quick-settings-header-bat-button");

    self->button = GTK_BUTTON(gtk_button_new());
    // add css class
    gtk_widget_add_css_class(GTK_WIDGET(self->button), "battery-button");

    self->icon = GTK_IMAGE(gtk_image_new());

    self->percentage = GTK_LABEL(gtk_label_new(NULL));

    // add button icon as first child of battery button container
    gtk_box_append(self->container, GTK_WIDGET(self->icon));

    // add percent label as next child
    gtk_box_append(self->container, GTK_WIDGET(self->percentage));

    // add container as child of battery button
    gtk_button_set_child(self->button, GTK_WIDGET(self->container));

    g_signal_connect(self->service, "changed", G_CALLBACK(on_power_changed), self);
    on_power_changed(self->service, self);
}

static void quick_settings_battery_button_init(
    QuickSettingsBatteryButton *self) {
    quick_settings_battery_button_init_layout(self);
}

GtkWidget *quick_settings_battery_button_get_widget(
    QuickSettingsBatteryButton *self) {
    return GTK_WIDGET(self->button);
}

void quick_settings_battery_button_reinitialize(
    QuickSettingsBatteryButton *self) {
    if (self->service) {
        g_signal_handlers_disconnect_by_func(self->service, on_power_changed, self);
        g_clear_object(&self->service);
    }

    // init our layout
    quick_settings_battery_button_init_layout(self);
}

GtkButton *quick_settings_battery_button_get_button(
    QuickSettingsBatteryButton *self) {
    return self->button;
}
