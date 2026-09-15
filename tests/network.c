#include <NetworkManager.h>
#include <adwaita.h>
#include "../src/services/network_inventory.h"

static const GPtrArray *fixture_connections(NMClient *client);
#define nm_client_get_connections fixture_connections
#define nm_client_get_active_connections fixture_connections
#include "../src/services/network_manager_service.c"

typedef struct { GObject parent_instance; } FixtureObject;
typedef struct { GObjectClass parent_class; } FixtureObjectClass;
G_DEFINE_TYPE(FixtureObject, fixture_object, G_TYPE_OBJECT)
static void fixture_object_class_init(FixtureObjectClass *klass) {
    const char *names[] = {"connection-added", "connection-removed",
        "active-connection-added", "active-connection-removed"};
    for (unsigned i = 0; i < G_N_ELEMENTS(names); ++i)
        g_signal_new(names[i], G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST,
                     0, NULL, NULL, NULL, G_TYPE_NONE, 1, G_TYPE_OBJECT);
    g_signal_new("changed", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST,
                 0, NULL, NULL, NULL, G_TYPE_NONE, 0);
}
static void fixture_object_init(FixtureObject *self) {}
static GObject *client_fixture, *device_fixture, *inventory_fixture;
static GPtrArray *device_list, *empty;
static gboolean online, have_primary;
static const GPtrArray *fixture_connections(NMClient *client) { return empty; }
GObject *way_shell_network_inventory_new(void) { return g_object_ref(inventory_fixture); }
NMClient *way_shell_network_inventory_client(GObject *inventory) { return online ? (NMClient *)client_fixture : NULL; }
const GPtrArray *way_shell_network_inventory_devices(GObject *inventory) { return device_list; }
NMDevice *way_shell_network_inventory_primary(GObject *inventory) { return online && have_primary ? (NMDevice *)device_fixture : NULL; }
NMState way_shell_network_inventory_state(GObject *inventory) { return online ? NM_STATE_CONNECTED_GLOBAL : NM_STATE_UNKNOWN; }
NMDeviceState way_shell_network_inventory_wifi_state(GObject *inventory) { return online ? NM_DEVICE_STATE_ACTIVATED : NM_DEVICE_STATE_UNKNOWN; }
gboolean way_shell_network_inventory_wifi_available(GObject *inventory) { return online; }
gboolean way_shell_network_inventory_ethernet_available(GObject *inventory) { return FALSE; }
gboolean way_shell_network_inventory_networking_enabled(GObject *inventory) { return online; }
gboolean way_shell_network_inventory_available(GObject *inventory) { return online; }
void way_shell_network_inventory_set_wireless(GObject *inventory, gboolean enabled) {}
void way_shell_network_inventory_set_networking(GObject *inventory, gboolean enabled) {}
static void setup(void) {
    client_fixture = g_object_new(fixture_object_get_type(), NULL);
    inventory_fixture = g_object_new(fixture_object_get_type(), NULL);
    device_fixture = g_object_new(G_TYPE_OBJECT, NULL);
    device_list = g_ptr_array_new(); empty = g_ptr_array_new();
    online = have_primary = FALSE;
}
static void teardown(void) {
    g_assert_cmpuint(client_fixture->ref_count, ==, 1);
    g_assert_cmpuint(device_fixture->ref_count, ==, 1);
    g_assert_cmpuint(inventory_fixture->ref_count, ==, 1);
    g_ptr_array_unref(device_list); g_ptr_array_unref(empty);
    g_object_unref(client_fixture); g_object_unref(device_fixture);
    g_object_unref(inventory_fixture);
}
static void changed_count(NetworkManagerService *service, guint *count) { ++*count; }
static void enabled_count(NetworkManagerService *service, gboolean enabled, guint *count) { ++*count; }
static void inventory_recovery_and_ownership(void) {
    setup();
    NetworkManagerService *service = g_object_new(NETWORK_MANAGER_SERVICE_TYPE, NULL);
    guint changes = 0, enabled_changes = 0;
    g_signal_connect(service, "changed", G_CALLBACK(changed_count), &changes);
    g_signal_connect(service, "networking-enabled-changed", G_CALLBACK(enabled_count), &enabled_changes);
    g_assert_null(service->client);
    g_assert_cmpuint(network_manager_service_get_devices(service)->len, ==, 0);
    g_assert_false(network_manager_service_get_networking_enabled(service));
    online = have_primary = TRUE;
    g_ptr_array_add(device_list, device_fixture);
    g_signal_emit_by_name(inventory_fixture, "changed");
    g_assert_true(service->client == (NMClient *)client_fixture);
    g_assert_true(network_manager_service_get_primary_device(service) == (NMDevice *)device_fixture);
    g_assert_true(network_manager_service_wifi_available(service));
    g_assert_cmpint(network_manager_service_wifi_state(service), ==, NM_DEVICE_STATE_ACTIVATED);
    g_assert_cmpuint(enabled_changes, ==, 1);
    have_primary = FALSE;
    g_signal_emit_by_name(inventory_fixture, "changed");
    g_assert_null(network_manager_service_get_primary_device(service));
    g_assert_cmpuint(enabled_changes, ==, 1);
    online = FALSE;
    g_ptr_array_set_size(device_list, 0);
    g_signal_emit_by_name(inventory_fixture, "changed");
    g_assert_null(service->client);
    g_assert_cmpuint(changes, ==, 3);
    g_assert_cmpuint(enabled_changes, ==, 2);
    g_assert_cmpuint(g_signal_handlers_block_matched(client_fixture, G_SIGNAL_MATCH_DATA,
        0, 0, NULL, NULL, service), ==, 0);
    online = TRUE;
    g_signal_emit_by_name(inventory_fixture, "changed");
    g_assert_true(service->client == (NMClient *)client_fixture);
    gpointer old_service = service;
    g_object_unref(service);
    g_assert_cmpuint(g_signal_handlers_block_matched(client_fixture, G_SIGNAL_MATCH_DATA,
        0, 0, NULL, NULL, old_service), ==, 0);
    g_assert_cmpuint(g_signal_handlers_block_matched(inventory_fixture, G_SIGNAL_MATCH_DATA,
        0, 0, NULL, NULL, old_service), ==, 0);
    teardown();
}
static void destroyed(gpointer data, GObject *object) { *(gboolean *)data = TRUE; }
static void vpn_snapshot_owns_connections(void) {
    setup(); online = TRUE;
    NetworkManagerService *service = g_object_new(NETWORK_MANAGER_SERVICE_TYPE, NULL);
    NMConnection *connection = nm_simple_connection_new();
    NMSetting *setting = nm_setting_connection_new();
    g_object_set(setting, "id", "fixture", "type", "vpn", NULL);
    nm_connection_add_setting(connection, setting);
    gboolean finalized = FALSE;
    g_object_weak_ref(G_OBJECT(connection), destroyed, &finalized);
    on_vpn_connection_added((NMClient *)client_fixture, connection, service);
    on_vpn_connection_added((NMClient *)client_fixture, connection, service);
    GHashTable *snapshot = g_hash_table_ref(network_manager_get_vpn_connections(service));
    g_object_unref(connection);
    g_assert_false(finalized);
    g_object_unref(service);
    NMConnection *retained = g_hash_table_lookup(snapshot, "fixture");
    g_assert_cmpstr(nm_connection_get_id(retained), ==, "fixture");
    g_assert_false(finalized);
    g_hash_table_unref(snapshot);
    g_assert_true(finalized);
    teardown();
}
static void vpn_removed_has_live_connection(NetworkManagerService *service,
    NMConnection *connection, int count, guint *removed) {
    g_assert_cmpstr(nm_connection_get_id(connection), ==, "fixture");
    g_assert_cmpint(count, ==, 0);
    ++*removed;
}
static void daemon_loss_clears_vpn_snapshot(void) {
    setup(); online = TRUE;
    NetworkManagerService *service = g_object_new(NETWORK_MANAGER_SERVICE_TYPE, NULL);
    NMConnection *connection = nm_simple_connection_new();
    NMSetting *setting = nm_setting_connection_new();
    g_object_set(setting, "id", "fixture", "type", "wireguard", NULL);
    nm_connection_add_setting(connection, setting);
    on_vpn_connection_added((NMClient *)client_fixture, connection, service);
    g_object_unref(connection);
    guint removed = 0;
    g_signal_connect(service, "vpn-removed", G_CALLBACK(vpn_removed_has_live_connection), &removed);
    online = FALSE;
    g_signal_emit_by_name(inventory_fixture, "changed");
    g_assert_cmpuint(removed, ==, 1);
    g_assert_cmpuint(g_hash_table_size(network_manager_get_vpn_connections(service)), ==, 0);
    g_assert_false(network_manager_has_vpn(service));
    g_object_unref(service);
    teardown();
}
int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/network/inventory-recovery-ownership", inventory_recovery_and_ownership);
    g_test_add_func("/network/vpn-snapshot-ownership", vpn_snapshot_owns_connections);
    g_test_add_func("/network/daemon-loss-clears-vpn", daemon_loss_clears_vpn_snapshot);
    return g_test_run();
}
