#define G_SETTINGS_ENABLE_BACKEND
#include <gio/gio.h>
#include <gio/gsettingsbackend.h>

int main(int argc, char **argv) {
    GSettingsSchemaSource *source = g_settings_schema_source_get_default();
    g_assert_nonnull(source);
    g_assert_cmpstr(G_OBJECT_TYPE_NAME(g_settings_backend_get_default()), ==,
                   "GMemorySettingsBackend");
    g_assert_cmpint(argc, >, 1);
    for (int i = 1; i < argc; i++) {
        GSettingsSchema *schema = g_settings_schema_source_lookup(source, argv[i], TRUE);
        if (!schema) {
            g_printerr("Missing schema: %s\n", argv[i]);
            return 1;
        }
        GSettings *settings = g_settings_new_full(schema, NULL, NULL);
        gchar **keys = g_settings_schema_list_keys(schema);
        for (gchar **key = keys; *key; key++) {
            GVariant *value = g_settings_get_value(settings, *key);
            g_assert_nonnull(value);
            g_variant_unref(value);
        }
        g_print("Discovered and read %s\n", argv[i]);
        g_strfreev(keys);
        g_object_unref(settings);
        g_settings_schema_unref(schema);
    }
    return 0;
}
