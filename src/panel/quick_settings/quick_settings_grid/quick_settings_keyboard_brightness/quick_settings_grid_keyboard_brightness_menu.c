#include "quick_settings_grid_keyboard_brightness_menu.h"

#include <adwaita.h>

#include "../../../../services/brightness_service/brightness_service.h"
#include "../../quick_settings.h"
#include "../../quick_settings_menu_widget.h"
#include "gtk/gtk.h"
#include "quick_settings_grid_keyboard_brightness.h"

enum signals { signals_n };

typedef struct _QuickSettingsGridKeyboardBrightnessMenu {
    GObject parent_instance;
    QuickSettingsMenuWidget menu;
    GtkScale *brightness_slider;
} QuickSettingsGridKeyboardBrightnessMenu;
G_DEFINE_TYPE(QuickSettingsGridKeyboardBrightnessMenu,
              quick_settings_grid_keyboard_brightness_menu, G_TYPE_OBJECT);

static void on_keyboard_brightness_changed(
    BrightnessService *bs, guint32 brightness,
    QuickSettingsGridKeyboardBrightnessMenu *self);

// stub out dispose, finalize, init and class init functions for GObject
static void quick_settings_grid_keyboard_brightness_menu_dispose(
    GObject *object) {
    QuickSettingsGridKeyboardBrightnessMenu *self =
        QUICK_SETTINGS_GRID_KEYBOARD_BRIGHTNESS_MENU(object);

    // kill signals
    BrightnessService *bs = brightness_service_get_global();
    g_signal_handlers_disconnect_by_data(bs, self);

    G_OBJECT_CLASS(quick_settings_grid_keyboard_brightness_menu_parent_class)
        ->dispose(object);
}

static void quick_settings_grid_keyboard_brightness_menu_finalize(
    GObject *object) {}

static void quick_settings_grid_keyboard_brightness_menu_class_init(
    QuickSettingsGridKeyboardBrightnessMenuClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->dispose =
        quick_settings_grid_keyboard_brightness_menu_dispose;
    object_class->finalize =
        quick_settings_grid_keyboard_brightness_menu_finalize;
}

static void on_value_changed(GtkRange *range,
                             QuickSettingsGridKeyboardBrightnessMenu *self);

static void block_brightness_service_signals(
    QuickSettingsGridKeyboardBrightnessMenu *self, gboolean block) {
    BrightnessService *bs = brightness_service_get_global();
    if (block) {
        g_signal_handlers_block_by_func(bs, on_keyboard_brightness_changed,
                                        self);
    } else {
        g_signal_handlers_unblock_by_func(bs, on_keyboard_brightness_changed,
                                          self);
    }
}

static void block_brightness_scale_signals(
    QuickSettingsGridKeyboardBrightnessMenu *self, gboolean block) {
    if (block) {
        g_signal_handlers_block_by_func(self->brightness_slider,
                                        on_value_changed, self);
    } else {
        g_signal_handlers_unblock_by_func(self->brightness_slider,
                                          on_value_changed, self);
    }
}

static void on_keyboard_brightness_changed(
    BrightnessService *bs, guint32 brightness,
    QuickSettingsGridKeyboardBrightnessMenu *self) {
    g_debug(
        "quick_settings_grid_keyboard_brightness_menu.c:on_keyboard_brightness_"
        "changed() "
        "called: %d",
        brightness);

    // we are setting out scale, so ignore next scale event
    block_brightness_scale_signals(self, true);

    gtk_range_set_value(GTK_RANGE(self->brightness_slider), brightness);

    block_brightness_scale_signals(self, false);
}

static void on_value_changed(GtkRange *range,
                             QuickSettingsGridKeyboardBrightnessMenu *self) {
    g_debug(
        "quick_settings_grid_keyboard_brightness_menu.c:on_value_changed() "
        "called: %f",
        gtk_range_get_value(range));

    // we are writing to brightness API so ignore next brightness service
    // event

    block_brightness_service_signals(self, true);

    BrightnessService *bs = brightness_service_get_global();
    brightness_service_set_keyboard(bs, gtk_range_get_value(range));

    block_brightness_service_signals(self, false);
}

static void on_brightness_availability(
    BrightnessService *bs, QuickSettingsGridKeyboardBrightnessMenu *self) {
    guint maximum = brightness_service_get_keyboard_max(bs);
    gboolean available = brightness_service_has_keyboard_brightness(bs);
    block_brightness_scale_signals(self, true);
    gtk_range_set_range(GTK_RANGE(self->brightness_slider), 0, MAX(maximum, 1));
    gtk_scale_clear_marks(self->brightness_slider);
    // Ordinary keyboards have a small discrete range. Avoid thousands of GTK
    // objects if a device exposes a larger numeric range.
    if (maximum <= 20) {
        for (guint i = 0; i <= maximum; i++)
            gtk_scale_add_mark(self->brightness_slider, i, GTK_POS_BOTTOM, NULL);
    }
    gtk_range_set_value(GTK_RANGE(self->brightness_slider), brightness_service_get_keyboard(bs));
    gtk_widget_set_sensitive(GTK_WIDGET(self->brightness_slider), available);
    gtk_widget_set_tooltip_text(GTK_WIDGET(self->brightness_slider),
                                available ? NULL : "Keyboard brightness is unavailable");
    block_brightness_scale_signals(self, false);
}

static void on_brightness_failed(BrightnessService *bs, const gchar *error,
    QuickSettingsGridKeyboardBrightnessMenu *self) {
    on_keyboard_brightness_changed(bs, brightness_service_get_keyboard(bs), self);
    gtk_widget_set_tooltip_text(GTK_WIDGET(self->brightness_slider), error);
}

static void quick_settings_grid_keyboard_brightness_menu_init_layout(
    QuickSettingsGridKeyboardBrightnessMenu *self) {
    quick_settings_menu_widget_init(&self->menu, false);
    gtk_image_set_from_icon_name(self->menu.icon,
                                 "keyboard-brightness-symbolic");
    gtk_label_set_text(self->menu.title, "Keyboard Backlight");

    BrightnessService *bs = brightness_service_get_global();
    guint32 keyboard_max = brightness_service_get_keyboard_max(bs);
    guint32 keyboard_cur = brightness_service_get_keyboard(bs);

    self->brightness_slider = GTK_SCALE(gtk_scale_new_with_range(
        GTK_ORIENTATION_HORIZONTAL, 0, MAX(keyboard_max, 1), 1.0));
    gtk_range_set_round_digits(GTK_RANGE(self->brightness_slider), 0);

    gtk_range_set_value(GTK_RANGE(self->brightness_slider), keyboard_cur);

    // connect signal
    g_signal_connect_object(self->brightness_slider, "value-changed",
                            G_CALLBACK(on_value_changed), self, 0);

    g_signal_connect(bs, "keyboard-brightness-changed",
                     G_CALLBACK(on_keyboard_brightness_changed), self);
    g_signal_connect(bs, "availability-changed", G_CALLBACK(on_brightness_availability), self);
    g_signal_connect(bs, "operation-failed", G_CALLBACK(on_brightness_failed), self);
    on_brightness_availability(bs, self);

    gtk_box_append(self->menu.options, GTK_WIDGET(self->brightness_slider));
}

static void quick_settings_grid_keyboard_brightness_menu_init(
    QuickSettingsGridKeyboardBrightnessMenu *self) {
    quick_settings_grid_keyboard_brightness_menu_init_layout(self);
}

void quick_settings_grid_keyboard_brightness_menu_on_reveal(
    QuickSettingsGridButton *button_, gboolean is_revealed) {
    QuickSettingsGridKeyboardBrightnessButton *button =
        (QuickSettingsGridKeyboardBrightnessButton *)button_;
    QuickSettingsGridKeyboardBrightnessMenu *self = button->menu;
    g_debug(
        "quick_settings_grid_keyboard_brightness_menu.c:on_reveal() called");
}

GtkWidget *quick_settings_grid_keyboard_brightness_menu_get_widget(
    QuickSettingsGridKeyboardBrightnessMenu *self) {
    return GTK_WIDGET(self->menu.container);
}

GtkScale *quick_settings_grid_keyboard_brightness_menu_get_temp_scale(
    QuickSettingsGridKeyboardBrightnessMenu *grid) {
    return grid->brightness_slider;
}
