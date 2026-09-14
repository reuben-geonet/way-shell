#include <adwaita.h>
#include "../src/services/power_profiles_service/power_profiles_service.h"

typedef struct { GObject parent; } TestProvider;
typedef struct { GObjectClass parent; } TestProviderClass;
G_DEFINE_TYPE(TestProvider, test_provider, G_TYPE_OBJECT)
static void test_provider_class_init(TestProviderClass *klass) {
    g_signal_new("profiles-changed", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_FIRST,
                 0, NULL, NULL, NULL, G_TYPE_NONE, 1, G_TYPE_ARRAY);
}
static void test_provider_init(TestProvider *self) {}
static TestProvider *provider;
static GArray *inventory;
static char *requested;
PowerProfilesService *power_profiles_service_get_global(void) {
    return (PowerProfilesService *)provider;
}
GArray *power_profiles_service_get_profiles(PowerProfilesService *self) { return inventory; }
void power_profiles_service_set_profile(PowerProfilesService *self, gchar *profile) {
    g_free(requested);
    requested = g_strdup(profile);
}
const char *power_profiles_service_profile_to_icon(const char *profile) {
    return "power-profile-balanced-symbolic";
}
#include "../src/panel/quick_settings/quick_settings_grid/quick_settings_grid_power_profiles/quick_settings_grid_power_profiles_menu.c"

static guint child_count(GtkWidget *parent) {
    guint count = 0;
    for (GtkWidget *child = gtk_widget_get_first_child(parent); child;
         child = gtk_widget_get_next_sibling(child)) count++;
    return count;
}
static void inventory_updates_visible_menu(void) {
    provider = g_object_new(test_provider_get_type(), NULL);
    inventory = g_array_new(FALSE, FALSE, sizeof(char *));
    char *balanced = "balanced", *performance = "performance";
    g_array_append_val(inventory, balanced);
    QuickSettingsGridPowerProfilesMenu *menu =
        g_object_new(QUICK_SETTINGS_GRID_POWER_PROFILES_MENU_TYPE, NULL);
    GtkWidget *visible = quick_settings_grid_power_profiles_menu_get_widget(menu);
    GtkWidget *options = GTK_WIDGET(menu->menu.options);
    g_assert_cmpuint(child_count(options), ==, 1);
    g_array_append_val(inventory, performance);
    g_signal_emit_by_name(provider, "profiles-changed", inventory);
    g_assert_true(visible == quick_settings_grid_power_profiles_menu_get_widget(menu));
    g_assert_true(options == GTK_WIDGET(menu->menu.options));
    g_assert_cmpuint(child_count(options), ==, 2);
    GtkWidget *last = gtk_widget_get_last_child(options);
    g_signal_emit_by_name(gtk_widget_get_first_child(last), "clicked");
    g_assert_cmpstr(requested, ==, "performance");
    // Removal and restart reuse the container held by the revealer.
    g_array_set_size(inventory, 0);
    g_signal_emit_by_name(provider, "profiles-changed", inventory);
    g_assert_cmpuint(child_count(options), ==, 0);
    g_array_append_val(inventory, balanced);
    g_signal_emit_by_name(provider, "profiles-changed", inventory);
    g_assert_cmpuint(child_count(options), ==, 1);
    gpointer weak = menu;
    g_object_add_weak_pointer(G_OBJECT(menu), &weak);
    g_object_unref(menu);
    g_assert_null(weak);
    g_signal_emit_by_name(provider, "profiles-changed", inventory);
    g_array_unref(inventory);
    g_object_unref(provider);
    g_clear_pointer(&requested, g_free);
}
int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    gtk_init();
    g_test_add_func("/power-profiles/visible-menu-inventory", inventory_updates_visible_menu);
    return g_test_run();
}
