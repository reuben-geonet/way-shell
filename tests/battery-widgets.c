/* Construct the actual GTK menu against inert services; no host D-Bus access. */
#include <adwaita.h>
#include <upower.h>
#include "../src/panel/quick_settings/quick_settings_header/quick_settings_battery_menu.c"

typedef struct { GObject parent_instance; } FixtureService;
typedef struct { GObjectClass parent_class; } FixtureServiceClass;
G_DEFINE_TYPE(FixtureService, fixture_service, G_TYPE_OBJECT)
static void fixture_service_class_init(FixtureServiceClass *klass) {
    g_signal_new("changed", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST, 0,
                 NULL, NULL, NULL, G_TYPE_NONE, 0);
    g_signal_new("quick-settings-hidden", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST, 0,
                 NULL, NULL, NULL, G_TYPE_NONE, 0);
}
static void fixture_service_init(FixtureService *self) {}

static GObject *power_fixture;
static GObject *settings_fixture;
static UpDevice *device_fixture;
UPowerService *upower_service_get_global(void) { return (UPowerService *)power_fixture; }
UpDevice *upower_service_get_primary_device(UPowerService *self) { return device_fixture; }
gboolean upower_service_primary_is_bat(UPowerService *self) { return TRUE; }
gboolean upower_service_primary_has_percentage(UPowerService *self) { return TRUE; }
/* This fixture spans the old owned-string and new borrowed-string declarations. */
__typeof__(upower_device_map_icon_name(NULL)) upower_device_map_icon_name(UpDevice *device) {
    return "battery-level-50-symbolic";
}
QuickSettings *quick_settings_get_global(void) { return (QuickSettings *)settings_fixture; }
static void destroyed(gpointer data, GObject *object) { *(gboolean *)data = TRUE; }

static void menu_construction_updates_reinitialize_and_destruction(void) {
    power_fixture = g_object_new(fixture_service_get_type(), NULL);
    settings_fixture = g_object_new(fixture_service_get_type(), NULL);
    device_fixture = up_device_new();
    for (unsigned i = 0; i < 3; ++i) {
        g_object_set(device_fixture, "is-rechargeable", TRUE,
                      "percentage", 50.0, "state", UP_DEVICE_STATE_DISCHARGING,
                      "time-to-empty", (gint64)3600, NULL);
        QuickSettingsBatteryMenu *menu = g_object_new(QUICK_SETTINGS_BATTERY_MENU_TYPE, NULL);
        g_assert_true(G_IS_OBJECT(menu));
        gboolean finalized = FALSE;
        g_object_weak_ref(G_OBJECT(menu), destroyed, &finalized);
        GtkWidget *container = g_object_ref_sink(quick_settings_battery_menu_get_widget(menu));
        g_assert_true(GTK_IS_BOX(container));
        g_assert_cmpstr(gtk_label_get_text(menu->battery_percentage), ==, "50%");
        g_object_set(device_fixture, "percentage", 75.0, NULL);
        g_signal_emit_by_name(power_fixture, "changed");
        g_assert_cmpstr(gtk_label_get_text(menu->battery_percentage), ==, "75%");
        quick_settings_battery_menu_reinitialize(menu);
        GtkWidget *replacement = g_object_ref_sink(quick_settings_battery_menu_get_widget(menu));
        g_assert_true(container != replacement);
        g_object_unref(container);
        g_signal_emit_by_name(settings_fixture, "quick-settings-hidden");
        g_object_unref(menu);
        g_assert_true(finalized);
        /* These signals must have no dangling callbacks into the disposed menu. */
        g_object_set(device_fixture, "percentage", 60.0, NULL);
        g_signal_emit_by_name(power_fixture, "changed");
        g_signal_emit_by_name(settings_fixture, "quick-settings-hidden");
        g_object_unref(replacement);
        g_assert_cmpuint(power_fixture->ref_count, ==, 1);
    }
    g_clear_object(&device_fixture);
    g_clear_object(&power_fixture);
    g_clear_object(&settings_fixture);
}
int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    gtk_init();
    g_test_add_func("/battery-menu/construct-update-reinitialize-destroy",
                    menu_construction_updates_reinitialize_and_destruction);
    return g_test_run();
}
