#include <NetworkManager.h>
#include <adwaita.h>

static NMClient *fixture_client_new(GCancellable *cancel, GError **error);
static const GPtrArray *fixture_devices(NMClient *client);
static const GPtrArray *fixture_connections(NMClient *client);
static NMState fixture_client_state(NMClient *client);
static NMActiveConnection *fixture_primary(NMClient *client);
static const GPtrArray *fixture_connection_devices(NMActiveConnection *connection);
static NMDeviceType fixture_device_type(NMDevice *device);
static NMDeviceState fixture_device_state(NMDevice *device);
static gboolean fixture_wireless_enabled(NMClient *client);
#define nm_client_new fixture_client_new
#define nm_client_get_devices fixture_devices
#define nm_client_get_connections fixture_connections
#define nm_client_get_active_connections fixture_connections
#define nm_client_get_state fixture_client_state
#define nm_client_get_primary_connection fixture_primary
#define nm_client_get_activating_connection fixture_primary
#define nm_active_connection_get_devices fixture_connection_devices
#define nm_device_get_device_type fixture_device_type
#define nm_device_get_state fixture_device_state
#define nm_client_wireless_get_enabled fixture_wireless_enabled
#include "../src/services/network_manager_service.c"

typedef struct { GObject parent_instance; } FixtureClient;
typedef struct { GObjectClass parent_class; } FixtureClientClass;
G_DEFINE_TYPE(FixtureClient, fixture_client, G_TYPE_OBJECT)
static void fixture_client_class_init(FixtureClientClass *klass) {
    for (unsigned i = 0; i < 4; ++i) {
        const char *names[] = {"connection-added", "connection-removed", "active-connection-added", "active-connection-removed"};
        g_signal_new(names[i], G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST,
                     0, NULL, NULL, NULL, G_TYPE_NONE, 1, G_TYPE_OBJECT);
    }
}
static void fixture_client_init(FixtureClient *self) {}
static GObject *client_fixture, *device_fixture;
static GPtrArray *device_list, *connection_device_list, *empty;
static NMState state;
static gboolean have_primary, wireless;
static NMDeviceState wifi_state;
static NMClient *fixture_client_new(GCancellable *cancel, GError **error) { return (NMClient *)g_object_ref(client_fixture); }
static const GPtrArray *fixture_devices(NMClient *client) { return device_list; }
static const GPtrArray *fixture_connections(NMClient *client) { return empty; }
static NMState fixture_client_state(NMClient *client) { return state; }
static NMActiveConnection *fixture_primary(NMClient *client) { return have_primary ? (NMActiveConnection *)client_fixture : NULL; }
static const GPtrArray *fixture_connection_devices(NMActiveConnection *connection) { return connection_device_list; }
static NMDeviceType fixture_device_type(NMDevice *device) { return NM_DEVICE_TYPE_WIFI; }
static NMDeviceState fixture_device_state(NMDevice *device) { return wifi_state; }
static gboolean fixture_wireless_enabled(NMClient *client) { return wireless; }
static void changed_count(NetworkManagerService *service, guint *count) { ++*count; }
static void state_removal_and_ownership(void) {
    client_fixture = g_object_new(fixture_client_get_type(), NULL);
    device_fixture = g_object_new(G_TYPE_OBJECT, NULL);
    device_list = g_ptr_array_new(); connection_device_list = g_ptr_array_new(); empty = g_ptr_array_new();
    g_ptr_array_add(device_list, device_fixture); g_ptr_array_add(connection_device_list, device_fixture);
    state = NM_STATE_CONNECTED_GLOBAL; have_primary = wireless = TRUE; wifi_state = NM_DEVICE_STATE_ACTIVATED;
    NetworkManagerService *service = g_object_new(NETWORK_MANAGER_SERVICE_TYPE, NULL);
    guint changes = 0;
    g_signal_connect(service, "changed", G_CALLBACK(changed_count), &changes);
    g_assert_true(network_manager_service_get_primary_device(service) == (NMDevice *)device_fixture);
    g_assert_cmpint(network_manager_service_wifi_state(service), ==, NM_DEVICE_STATE_ACTIVATED);
    have_primary = FALSE;
    on_changed((NMClient *)client_fixture, NULL, service);
    g_assert_null(network_manager_service_get_primary_device(service));
    g_assert_cmpuint(changes, ==, 1);
    have_primary = TRUE; g_ptr_array_set_size(connection_device_list, 0);
    on_changed((NMClient *)client_fixture, NULL, service);
    g_assert_null(network_manager_service_get_primary_device(service));
    g_assert_cmpuint(changes, ==, 2);
    wireless = FALSE;
    g_assert_cmpint(network_manager_service_wifi_state(service), ==, NM_DEVICE_STATE_DISCONNECTED);
    wireless = TRUE; wifi_state = NM_DEVICE_STATE_CONFIG;
    g_assert_cmpint(network_manager_service_wifi_state(service), ==, NM_DEVICE_STATE_PREPARE);
    g_ptr_array_set_size(device_list, 0);
    on_changed((NMClient *)client_fixture, NULL, service);
    g_assert_false(network_manager_service_wifi_available(service));
    g_assert_cmpint(network_manager_service_wifi_state(service), ==, NM_DEVICE_STATE_UNKNOWN);
    gpointer old_service = service;
    g_object_unref(service);
    g_assert_cmpuint(g_signal_handlers_block_matched(client_fixture, G_SIGNAL_MATCH_DATA,
        0, 0, NULL, NULL, old_service), ==, 0);
    g_assert_cmpuint(client_fixture->ref_count, ==, 1);
    g_assert_cmpuint(device_fixture->ref_count, ==, 1);
    g_ptr_array_unref(device_list); g_ptr_array_unref(connection_device_list); g_ptr_array_unref(empty);
    g_object_unref(client_fixture); g_object_unref(device_fixture);
}
static void destroyed(gpointer data, GObject *object) { *(gboolean *)data = TRUE; }
static void vpn_snapshot_owns_connections(void) {
    client_fixture = g_object_new(fixture_client_get_type(), NULL);
    device_list = g_ptr_array_new(); connection_device_list = g_ptr_array_new(); empty = g_ptr_array_new();
    state = NM_STATE_DISCONNECTED; have_primary = FALSE;
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
    g_ptr_array_unref(device_list); g_ptr_array_unref(connection_device_list); g_ptr_array_unref(empty);
    g_object_unref(client_fixture);
}
int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/network/state-removal-ownership", state_removal_and_ownership);
    g_test_add_func("/network/vpn-snapshot-ownership", vpn_snapshot_owns_connections);
    return g_test_run();
}
