#include <adwaita.h>
#include "../src/services/power_profiles_service/power_profiles_dbus.h"

static DbusPowerProfiles *fixture;

static DbusPowerProfiles *fixture_proxy(
    GDBusConnection *connection, GDBusProxyFlags flags, const gchar *name,
    const gchar *path, GCancellable *cancellable, GError **error) {
    return g_object_ref(fixture);
}

#define dbus_power_profiles_proxy_new_sync fixture_proxy
#include "../src/services/power_profiles_service/power_profiles_service.c"
#undef dbus_power_profiles_proxy_new_sync

DBUSService *dbus_service_get_global(void) { return NULL; }
GDBusConnection *dbus_service_get_system_bus(DBUSService *self) { return NULL; }

static GVariant *profile_list(const char *first, const char *second) {
    GVariantBuilder list;
    g_variant_builder_init(&list, G_VARIANT_TYPE("aa{sv}"));
    for (guint i = 0; i < (second ? 2 : 1); i++) {
        GVariantBuilder profile;
        g_variant_builder_init(&profile, G_VARIANT_TYPE("a{sv}"));
        g_variant_builder_add(&profile, "{sv}", "Profile",
                              g_variant_new_string(i ? second : first));
        g_variant_builder_add_value(&list, g_variant_builder_end(&profile));
    }
    return g_variant_ref_sink(g_variant_builder_end(&list));
}

static void count_profile_updates(PowerProfilesService *service, GArray *profiles,
                              guint *count) {
    (*count)++;
    g_assert_cmpuint(profiles->len, ==, 2);
    g_assert_cmpstr(g_array_index(profiles, char *, 1), ==, "performance");
}

static void profiles_property_updates_inventory(void) {
    fixture = dbus_power_profiles_skeleton_new();
    dbus_power_profiles_set_active_profile(fixture, "balanced");
    GVariant *initial = profile_list("balanced", NULL);
    dbus_power_profiles_set_profiles(fixture, initial);
    PowerProfilesService *service = g_object_new(POWER_PROFILES_SERVICE_TYPE, NULL);
    guint updates = 0;
    g_signal_connect(service, "profiles-changed", G_CALLBACK(count_profile_updates),
                     &updates);
    GVariant *replacement = profile_list("balanced", "performance");
    dbus_power_profiles_set_profiles(fixture, replacement);
    g_assert_cmpuint(updates, ==, 1);
    g_assert_cmpuint(power_profiles_service_get_profiles(service)->len, ==, 2);
    gpointer weak = service;
    g_object_add_weak_pointer(G_OBJECT(service), &weak);
    g_object_unref(service);
    g_assert_null(weak);
    // No callback may retain or access the released service.
    dbus_power_profiles_set_profiles(fixture, initial);
    g_assert_cmpuint(updates, ==, 1);
    g_variant_unref(initial);
    g_variant_unref(replacement);
    g_object_unref(fixture);
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/power-profiles/property-inventory", profiles_property_updates_inventory);
    return g_test_run();
}
