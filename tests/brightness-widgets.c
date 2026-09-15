#include <adwaita.h>
#include "../src/panel/quick_settings/quick_settings_grid/quick_settings_keyboard_brightness/quick_settings_grid_keyboard_brightness_menu.c"

typedef struct { GObject parent_instance; } FixtureService;
typedef struct { GObjectClass parent_class; } FixtureServiceClass;
G_DEFINE_TYPE(FixtureService, fixture_service, G_TYPE_OBJECT)
static void fixture_service_class_init(FixtureServiceClass *klass) {
    GType type = G_TYPE_FROM_CLASS(klass);
    g_signal_new("availability-changed", type, G_SIGNAL_RUN_LAST, 0,
                 NULL, NULL, NULL, G_TYPE_NONE, 0);
    g_signal_new("keyboard-brightness-changed", type, G_SIGNAL_RUN_LAST, 0,
                 NULL, NULL, NULL, G_TYPE_NONE, 1, G_TYPE_UINT);
    g_signal_new("operation-failed", type, G_SIGNAL_RUN_LAST, 0,
                 NULL, NULL, NULL, G_TYPE_NONE, 1, G_TYPE_STRING);
}
static void fixture_service_init(FixtureService *self) {}
static GObject *fixture;
static guint current, maximum, requests;
BrightnessService *brightness_service_get_global(void) { return (BrightnessService *)fixture; }
guint32 brightness_service_get_keyboard(BrightnessService *self) { return current; }
guint32 brightness_service_get_keyboard_max(BrightnessService *self) { return maximum; }
gboolean brightness_service_has_keyboard_brightness(BrightnessService *self) { return maximum > 0; }
void brightness_service_set_keyboard(BrightnessService *self, guint32 value) { requests++; }
static void lifecycle(void) {
    fixture = g_object_new(fixture_service_get_type(), NULL);
    for (unsigned cycle = 0; cycle < 3; ++cycle) {
        current = maximum = requests = 0;
        QuickSettingsGridKeyboardBrightnessMenu *menu = g_object_new(QUICK_SETTINGS_GRID_KEYBOARD_BRIGHTNESS_MENU_TYPE, NULL);
        GtkWidget *container = g_object_ref_sink(quick_settings_grid_keyboard_brightness_menu_get_widget(menu));
        GtkRange *slider = GTK_RANGE(menu->brightness_slider);
        g_assert_false(gtk_widget_get_sensitive(GTK_WIDGET(slider)));
        current = 1; maximum = 3;
        g_signal_emit_by_name(fixture, "availability-changed");
        g_assert_true(gtk_widget_get_sensitive(GTK_WIDGET(slider)));
        g_assert_cmpfloat(gtk_range_get_value(slider), ==, 1);
        g_assert_cmpfloat(gtk_adjustment_get_upper(gtk_range_get_adjustment(slider)), ==, 3);
        gtk_range_set_value(slider, 3);
        g_assert_cmpuint(requests, ==, 1);
        g_signal_emit_by_name(fixture, "operation-failed", "Permission denied");
        g_assert_cmpfloat(gtk_range_get_value(slider), ==, 1);
        g_assert_cmpuint(requests, ==, 1);
        maximum = 0;
        g_signal_emit_by_name(fixture, "availability-changed");
        g_assert_false(gtk_widget_get_sensitive(GTK_WIDGET(slider)));
        gpointer old_menu = menu;
        g_object_unref(menu);
        g_assert_cmpuint(g_signal_handlers_block_matched(fixture,
            G_SIGNAL_MATCH_DATA, 0, 0, NULL, NULL, old_menu), ==, 0);
        gtk_range_set_value(slider, 0);
        g_assert_cmpuint(requests, ==, 1);
        g_signal_emit_by_name(fixture, "availability-changed");
        g_object_unref(container);
    }
    g_object_unref(fixture);
}
int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    gtk_init();
    g_test_add_func("/brightness-menu/availability-failure-lifetime", lifecycle);
    return g_test_run();
}
