#include <adwaita.h>
#include "../src/services/status_notifier_service/status_notifier_service.h"

/* Track the native objects retained by the real parser, including otherwise
 * unreachable leaves and invisible submenus. No display or live bus is needed. */
static guint live_objects, live_layouts;
static GVariant *reply_layout;
static GObject *bus_service;
static void object_gone(gpointer data, GObject *object) { live_objects--; }
static GMenu *tracked_menu_new(void) {
    GMenu *menu = g_menu_new();
    live_objects++;
    g_object_weak_ref(G_OBJECT(menu), object_gone, NULL);
    return menu;
}
static GMenuItem *tracked_menu_item_new(const gchar *label, const gchar *action) {
    GMenuItem *item = g_menu_item_new(label, action);
    live_objects++;
    g_object_weak_ref(G_OBJECT(item), object_gone, NULL);
    return item;
}
#define g_menu_new tracked_menu_new
#define g_menu_item_new tracked_menu_item_new
#include "../src/services/status_notifier_service/libdbusmenu.c"
#undef g_menu_new
#undef g_menu_item_new

static gboolean fixture_get_layout(DbusDbusmenu *proxy, gint parent, gint depth,
                                    const gchar *const *properties, guint *revision,
                                    GVariant **layout, GCancellable *cancellable,
                                    GError **error) {
    g_assert_cmpint(parent, ==, 0);
    g_assert_cmpint(depth, ==, -1);
    *revision = 1;
    *layout = g_variant_ref(reply_layout);
    return TRUE;
}
static guint fixture_own_name(GDBusConnection *connection, const gchar *name,
                              GBusNameOwnerFlags flags,
                              GBusNameAcquiredCallback acquired,
                              GBusNameLostCallback lost, gpointer data,
                              GDestroyNotify destroy) {
    return 1;
}
#define dbus_dbusmenu_call_get_layout_sync fixture_get_layout
#define g_bus_own_name_on_connection fixture_own_name
#include "../src/services/status_notifier_service/status_notifier_service.c"
#undef dbus_dbusmenu_call_get_layout_sync
#undef g_bus_own_name_on_connection

typedef struct { GObject parent_instance; } MenuFixtureBus;
typedef struct { GObjectClass parent_class; } MenuFixtureBusClass;
G_DEFINE_TYPE(MenuFixtureBus, menu_fixture_bus, G_TYPE_OBJECT)
static void menu_fixture_bus_class_init(MenuFixtureBusClass *klass) {
    g_signal_new("dbus-session-name-lost", G_TYPE_FROM_CLASS(klass),
                 G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 2,
                 G_TYPE_HASH_TABLE, G_TYPE_STRING);
}
static void menu_fixture_bus_init(MenuFixtureBus *self) {}
DBUSService *dbus_service_get_global(void) { return (DBUSService *)bus_service; }
GDBusConnection *dbus_service_get_session_bus(DBUSService *self) { return NULL; }

static GVariant *node(gint id, const gchar *label, gboolean visible,
                      gboolean separator, GVariant *children) {
    GVariantBuilder properties;
    g_variant_builder_init(&properties, G_VARIANT_TYPE("a{sv}"));
    if (label) g_variant_builder_add(&properties, "{sv}", "label", g_variant_new_string(label));
    if (!visible) g_variant_builder_add(&properties, "{sv}", "visible", g_variant_new_boolean(FALSE));
    if (separator) g_variant_builder_add(&properties, "{sv}", "type", g_variant_new_string("separator"));
    if (!children) children = g_variant_new_array(G_VARIANT_TYPE_VARIANT, NULL, 0);
    return g_variant_new("(i@a{sv}@av)", id,
                          g_variant_builder_end(&properties), children);
}
static void layout_gone(gpointer data) {
    live_layouts--;
    g_free(data);
}
static GVariant *tracked_layout(GVariant *layout) {
    layout = g_variant_ref_sink(layout);
    GBytes *serialized = g_variant_get_data_as_bytes(layout);
    gsize size;
    gpointer data = g_memdup2(g_bytes_get_data(serialized, &size), g_bytes_get_size(serialized));
    GBytes *bytes = g_bytes_new_with_free_func(data, size, layout_gone, data);
    live_layouts++;
    GVariant *result = g_variant_ref_sink(g_variant_new_from_bytes(
        G_VARIANT_TYPE("(ia{sv}av)"), bytes, TRUE));
    g_bytes_unref(bytes);
    g_bytes_unref(serialized);
    g_variant_unref(layout);
    return result;
}
static GVariant *leaf_layout(const gchar *label) {
    GVariantBuilder children;
    g_variant_builder_init(&children, G_VARIANT_TYPE("av"));
    g_variant_builder_add(&children, "v", node(1, label, TRUE, FALSE, NULL));
    return tracked_layout(node(0, NULL, TRUE, FALSE, g_variant_builder_end(&children)));
}
static void assert_action(GMenuModel *model, gint index, const gchar *label, gint id) {
    gchar *actual = NULL;
    g_assert_true(g_menu_model_get_item_attribute(model, index, G_MENU_ATTRIBUTE_LABEL, "s", &actual));
    g_assert_cmpstr(actual, ==, label);
    g_free(actual);
    g_assert_true(g_menu_model_get_item_attribute(model, index, G_MENU_ATTRIBUTE_ACTION, "s", &actual));
    g_assert_cmpstr(actual, ==, SNI_GRACTION_ITEM_CLICKED);
    g_free(actual);
    GVariant *target = g_menu_model_get_item_attribute_value(model, index, G_MENU_ATTRIBUTE_TARGET,
                                                            G_VARIANT_TYPE("(si)"));
    const gchar *key;
    gint actual_id;
    g_variant_get(target, "(&si)", &key, &actual_id);
    g_assert_cmpstr(key, ==, ":1.42");
    g_assert_cmpint(actual_id, ==, id);
    g_variant_unref(target);
}
static void menu_keeps_labels_sections_submenus_and_targets(void) {
    GVariantBuilder children, submenu;
    g_variant_builder_init(&children, G_VARIANT_TYPE("av"));
    g_variant_builder_add(&children, "v", node(-1, "Ignored", TRUE, FALSE, NULL));
    g_variant_builder_add(&children, "v", node(1, "_Open", TRUE, FALSE, NULL));
    g_variant_builder_add(&children, "v", node(2, "Hidden", FALSE, FALSE, NULL));
    g_variant_builder_add(&children, "v", node(3, NULL, TRUE, TRUE, NULL));
    g_variant_builder_init(&submenu, G_VARIANT_TYPE("av"));
    g_variant_builder_add(&submenu, "v", node(5, "Child", TRUE, FALSE, NULL));
    g_variant_builder_add(&children, "v", node(4, "Submenu", TRUE, FALSE, g_variant_builder_end(&submenu)));
    g_variant_builder_add(&children, "v", node(6, NULL, TRUE, TRUE, NULL));
    g_variant_builder_add(&children, "v", node(7, "Quit", TRUE, FALSE, NULL));
    GVariant *layout = tracked_layout(node(0, NULL, TRUE, FALSE, g_variant_builder_end(&children)));
    StatusNotifierItem item = {.bus_name = ":1.42"};
    libdbusmenu_parse_layout(layout, NULL, &item);
    g_variant_unref(layout);
    GMenuModel *root = G_MENU_MODEL(item.menu_model);
    g_assert_cmpint(g_menu_model_get_n_items(root), ==, 3);
    assert_action(root, 0, "_Open", 1);
    GMenuModel *section = g_menu_model_get_item_link(root, 1, G_MENU_LINK_SECTION);
    g_assert_cmpint(g_menu_model_get_n_items(section), ==, 1);
    assert_action(section, 0, "Submenu", 4);
    GMenuModel *child = g_menu_model_get_item_link(section, 0, G_MENU_LINK_SUBMENU);
    g_assert_cmpint(g_menu_model_get_n_items(child), ==, 1);
    assert_action(child, 0, "Child", 5);
    g_object_unref(child);
    g_object_unref(section);
    section = g_menu_model_get_item_link(root, 2, G_MENU_LINK_SECTION);
    g_assert_cmpint(g_menu_model_get_n_items(section), ==, 1);
    assert_action(section, 0, "Quit", 7);
    g_object_unref(section);
    g_object_unref(root);
}
static void menu_replacement_releases_all_native_owners(void) {
    live_objects = live_layouts = 0;
    StatusNotifierItem item = {.bus_name = ":1.42"};
    for (guint i = 0; i < 3; i++) {
        GVariant *layout = leaf_layout(i ? "Updated" : "Initial");
        libdbusmenu_parse_layout(layout, NULL, &item);
        g_variant_unref(layout);
        assert_action(G_MENU_MODEL(item.menu_model), 0, i ? "Updated" : "Initial", 1);
    }
    g_clear_object(&item.menu_model);
    g_assert_cmpuint(live_objects, ==, 0);
    g_assert_cmpuint(live_layouts, ==, 0);
}
static guint signal_order;
static void menu_will_update(StatusNotifierService *service, StatusNotifierItem *item, gpointer data) {
    g_assert_true(service == global);
    g_assert_true(item == data);
    g_assert_cmpuint(signal_order % 2, ==, 0);
    signal_order++;
}
static void menu_did_update(StatusNotifierService *service, StatusNotifierItem *item, gpointer data) {
    g_assert_true(service == global);
    g_assert_true(item == data);
    g_assert_cmpuint(signal_order % 2, ==, 1);
    assert_action(G_MENU_MODEL(item->menu_model), 0, "Updated", 1);
    signal_order++;
}
static void menu_updates_signal_the_service_and_release_replies(void) {
    live_objects = live_layouts = signal_order = 0;
    bus_service = g_object_new(menu_fixture_bus_get_type(), NULL);
    global = g_object_new(status_notifier_service_get_type(), NULL);
    StatusNotifierItem item = {.bus_name = ":1.42"};
    reply_layout = leaf_layout("Updated");
    g_signal_connect(global, "status-notifier-item-menu-will-update", G_CALLBACK(menu_will_update), &item);
    g_signal_connect(global, "status-notifier-item-menu-updated", G_CALLBACK(menu_did_update), &item);
    on_menu_layout_update(NULL, 1, 0, &item);
    on_property_update(NULL, NULL, NULL, &item);
    g_assert_cmpuint(signal_order, ==, 4);
    g_clear_object(&item.menu_model);
    g_clear_pointer(&reply_layout, g_variant_unref);
    g_assert_cmpuint(live_objects, ==, 0);
    g_assert_cmpuint(live_layouts, ==, 0);
    g_signal_handlers_disconnect_by_data(bus_service, global);
    g_clear_pointer(&global->items, g_hash_table_unref);
    g_clear_object(&global->host);
    g_clear_object(&global->watcher);
    g_clear_object(&global);
    g_clear_object(&bus_service);
}
int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/tray/menu/layout-contract", menu_keeps_labels_sections_submenus_and_targets);
    g_test_add_func("/tray/menu/ownership", menu_replacement_releases_all_native_owners);
    g_test_add_func("/tray/menu/update-signals", menu_updates_signal_the_service_and_release_replies);
    return g_test_run();
}
