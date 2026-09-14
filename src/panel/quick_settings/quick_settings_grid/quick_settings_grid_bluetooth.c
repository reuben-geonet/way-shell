#include "quick_settings_grid_bluetooth.h"

#include "../../../services/bluetooth_service/bluetooth_service.h"
#include "../../../services/bluetooth_service/bluetooth_settings.h"
#include "../quick_settings.h"
#include "../quick_settings_menu_widget.h"

struct _QuickSettingsGridBluetoothButton {
    QuickSettingsGridButton button;
    QuickSettingsMenuWidget menu;
    BluetoothService *service;
    GSettings *settings;
    GHashTable *rows;
    GtkLabel *placeholder;
    GtkLabel *error;
    GtkButton *settings_button;
    gboolean last_powered;
    gboolean last_ready;
};

typedef struct {
    QuickSettingsGridBluetoothButton *menu;
    char *path;
    GtkButton *button;
    GtkImage *icon;
    GtkLabel *name;
    GtkLabel *action;
    GtkSpinner *spinner;
} DeviceRow;

static void set_accessible_label(GtkWidget *widget, const char *label) {
    gtk_accessible_update_property(GTK_ACCESSIBLE(widget),
                                    GTK_ACCESSIBLE_PROPERTY_LABEL, label, -1);
}

static void row_free(gpointer data) {
    DeviceRow *row = data;
    gtk_box_remove(row->menu->menu.options, GTK_WIDGET(row->button));
    g_free(row->path);
    g_free(row);
}

static void row_clicked(GtkButton *button, DeviceRow *row) {
    bluetooth_service_toggle_device(row->menu->service, row->path);
}

static DeviceRow *row_new(QuickSettingsGridBluetoothButton *self,
                          BluetoothDevice *device) {
    DeviceRow *row = g_new0(DeviceRow, 1);
    row->menu = self;
    row->path = g_strdup(device->path);
    row->button = GTK_BUTTON(gtk_button_new());
    gtk_widget_add_css_class(GTK_WIDGET(row->button),
                             "quick-settings-menu-option-bluetooth");
    GtkBox *box = GTK_BOX(gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0));
    row->icon = GTK_IMAGE(gtk_image_new());
    gtk_image_set_pixel_size(row->icon, 20);
    row->name = GTK_LABEL(gtk_label_new(NULL));
    gtk_widget_set_hexpand(GTK_WIDGET(row->name), TRUE);
    gtk_label_set_xalign(row->name, 0);
    gtk_label_set_ellipsize(row->name, PANGO_ELLIPSIZE_END);
    gtk_label_set_max_width_chars(row->name, 24);
    row->action = GTK_LABEL(gtk_label_new(NULL));
    gtk_widget_add_css_class(GTK_WIDGET(row->action), "bluetooth-action");
    row->spinner = GTK_SPINNER(gtk_spinner_new());
    gtk_widget_set_size_request(GTK_WIDGET(row->spinner), 16, 16);
    gtk_box_append(box, GTK_WIDGET(row->icon));
    gtk_box_append(box, GTK_WIDGET(row->name));
    gtk_box_append(box, GTK_WIDGET(row->action));
    gtk_box_append(box, GTK_WIDGET(row->spinner));
    gtk_button_set_child(row->button, GTK_WIDGET(box));
    gtk_box_append(self->menu.options, GTK_WIDGET(row->button));
    g_signal_connect(row->button, "clicked", G_CALLBACK(row_clicked), row);
    return row;
}

static void sync_menu(QuickSettingsGridBluetoothButton *self,
                      gboolean reorder) {
    gboolean powered = bluetooth_service_powered(self->service);
    gboolean busy = bluetooth_service_busy(self->service);
    gboolean ready = bluetooth_service_ready(self->service);
    gboolean blocked = bluetooth_service_hardware_blocked(self->service);
    if (powered != self->last_powered || (ready && !self->last_ready))
        gtk_revealer_set_reveal_child(self->menu.banner, FALSE);
    self->last_powered = powered;
    self->last_ready = ready;
    g_autoptr(GPtrArray) devices = bluetooth_service_get_devices(self->service);
    g_autoptr(GHashTable) present = g_hash_table_new(g_str_hash, g_str_equal);
    guint connected = 0;
    const char *single_name = NULL;
    GtkWidget *previous = NULL;
    for (guint i = 0; i < devices->len; i++) {
        BluetoothDevice *device = g_ptr_array_index(devices, i);
        g_hash_table_add(present, device->path);
        DeviceRow *row = g_hash_table_lookup(self->rows, device->path);
        if (!row) {
            row = row_new(self, device);
            g_hash_table_insert(self->rows, g_strdup(device->path), row);
        }
        /* Prefer a symbolic device icon, as GNOME does, with a themed fallback. */
        g_autofree char *symbolic = g_str_has_suffix(device->icon, "-symbolic")
            ? g_strdup(device->icon) : g_strconcat(device->icon, "-symbolic", NULL);
        const char *names[] = {symbolic, device->icon, "bluetooth-symbolic", NULL};
        g_autoptr(GIcon) icon = g_themed_icon_new_from_names((char **)names, -1);
        gtk_image_set_from_gicon(row->icon, icon);
        gtk_label_set_text(row->name, device->alias);
        gtk_label_set_text(row->action, device->connected ? "Disconnect" : "Connect");
        gtk_widget_set_visible(GTK_WIDGET(row->action), !device->busy);
        gtk_widget_set_visible(GTK_WIDGET(row->spinner), device->busy);
        gtk_spinner_set_spinning(row->spinner, device->busy);
        gtk_widget_set_sensitive(GTK_WIDGET(row->button), !device->busy && !busy);
        g_autofree char *label = g_strdup_printf("%s %s",
            device->connected ? "Disconnect" : "Connect to", device->alias);
        set_accessible_label(GTK_WIDGET(row->button), label);
        if (reorder)
            gtk_box_reorder_child_after(self->menu.options,
                                         GTK_WIDGET(row->button), previous);
        previous = GTK_WIDGET(row->button);
        if (device->connected) {
            connected++;
            single_name = device->alias;
        }
    }
    GHashTableIter iter;
    gpointer key;
    g_hash_table_iter_init(&iter, self->rows);
    while (g_hash_table_iter_next(&iter, &key, NULL))
        if (!g_hash_table_contains(present, key)) g_hash_table_iter_remove(&iter);

    gtk_widget_set_visible(GTK_WIDGET(self->placeholder), !devices->len);
    gtk_widget_set_visible(GTK_WIDGET(self->menu.scroll), devices->len > 0);
    gtk_label_set_text(self->placeholder,
        !ready ? "Bluetooth service unavailable" :
        blocked ? "Bluetooth is disabled by a hardware switch" :
        powered ? "No available or connected devices" :
                  "Turn on Bluetooth to connect to devices");
    g_autofree char *subtitle = connected > 1
        ? g_strdup_printf("%u Connected", connected) : g_strdup(single_name);
    gtk_label_set_text(self->button.subtitle, subtitle ? subtitle : "");
    gtk_widget_set_visible(GTK_WIDGET(self->button.subtitle), subtitle != NULL);
    /* Center the text block against the taller icon for both one and two
     * lines, without making the whole button expand vertically. */
    gtk_widget_set_valign(gtk_widget_get_parent(GTK_WIDGET(self->button.title)),
                          GTK_ALIGN_CENTER);
    if (subtitle)
        gtk_widget_add_css_class(GTK_WIDGET(self->button.toggle), "with-subtitle");
    else
        gtk_widget_remove_css_class(GTK_WIDGET(self->button.toggle), "with-subtitle");
    quick_settings_grid_button_set_toggled(&self->button, powered);
    /* GNOME uses this symbolic Bluetooth glyph with three dots for both
     * transitions. The service stays busy through adapter re-enumeration. */
    const char *icon_name = busy ? "bluetooth-acquiring-symbolic" :
        powered ? "bluetooth-active-symbolic" : "bluetooth-disabled-symbolic";
    gtk_image_set_from_icon_name(self->button.icon, icon_name);
    quick_settings_menu_widget_set_icon(&self->menu, icon_name);
    gboolean target = bluetooth_service_target_powered(self->service);
    gtk_widget_set_tooltip_text(GTK_WIDGET(self->button.toggle), busy
        ? (target ? "Turning Bluetooth on…" : "Turning Bluetooth off…") : NULL);
    gtk_widget_set_sensitive(GTK_WIDGET(self->button.toggle), ready && !blocked);
    set_accessible_label(GTK_WIDGET(self->button.toggle),
                         target ? "Turn Bluetooth off" : "Turn Bluetooth on");
}

static void on_changed(BluetoothService *service,
                        QuickSettingsGridBluetoothButton *self) {
    sync_menu(self, !gtk_revealer_get_reveal_child(self->button.revealer));
}

static void on_revealed(GObject *revealer, GParamSpec *pspec,
                         QuickSettingsGridBluetoothButton *self) {
    if (gtk_revealer_get_reveal_child(self->button.revealer)) sync_menu(self, TRUE);
}

static void on_error(BluetoothService *service, const char *message,
                      QuickSettingsGridBluetoothButton *self) {
    gtk_label_set_text(self->error, message);
    gtk_revealer_set_reveal_child(self->menu.banner, TRUE);
    if (!gtk_revealer_get_reveal_child(self->button.revealer) &&
        quick_settings_is_visible(quick_settings_get_global()))
        g_signal_emit_by_name(self->button.reveal_button, "clicked");
}

static void dismiss_error(GtkButton *button,
                          QuickSettingsGridBluetoothButton *self) {
    gtk_revealer_set_reveal_child(self->menu.banner, FALSE);
}

static void on_succeeded(BluetoothService *service,
                          QuickSettingsGridBluetoothButton *self) {
    gtk_revealer_set_reveal_child(self->menu.banner, FALSE);
}

static void toggle_power(GtkButton *button,
                         QuickSettingsGridBluetoothButton *self) {
    gtk_revealer_set_reveal_child(self->menu.banner, FALSE);
    bluetooth_service_set_powered(self->service,
                                  !bluetooth_service_target_powered(self->service));
}

static void launch_settings(GtkButton *button,
                            QuickSettingsGridBluetoothButton *self) {
    g_autofree char *command = g_settings_get_string(self->settings,
                                                    "bluetooth-settings-command");
    g_autoptr(GError) error = NULL;
    if (!bluetooth_settings_launch(command, &error)) {
        on_error(self->service, error->message, self);
        return;
    }
    QuickSettings *qs = quick_settings_get_global();
    if (quick_settings_is_visible(qs)) quick_settings_toggle(qs);
}

QuickSettingsGridBluetoothButton *quick_settings_grid_bluetooth_button_init(BluetoothService *service) {
    QuickSettingsGridBluetoothButton *self =
        g_new0(QuickSettingsGridBluetoothButton, 1);
    self->service = g_object_ref(service);
    self->settings = g_settings_new("org.ldelossa.way-shell.system");
    self->rows = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, row_free);
    quick_settings_menu_widget_init(&self->menu, TRUE);
    /* This menu sizes to its devices; the shared menu defaults to filling
     * the available height, leaving an empty viewport/footer gap. */
    gtk_widget_set_vexpand(GTK_WIDGET(self->menu.container), FALSE);
    gtk_widget_set_vexpand(GTK_WIDGET(self->menu.options_container), FALSE);
    quick_settings_menu_widget_set_title(&self->menu, "Bluetooth");
    quick_settings_menu_widget_set_icon(&self->menu, "bluetooth-active-symbolic");
    gtk_scrolled_window_set_min_content_height(self->menu.scroll, 0);
    gtk_scrolled_window_set_max_content_height(self->menu.scroll, 240);
    gtk_scrolled_window_set_propagate_natural_height(self->menu.scroll, TRUE);
    gtk_widget_set_vexpand(GTK_WIDGET(self->menu.scroll), FALSE);
    self->placeholder = GTK_LABEL(gtk_label_new(NULL));
    gtk_label_set_wrap(self->placeholder, TRUE);
    /* GtkRevealer still contributes its child's natural width while closed.
     * Wrap the padded placeholder instead of widening the entire grid. */
    gtk_label_set_max_width_chars(self->placeholder, 20);
    gtk_label_set_justify(self->placeholder, GTK_JUSTIFY_CENTER);
    gtk_widget_add_css_class(GTK_WIDGET(self->placeholder), "bluetooth-placeholder");
    gtk_box_append(self->menu.options_container, GTK_WIDGET(self->placeholder));
    GtkWidget *separator = gtk_separator_new(GTK_ORIENTATION_HORIZONTAL);
    gtk_widget_add_css_class(separator, "bluetooth-separator");
    gtk_box_append(self->menu.options_container, separator);
    self->settings_button = GTK_BUTTON(gtk_button_new_with_label("Bluetooth Settings"));
    gtk_widget_add_css_class(GTK_WIDGET(self->settings_button), "bluetooth-settings");
    gtk_label_set_xalign(GTK_LABEL(gtk_button_get_child(self->settings_button)), 0);
    gtk_box_append(self->menu.options_container, GTK_WIDGET(self->settings_button));
    g_signal_connect(self->settings_button, "clicked", G_CALLBACK(launch_settings), self);

    GtkButton *error_button = GTK_BUTTON(gtk_button_new());
    self->error = GTK_LABEL(gtk_label_new(NULL));
    gtk_label_set_wrap(self->error, TRUE);
    gtk_label_set_max_width_chars(self->error, 38);
    gtk_button_set_child(error_button, GTK_WIDGET(self->error));
    gtk_widget_add_css_class(GTK_WIDGET(error_button), "failure-banner");
    set_accessible_label(GTK_WIDGET(error_button), "Dismiss Bluetooth error");
    gtk_revealer_set_child(self->menu.banner, GTK_WIDGET(error_button));
    g_signal_connect(error_button, "clicked", G_CALLBACK(dismiss_error), self);

    quick_settings_grid_button_init(&self->button, QUICK_SETTINGS_BUTTON_BLUETOOTH,
        "Bluetooth", "", "bluetooth-disabled-symbolic",
        GTK_WIDGET(self->menu.container), NULL);
    /* Own the revealer until this button is freed, including grid removal. */
    g_object_ref_sink(self->button.revealer);
    set_accessible_label(GTK_WIDGET(self->button.reveal_button), "Open Bluetooth menu");
    g_signal_connect(self->button.toggle, "clicked", G_CALLBACK(toggle_power), self);
    g_signal_connect(self->button.revealer, "notify::reveal-child",
                      G_CALLBACK(on_revealed), self);
    g_signal_connect(self->service, "changed", G_CALLBACK(on_changed), self);
    g_signal_connect(self->service, "operation-error", G_CALLBACK(on_error), self);
    g_signal_connect(self->service, "operation-succeeded", G_CALLBACK(on_succeeded), self);
    sync_menu(self, TRUE);
    return self;
}

void quick_settings_grid_bluetooth_button_free(QuickSettingsGridBluetoothButton *self) {
    g_signal_handlers_disconnect_by_data(self->service, self);
    g_signal_handlers_disconnect_by_data(self->button.revealer, self);
    g_hash_table_unref(self->rows);
    g_object_unref(self->settings);
    g_object_unref(self->service);
    g_object_unref(self->button.revealer);
    g_object_unref(self->button.container);
    g_free(self);
}
