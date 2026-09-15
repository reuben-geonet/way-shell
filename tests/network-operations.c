/* Exercise the C Wi-Fi control flow before transferring these cases to Rust. */
#include <NetworkManager.h>
static GBytes *operation_ssid(NMAccessPoint *ap);
static void operation_commit(NMRemoteConnection *, gboolean, GCancellable *, GAsyncReadyCallback, gpointer);
static gboolean operation_commit_finish(NMRemoteConnection *, GAsyncResult *, GError **);
static void operation_activate(NMClient *, NMConnection *, NMDevice *, const char *, GCancellable *, GAsyncReadyCallback, gpointer);
static void operation_add(NMClient *, NMConnection *, NMDevice *, const char *, GCancellable *, GAsyncReadyCallback, gpointer);
static NMActiveConnection *operation_activate_finish(NMClient *, GAsyncResult *, GError **);
static NMActiveConnection *operation_add_finish(NMClient *, GAsyncResult *, GError **);
#define nm_access_point_get_ssid operation_ssid
#define nm_remote_connection_commit_changes_async operation_commit
#define nm_remote_connection_commit_changes_finish operation_commit_finish
#define nm_client_activate_connection_async operation_activate
#define nm_client_activate_connection_finish operation_activate_finish
#define nm_client_add_and_activate_connection_async operation_add
#define nm_client_add_and_activate_connection_finish operation_add_finish
#undef NM_CLIENT
#undef NM_DEVICE
#undef NM_REMOTE_CONNECTION
#define NM_CLIENT(value) ((NMClient *)(value))
#define NM_DEVICE(value) ((NMDevice *)(value))
#define NM_REMOTE_CONNECTION(value) ((NMRemoteConnection *)(value))
#define main inventory_fixture_main
#include "network.c"
#undef main

enum { ACTIVATE = 1, ADD, COMMIT };
typedef struct {
    GObject *source;
    GObject *connection;
    GObject *device;
    GCancellable *cancel;
    GAsyncReadyCallback callback;
    gpointer data;
    guint kind;
} PendingOperation;
static GPtrArray *pending_operations, *activated_devices;
static guint finished_connections;
static GBytes *operation_ssid(NMAccessPoint *ap) {
    return g_object_get_data(G_OBJECT(ap), "ssid");
}
static void queue_operation(guint kind, GObject *source, GObject *connection,
    GObject *device, GCancellable *cancel, GAsyncReadyCallback callback, gpointer data) {
    PendingOperation *operation = g_new0(PendingOperation, 1);
    operation->source = g_object_ref(source);
    if (connection) operation->connection = g_object_ref(connection);
    if (device) operation->device = g_object_ref(device);
    if (cancel) operation->cancel = g_object_ref(cancel);
    operation->callback = callback; operation->data = data; operation->kind = kind;
    g_ptr_array_add(pending_operations, operation);
}
static void operation_commit(NMRemoteConnection *connection, gboolean save,
    GCancellable *cancel, GAsyncReadyCallback callback, gpointer data) {
    g_assert_true(save);
    queue_operation(COMMIT, G_OBJECT(connection), NULL, NULL, cancel, callback, data);
}
static gboolean operation_commit_finish(NMRemoteConnection *connection,
    GAsyncResult *result, GError **error) {
    g_assert_true(g_task_get_source_tag(G_TASK(result)) == GUINT_TO_POINTER(COMMIT));
    return g_task_propagate_boolean(G_TASK(result), error);
}
static void operation_activate(NMClient *client, NMConnection *connection,
    NMDevice *device, const char *specific, GCancellable *cancel,
    GAsyncReadyCallback callback, gpointer data) {
    g_ptr_array_add(activated_devices, device);
    queue_operation(ACTIVATE, G_OBJECT(client), G_OBJECT(connection), G_OBJECT(device), cancel, callback, data);
}
static void operation_add(NMClient *client, NMConnection *connection,
    NMDevice *device, const char *specific, GCancellable *cancel,
    GAsyncReadyCallback callback, gpointer data) {
    queue_operation(ADD, G_OBJECT(client), G_OBJECT(connection), G_OBJECT(device), cancel, callback, data);
}
static NMActiveConnection *operation_activate_finish(NMClient *client, GAsyncResult *result, GError **error) {
    g_assert_true(g_task_get_source_tag(G_TASK(result)) == GUINT_TO_POINTER(ACTIVATE));
    return g_task_propagate_pointer(G_TASK(result), error);
}
static NMActiveConnection *operation_add_finish(NMClient *client, GAsyncResult *result, GError **error) {
    g_assert_true(g_task_get_source_tag(G_TASK(result)) == GUINT_TO_POINTER(ADD));
    return g_task_propagate_pointer(G_TASK(result), error);
}
static void connection_destroyed(gpointer data, GObject *object) { ++finished_connections; }
static void complete_operation(guint index, gboolean deny) {
    PendingOperation *operation = g_ptr_array_steal_index(pending_operations, index);
    GTask *result = g_task_new(operation->source, operation->cancel, NULL, NULL);
    g_task_set_source_tag(result, GUINT_TO_POINTER(operation->kind));
    if (deny) g_task_return_new_error(result, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED, "fixture denied");
    else if (operation->kind == COMMIT) g_task_return_boolean(result, TRUE);
    else {
        GObject *connection = g_object_new(G_TYPE_OBJECT, NULL);
        g_object_weak_ref(connection, connection_destroyed, NULL);
        g_task_return_pointer(result, connection, g_object_unref);
    }
    operation->callback(operation->source, G_ASYNC_RESULT(result), operation->data);
    g_object_unref(result);
    g_clear_object(&operation->source); g_clear_object(&operation->connection);
    g_clear_object(&operation->device); g_clear_object(&operation->cancel);
    g_free(operation);
    while (g_main_context_iteration(NULL, FALSE)) {}
}
static GObject *access_point(const char *ssid) {
    GObject *ap = g_object_new(G_TYPE_OBJECT, NULL);
    g_object_set_data_full(ap, "ssid", g_bytes_new(ssid, strlen(ssid)), (GDestroyNotify)g_bytes_unref);
    return ap;
}
static NMConnection *saved_connection(GObject *ap) {
    NMConnection *connection = nm_simple_connection_new();
    NMSetting *wireless = nm_setting_wireless_new();
    g_object_set(wireless, "ssid", operation_ssid((NMAccessPoint *)ap), NULL);
    nm_connection_add_setting(connection, wireless);
    return connection;
}
static NetworkManagerService *setup_operations(void) {
    setup(); online = TRUE;
    pending_operations = g_ptr_array_new(); activated_devices = g_ptr_array_new();
    finished_connections = 0;
    return g_object_new(NETWORK_MANAGER_SERVICE_TYPE, NULL);
}
static void teardown_operations(void) {
    g_assert_cmpuint(pending_operations->len, ==, 0);
    g_ptr_array_unref(pending_operations); g_ptr_array_unref(activated_devices);
    teardown();
}
static void saved_join_uses_matching_finish(void) {
    NetworkManagerService *service = setup_operations();
    GObject *ap = access_point("fixture-network");
    NMConnection *saved = saved_connection(ap);
    g_ptr_array_add(empty, saved);
    network_manager_service_ap_join(service, (NMDeviceWifi *)device_fixture, (NMAccessPoint *)ap, "fixture-password");
    complete_operation(0, FALSE); // Persist the changed password, then activate.
    g_assert_cmpuint(activated_devices->len, ==, 1);
    complete_operation(0, FALSE);
    g_assert_cmpuint(finished_connections, ==, 1);
    g_object_unref(saved); g_object_unref(ap); g_object_unref(service);
    teardown_operations();
}
static void concurrent_joins_keep_device_ownership(void) {
    NetworkManagerService *service = setup_operations();
    GObject *first_ap = access_point("fixture-one"), *second_ap = access_point("fixture-two");
    GObject *second_device = g_object_new(G_TYPE_OBJECT, NULL);
    NMConnection *first = saved_connection(first_ap), *second = saved_connection(second_ap);
    g_ptr_array_add(empty, first); g_ptr_array_add(empty, second);
    network_manager_service_ap_join(service, (NMDeviceWifi *)device_fixture, (NMAccessPoint *)first_ap, NULL);
    network_manager_service_ap_join(service, (NMDeviceWifi *)second_device, (NMAccessPoint *)second_ap, NULL);
    complete_operation(0, FALSE);
    g_assert_true(g_ptr_array_index(activated_devices, 0) == device_fixture);
    complete_operation(0, FALSE);
    g_assert_true(g_ptr_array_index(activated_devices, 1) == second_device);
    complete_operation(0, FALSE); complete_operation(0, FALSE);
    g_assert_cmpuint(finished_connections, ==, 2);
    g_object_unref(service); g_object_unref(first); g_object_unref(second);
    g_object_unref(first_ap); g_object_unref(second_ap); g_object_unref(second_device);
    teardown_operations();
}
static void destroy_cancels_pending_join(void) {
    NetworkManagerService *service = setup_operations();
    GObject *ap = access_point("fixture-network"); NMConnection *saved = saved_connection(ap);
    g_ptr_array_add(empty, saved);
    network_manager_service_ap_join(service, (NMDeviceWifi *)device_fixture, (NMAccessPoint *)ap, NULL);
    PendingOperation *operation = g_ptr_array_index(pending_operations, 0);
    gboolean finalized = FALSE; g_object_weak_ref(G_OBJECT(service), destroyed, &finalized);
    g_object_unref(service);
    g_assert_true(finalized);
    g_assert_true(g_cancellable_is_cancelled(operation->cancel));
    complete_operation(0, FALSE);
    g_assert_cmpuint(activated_devices->len, ==, 0);
    g_object_unref(saved); g_object_unref(ap);
    teardown_operations();
}
static void new_join_releases_results_and_credentials(void) {
    NetworkManagerService *service = setup_operations();
    GObject *ap = access_point("fixture-new-network");
    for (guint deny = 0; deny < 2; ++deny) {
        gboolean exposed = FALSE, connection_freed = FALSE, cancel_freed = FALSE;
        guint handler = g_log_set_handler(NULL, G_LOG_LEVEL_DEBUG, capture_credentials, &exposed);
        network_manager_service_ap_join(service, (NMDeviceWifi *)device_fixture,
            (NMAccessPoint *)ap, "fixture-secret-do-not-log");
        g_log_remove_handler(NULL, handler);
        g_assert_false(exposed);
        PendingOperation *operation = g_ptr_array_index(pending_operations, 0);
        g_assert_cmpuint(operation->kind, ==, ADD);
        NMSettingWirelessSecurity *security = nm_connection_get_setting_wireless_security(NM_CONNECTION(operation->connection));
        g_assert_cmpstr(nm_setting_wireless_security_get_psk(security), ==, "fixture-secret-do-not-log");
        g_object_weak_ref(operation->connection, destroyed, &connection_freed);
        g_object_weak_ref(G_OBJECT(operation->cancel), destroyed, &cancel_freed);
        complete_operation(0, deny);
        g_assert_true(connection_freed);
        g_assert_true(cancel_freed);
    }
    g_assert_cmpuint(finished_connections, ==, 1);
    g_object_unref(ap); g_object_unref(service);
    teardown_operations();
}
static void daemon_loss_cancels_pending_join(void) {
    NetworkManagerService *service = setup_operations();
    GObject *ap = access_point("fixture-saved-network"); NMConnection *saved = saved_connection(ap);
    g_ptr_array_add(empty, saved);
    network_manager_service_ap_join(service, (NMDeviceWifi *)device_fixture, (NMAccessPoint *)ap, NULL);
    online = FALSE;
    g_signal_emit_by_name(inventory_fixture, "changed");
    PendingOperation *operation = g_ptr_array_index(pending_operations, 0);
    g_assert_true(g_cancellable_is_cancelled(operation->cancel));
    complete_operation(0, FALSE);
    g_assert_cmpuint(activated_devices->len, ==, 0);
    g_object_unref(service); g_object_unref(saved); g_object_unref(ap);
    teardown_operations();
}
int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/network-operations/saved-finish", saved_join_uses_matching_finish);
    g_test_add_func("/network-operations/concurrent-devices", concurrent_joins_keep_device_ownership);
    g_test_add_func("/network-operations/destroy-cancels", destroy_cancels_pending_join);
    g_test_add_func("/network-operations/new-result-ownership", new_join_releases_results_and_credentials);
    g_test_add_func("/network-operations/daemon-loss-cancels", daemon_loss_cancels_pending_join);
    return g_test_run();
}
