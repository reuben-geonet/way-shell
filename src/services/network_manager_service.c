#include "network_manager_service.h"
#include "network_inventory.h"

#include <NetworkManager.h>
#include <adwaita.h>

#include "glib-object.h"
#include "glib.h"
#include "nm-core-types.h"
#include "nm-dbus-interface.h"

static NetworkManagerService *global = NULL;

enum signals {
    changed,
    networking_enabled,
    vpn_added,
    vpn_removed,
    vpn_activated,
    vpn_deactivated,
    signals_n
};

struct _NetworkManagerService {
    GObject parent_instance;
    GObject *inventory;
    NMClient *client;
    NMDevice *primary_dev;
    gboolean networking_is_enabled;
    gboolean has_vpn;
    GHashTable *vpn_conns;
    GHashTable *active_vpn_conns;
    GPtrArray *wireless_operations;
};
static guint signals[signals_n] = {0};
G_DEFINE_TYPE(NetworkManagerService, network_manager_service, G_TYPE_OBJECT);

static void cancel_wireless_operations(NetworkManagerService *self) {
    if (!self->wireless_operations) return;
    for (guint i = 0; i < self->wireless_operations->len; ++i)
        g_cancellable_cancel(g_ptr_array_index(self->wireless_operations, i));
    g_ptr_array_set_size(self->wireless_operations, 0);
}

// Temporary C connection operations are cancelled before releasing their owner.
static void network_manager_service_dispose(GObject *gobject) {
    NetworkManagerService *self = NETWORK_MANAGER_SERVICE(gobject);

    cancel_wireless_operations(self);
    g_clear_pointer(&self->wireless_operations, g_ptr_array_unref);
    if (self->inventory)
        g_signal_handlers_disconnect_by_data(self->inventory, self);
    g_clear_object(&self->inventory);
    if (self->client)
        g_signal_handlers_disconnect_by_data(self->client, self);
    g_clear_object(&self->primary_dev);
    g_clear_object(&self->client);
    g_clear_pointer(&self->vpn_conns, g_hash_table_unref);
    g_clear_pointer(&self->active_vpn_conns, g_hash_table_unref);

    // Chain-up
    G_OBJECT_CLASS(network_manager_service_parent_class)->dispose(gobject);
};

static void network_manager_service_finalize(GObject *gobject) {
    // Chain-up
    G_OBJECT_CLASS(network_manager_service_parent_class)->finalize(gobject);
};

static void network_manager_service_class_init(
    NetworkManagerServiceClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->dispose = network_manager_service_dispose;
    object_class->finalize = network_manager_service_finalize;

    signals[changed] =
        g_signal_new("changed", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST, 0,
                     NULL, NULL, NULL, G_TYPE_NONE, 0);
    signals[networking_enabled] = g_signal_new(
        "networking-enabled-changed", G_TYPE_FROM_CLASS(klass),
        G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 1, G_TYPE_BOOLEAN);

    signals[vpn_added] = g_signal_new(
        "vpn-added", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST, 0, NULL, NULL,
        NULL, G_TYPE_NONE, 2, G_TYPE_POINTER, G_TYPE_INT);

    signals[vpn_removed] = g_signal_new(
        "vpn-removed", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST, 0, NULL,
        NULL, NULL, G_TYPE_NONE, 2, G_TYPE_POINTER, G_TYPE_INT);

    signals[vpn_activated] = g_signal_new(
        "vpn-activated", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST, 0, NULL,
        NULL, NULL, G_TYPE_NONE, 1, G_TYPE_POINTER);

    signals[vpn_deactivated] = g_signal_new(
        "vpn-deactivated", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST, 0, NULL,
        NULL, NULL, G_TYPE_NONE, 1, G_TYPE_POINTER);
};

gboolean connection_is_vpn(NMConnection *conn) {
    const gchar *type = nm_connection_get_connection_type(conn);
    // we consider wireguard a vpn, tho NetworkManager has its own built in
    // type for this.
    return (g_strcmp0(type, "vpn") == 0 || g_strcmp0(type, "wireguard") == 0);
}

gboolean active_connection_is_vpn(NMActiveConnection *conn) {
    const gchar *type = nm_active_connection_get_connection_type(conn);
    // we consider wireguard a vpn, tho NetworkManager has its own built in
    // type for this.
    return (g_strcmp0(type, "vpn") == 0 || g_strcmp0(type, "wireguard") == 0);
}

static void on_vpn_connection_added(NMClient *client, NMConnection *conn,
                                    NetworkManagerService *self) {
    const gchar *id = nm_connection_get_id(conn);

    // We only care about tracking VPN connections.
    if (!connection_is_vpn(conn)) return;

    g_hash_table_insert(self->vpn_conns, g_strdup(id), g_object_ref(conn));
    int len = g_hash_table_size(self->vpn_conns);

    self->has_vpn = true;

    g_signal_emit(self, signals[vpn_added], 0, conn, len);
}

static void on_vpn_connection_removed(NMClient *client, NMConnection *conn,
                                      NetworkManagerService *self) {
    // We only care about tracking VPN connections.
    if (!connection_is_vpn(conn)) return;

    const gchar *id = nm_connection_get_id(conn);

    g_hash_table_remove(self->vpn_conns, id);
    int len = g_hash_table_size(self->vpn_conns);

    if (len == 0) self->has_vpn = false;

    g_signal_emit(self, signals[vpn_removed], 0, conn, len);
}

static void on_active_vpn_connection_added(NMClient *client,
                                           NMActiveConnection *conn,
                                           NetworkManagerService *self) {
    // We only care about tracking VPN connections.
    if (!active_connection_is_vpn(conn)) return;

    const gchar *id = nm_active_connection_get_id(conn);
    g_hash_table_insert(self->active_vpn_conns, g_strdup(id), g_object_ref(conn));

    g_signal_emit(self, signals[vpn_activated], 0, conn);
}

static void on_active_vpn_connection_removed(NMClient *client,
                                             NMActiveConnection *conn,
                                             NetworkManagerService *self) {
    // We only care about tracking VPN connections.
    if (!active_connection_is_vpn(conn)) return;

    const gchar *id = nm_active_connection_get_id(conn);
    g_hash_table_remove(self->active_vpn_conns, id);

    g_signal_emit(self, signals[vpn_deactivated], 0, conn);
}

/* C connection operations remain temporarily attached to the Rust-owned client. */
static void on_inventory_changed(GObject *inventory, NetworkManagerService *self) {
    NMClient *client = way_shell_network_inventory_client(inventory);
    if (client != self->client) {
        cancel_wireless_operations(self);
        if (self->client)
            g_signal_handlers_disconnect_by_data(self->client, self);

        /* Retain each value while removal signals notify the existing widgets. */
        GList *connections = g_hash_table_get_values(self->active_vpn_conns);
        for (GList *item = connections; item; item = item->next)
            g_object_ref(item->data);
        for (GList *item = connections; item; item = item->next)
            on_active_vpn_connection_removed(self->client, item->data, self);
        g_list_free_full(connections, g_object_unref);
        connections = g_hash_table_get_values(self->vpn_conns);
        for (GList *item = connections; item; item = item->next)
            g_object_ref(item->data);
        for (GList *item = connections; item; item = item->next)
            on_vpn_connection_removed(self->client, item->data, self);
        g_list_free_full(connections, g_object_unref);

        g_set_object(&self->client, client);
        if (client) {
            const GPtrArray *saved = nm_client_get_connections(client);
            for (guint i = 0; i < saved->len; ++i)
                on_vpn_connection_added(client, saved->pdata[i], self);
            const GPtrArray *active = nm_client_get_active_connections(client);
            for (guint i = 0; i < active->len; ++i)
                on_active_vpn_connection_added(client, active->pdata[i], self);
            g_signal_connect(client, "connection-added", G_CALLBACK(on_vpn_connection_added), self);
            g_signal_connect(client, "connection-removed", G_CALLBACK(on_vpn_connection_removed), self);
            g_signal_connect(client, "active-connection-added", G_CALLBACK(on_active_vpn_connection_added), self);
            g_signal_connect(client, "active-connection-removed", G_CALLBACK(on_active_vpn_connection_removed), self);
        }
    }
    g_set_object(&self->primary_dev, way_shell_network_inventory_primary(inventory));
    gboolean enabled = way_shell_network_inventory_networking_enabled(inventory);
    if (enabled != self->networking_is_enabled) {
        self->networking_is_enabled = enabled;
        g_signal_emit(self, signals[networking_enabled], 0, enabled);
    }
    g_signal_emit(self, signals[changed], 0);
}

static void network_manager_service_init(NetworkManagerService *self) {
    self->vpn_conns = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_object_unref);
    self->active_vpn_conns = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_object_unref);
    self->wireless_operations = g_ptr_array_new_with_free_func(g_object_unref);
    self->inventory = way_shell_network_inventory_new();
    g_signal_connect(self->inventory, "changed", G_CALLBACK(on_inventory_changed), self);
    on_inventory_changed(self->inventory, self);
}

const GPtrArray *network_manager_service_get_devices(
    NetworkManagerService *self) {
    return way_shell_network_inventory_devices(self->inventory);
};

NMDevice *network_manager_service_get_primary_device(
    NetworkManagerService *self) {
    return self->primary_dev;
};

gboolean network_manager_service_wifi_available(NetworkManagerService *self) {
    return way_shell_network_inventory_wifi_available(self->inventory);
};

gboolean network_manager_service_ethernet_available(
    NetworkManagerService *self) {
    return way_shell_network_inventory_ethernet_available(self->inventory);
};

NMState network_manager_service_get_state(NetworkManagerService *self) {
    return way_shell_network_inventory_state(self->inventory);
};

// A simplification of Device state which returns
// NM_DEVICE_STATE_UNKNOWN if there are no wifi devices on the system
// NM_DEVICE_STATE_DISCONNECTED if all wifi devices are disconnected
// NM_DEVICE_STATE_PREPARE if at least one wifi device is in the process of
// connecting to a network.
// NM_DEVICE_STATE_ACTIVATED if at least one wifi device is activated
NMDeviceState network_manager_service_wifi_state(NetworkManagerService *self) {
    return way_shell_network_inventory_wifi_state(self->inventory);
}

int network_manager_service_global_init(void) {
    g_debug(
        "network_manager_service.c:network_manager_service_global_init() "
        "initializing global network manager service");
    global = g_object_new(NETWORK_MANAGER_SERVICE_TYPE, NULL);
    return 0;
};

// Get the global clock service
// Will return NULL if `network_manager_service_global_init` has not been
// called.
NetworkManagerService *network_manager_service_get_global() { return global; };

char *network_manager_service_ap_strength_to_icon_name(guchar strength) {
    if (strength < 25) {
        return "network-wireless-signal-weak-symbolic";
    } else if (strength < 50) {
        return "network-wireless-signal-ok-symbolic";
    } else if (strength < 75) {
        return "network-wireless-signal-good-symbolic";
    } else {
        return "network-wireless-signal-excellent-symbolic";
    }
};

char *network_manager_service_ap_to_name(NMAccessPoint *ap) {
    GBytes *bytes = nm_access_point_get_ssid(ap);

    if (!bytes) return "";

    if (nm_utils_is_empty_ssid(g_bytes_get_data(bytes, NULL),
                               g_bytes_get_size(bytes)))
        return "";

    char *ssid = nm_utils_ssid_to_utf8(g_bytes_get_data(bytes, NULL),
                                       g_bytes_get_size(bytes));
    return ssid;
}

typedef struct {
    GWeakRef owner;
    NMClient *client;
    NMDevice *device;
    GCancellable *cancel;
    gboolean create;
} WirelessOperation;

static void wireless_operation_free(WirelessOperation *operation) {
    NetworkManagerService *self = g_weak_ref_get(&operation->owner);
    if (self && self->wireless_operations)
        g_ptr_array_remove_fast(self->wireless_operations, operation->cancel);
    g_clear_object(&self);
    g_weak_ref_clear(&operation->owner);
    g_clear_object(&operation->client);
    g_clear_object(&operation->device);
    g_clear_object(&operation->cancel);
    g_free(operation);
}

static void on_ap_join(GObject *source_object, GAsyncResult *res, gpointer data) {
    WirelessOperation *operation = data;
    GError *error = NULL;
    NMActiveConnection *connection = operation->create
        ? nm_client_add_and_activate_connection_finish(NM_CLIENT(source_object), res, &error)
        : nm_client_activate_connection_finish(NM_CLIENT(source_object), res, &error);
    if (error && !g_error_matches(error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
        g_debug("Wi-Fi activation failed: %s", error->message);
    g_clear_error(&error);
    g_clear_object(&connection);
    wireless_operation_free(operation);
}

static void on_remote_conn_sync(GObject *source_object, GAsyncResult *res, gpointer data) {
    WirelessOperation *operation = data;
    GError *error = NULL;
    gboolean committed = nm_remote_connection_commit_changes_finish(
        NM_REMOTE_CONNECTION(source_object), res, &error);
    NetworkManagerService *self = g_weak_ref_get(&operation->owner);
    gboolean current = self && self->client == operation->client &&
        !g_cancellable_is_cancelled(operation->cancel);
    g_clear_object(&self);
    if (!committed || !current) {
        if (error && !g_error_matches(error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
            g_debug("Wi-Fi settings update failed: %s", error->message);
        g_clear_error(&error);
        wireless_operation_free(operation);
        return;
    }
    nm_client_activate_connection_async(operation->client, NM_CONNECTION(source_object),
        operation->device, NULL, operation->cancel, on_ap_join, operation);
}

void network_manager_service_ap_join(NetworkManagerService *self,
                                     NMDeviceWifi *dev, NMAccessPoint *ap,
                                     const char *password) {
    g_debug(
        "network_manager_service.c:network_manager_service_wifi_join() called");

    if (!self || !self->client || !dev || !ap) return;
    NMClient *client = self->client;
    GBytes *ap_ssid = nm_access_point_get_ssid(ap);
    NMConnection *found_conn = NULL;
    gboolean new = false;
    if (!ap_ssid || nm_utils_is_empty_ssid(g_bytes_get_data(ap_ssid, NULL),
                                          g_bytes_get_size(ap_ssid))) {
        g_debug(
            "network_manager_service.c:network_manager_service_wifi_join() "
            "missing required arguments");
        return;
    }

    const GPtrArray *connections = nm_client_get_connections(self->client);

    for (int i = 0; i < connections->len; i++) {
        NMConnection *conn = connections->pdata[i];
        NMSettingWireless *wireless = nm_connection_get_setting_wireless(conn);
        if (!wireless) continue;

        GBytes *conn_ssid = nm_setting_wireless_get_ssid(wireless);
        if (conn_ssid && g_bytes_equal(ap_ssid, conn_ssid)) {
            g_debug(
                "network_manager_service.c:network_manager_service_wifi_join() "
                "found matching connection");
            found_conn = conn;
            break;
        }
    }

    // didn't find one...
    if (!found_conn) {
        new = true;
        g_autofree char *ssid_name = network_manager_service_ap_to_name(ap);
        found_conn = nm_simple_connection_new();

        NMSettingConnection *conn_settings =
            NM_SETTING_CONNECTION(nm_setting_connection_new());
        g_object_set(conn_settings, NM_SETTING_CONNECTION_ID, ssid_name,
                     NM_SETTING_CONNECTION_AUTOCONNECT, true, NULL);
        nm_connection_add_setting(found_conn, NM_SETTING(conn_settings));

        NMSettingWireless *wireless_settings =
            NM_SETTING_WIRELESS(nm_setting_wireless_new());
        g_object_set(wireless_settings, NM_SETTING_WIRELESS_SSID, ap_ssid,
                     NULL);
        nm_connection_add_setting(found_conn, NM_SETTING(wireless_settings));

        g_debug(
            "network_manager_service.c:network_manager_service_wifi_join() "
            "created new connection [%s] and connecting...",
            ssid_name);
    }

    // update the password even if we found a conn, the UI does not currently
    // check for cached passwords and requires the user to enter a password.
    //
    // this is kinda useful because if the password has changed, they can just
    // re-enter it withou any special conditions in the UI code.
    // this may change tho if it becomes too inconenvient to put in a password
    // when switching between known networks...
    if (password) {
        NMSettingWirelessSecurity *sec_settings =
            NM_SETTING_WIRELESS_SECURITY(nm_setting_wireless_security_new());
        g_object_set(sec_settings, NM_SETTING_WIRELESS_SECURITY_PSK, password,
                     NM_SETTING_WIRELESS_SECURITY_KEY_MGMT, "wpa-psk", NULL);
        nm_connection_add_setting(found_conn, NM_SETTING(sec_settings));
    }

    WirelessOperation *operation = g_new0(WirelessOperation, 1);
    g_weak_ref_init(&operation->owner, self);
    operation->client = g_object_ref(client);
    operation->device = g_object_ref(NM_DEVICE(dev));
    operation->cancel = g_cancellable_new();
    operation->create = new;
    g_ptr_array_add(self->wireless_operations, g_object_ref(operation->cancel));
    if (new) {
        nm_client_add_and_activate_connection_async(client, found_conn, NM_DEVICE(dev),
            NULL, operation->cancel, on_ap_join, operation);
        g_object_unref(found_conn);
    } else {
        nm_remote_connection_commit_changes_async(NM_REMOTE_CONNECTION(found_conn),
            true, operation->cancel, on_remote_conn_sync, operation);
    }
}

static void on_wifi_disconnect(GObject *source_object, GAsyncResult *res,
                               gpointer data) {
    GError *error = NULL;
    NMClient *client = NM_CLIENT(source_object);
    nm_client_deactivate_connection_finish(client, res, &error);

    if (error) {
        g_debug(
            "network_manager_service.c:on_ap_join() failed to disconnect wifi: "
            "%s",
            error->message);
        g_error_free(error);
        return;
    }
}

void network_manager_service_ap_disconnect(NetworkManagerService *self,
                                           NMDeviceWifi *dev) {
    if (!self || !self->client || !dev) return;
    NMActiveConnection *active_con =
        nm_device_get_active_connection(NM_DEVICE(dev));

    if (!active_con) return;

    nm_client_deactivate_connection_async(self->client, active_con, NULL,
                                          on_wifi_disconnect, self);
}

void network_manager_service_wireless_enable(NetworkManagerService *self,
                                             gboolean enabled) {
    way_shell_network_inventory_set_wireless(self->inventory, enabled);
}

void network_manager_service_networking_enable(NetworkManagerService *self,
                                               gboolean enabled) {
    way_shell_network_inventory_set_networking(self->inventory, enabled);
}

gboolean network_manager_service_get_networking_enabled(
    NetworkManagerService *self) {
    return way_shell_network_inventory_networking_enabled(self->inventory);
};

gboolean network_manager_has_vpn(NetworkManagerService *self) {
    return self->has_vpn;
}

GHashTable *network_manager_get_vpn_connections(NetworkManagerService *self) {
    return self->vpn_conns;
}

static void on_vpn_activated(GObject *source_object, GAsyncResult *res,
                             gpointer data) {
    GError *error = NULL;
    NMClient *client = NM_CLIENT(source_object);
    NMActiveConnection *connection = nm_client_activate_connection_finish(client, res, &error);
    g_clear_object(&connection);

    if (error) {
        g_debug(
            "network_manager_service.c:on_vpn_activated() failed to activate "
            "vpn: %s",
            error->message);
        g_error_free(error);
        return;
    }
}

static void on_vpn_deactivated(GObject *source_object, GAsyncResult *res,
                               gpointer data) {
    GError *error = NULL;
    NMClient *client = NM_CLIENT(source_object);
    nm_client_deactivate_connection_finish(client, res, &error);

    if (error) {
        g_debug(
            "network_manager_service.c:on_vpn_activated() failed to deactivate "
            "vpn: %s",
            error->message);
        g_error_free(error);
        return;
    }
}

void network_manager_activate_vpn(NetworkManagerService *self, const gchar *id,
                                  gboolean activate) {
    if (!self || !self->client || !id) return;
    NMConnection *conn = g_hash_table_lookup(self->vpn_conns, id);
    if (!conn) return;

    if (activate) {
        NMDevice *dev = self->primary_dev;

        const char *type = nm_connection_get_connection_type(conn);

        if (g_strcmp0(type, "wireguard") == 0) {
            // wireguard connections setup their own interfaces on created,
            // so we don't need to provide a base device.
            dev = NULL;
        }

        nm_client_activate_connection_async(self->client, conn, dev, NULL, NULL,
                                            on_vpn_activated, self);
    } else {
        NMActiveConnection *active_conn = NULL;

        // determine if the desired conn is actually active...
        const GPtrArray *active_conns =
            nm_client_get_active_connections(self->client);

        for (int i = 0; i < active_conns->len; i++) {
            NMActiveConnection *ac = active_conns->pdata[i];
            if (g_strcmp0(nm_active_connection_get_id(ac), id) == 0) {
                active_conn = ac;
                break;
            }
        }

        if (!active_conn) return;

        nm_client_deactivate_connection_async(self->client, active_conn, NULL,
                                              on_vpn_deactivated, self);
    }
}

GHashTable *network_manager_service_get_active_vpn_connections(
    NetworkManagerService *self) {
    return self->active_vpn_conns;
}
