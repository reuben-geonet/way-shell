#include "bluetooth_service.h"

#include <errno.h>
#include <fcntl.h>
#include <glib-unix.h>
#include <linux/rfkill.h>
#include <unistd.h>

#define ADAPTER "org.bluez.Adapter1"
#define DEVICE "org.bluez.Device1"
#define OPERATION_TIMEOUT_MS 30000
#define POWER_TIMEOUT_MS 5000

struct _BluetoothService {
    GObject parent_instance;
    GDBusConnection *connection;
    GDBusObjectManager *manager;
    GCancellable *initialization;
    GHashTable *pending; /* object path -> GCancellable */
    GHashTable *radios; /* rfkill index -> struct rfkill_event */
    GHashTable *restore_power; /* adapter path -> powered + 1 */
    GHashTable *restore_blocks; /* rfkill index -> soft + 1 */
    int rfkill_fd;
    guint rfkill_watch;
    guint changed_idle;
    guint power_timeout;
    gboolean requested_power;
    gboolean block_after_off;
    GHashTable *power_targets; /* optional per-adapter Airplane Mode restoration */
    gboolean airplane_mode;
    gboolean airplane_override;
};

G_DEFINE_TYPE(BluetoothService, bluetooth_service, G_TYPE_OBJECT)
enum { CHANGED, OPERATION_ERROR, OPERATION_SUCCEEDED, N_SIGNALS };
static guint signals[N_SIGNALS];
static BluetoothService *global;

static void reconcile_power(BluetoothService *self);
static gboolean adapter_target(BluetoothService *self, const char *path);
static gboolean software_blocked(BluetoothService *self);

static gboolean emit_changed(gpointer data) {
    BluetoothService *self = data;
    self->changed_idle = 0;
    reconcile_power(self);
    g_signal_emit(self, signals[CHANGED], 0);
    return G_SOURCE_REMOVE;
}

static void changed(BluetoothService *self) {
    if (!self->changed_idle)
        self->changed_idle = g_idle_add(emit_changed, self);
}

static void report_error(BluetoothService *self, const char *message) {
    g_message("Bluetooth: %s", message);
    g_signal_emit(self, signals[OPERATION_ERROR], 0, message);
}

static gboolean boolean_property(GDBusProxy *proxy, const char *name) {
    g_autoptr(GVariant) value = g_dbus_proxy_get_cached_property(proxy, name);
    return value && g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN) &&
           g_variant_get_boolean(value);
}

static char *string_property(GDBusProxy *proxy, const char *name,
                            const char *fallback) {
    g_autoptr(GVariant) value = g_dbus_proxy_get_cached_property(proxy, name);
    if (value && (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING) ||
                  g_variant_is_of_type(value, G_VARIANT_TYPE_OBJECT_PATH)))
        return g_variant_dup_string(value, NULL);
    return g_strdup(fallback);
}

static GDBusProxy *get_proxy(BluetoothService *self, const char *path,
                            const char *interface) {
    if (!self->manager) return NULL;
    return G_DBUS_PROXY(g_dbus_object_manager_get_interface(self->manager,
                                                           path, interface));
}

static GList *objects(BluetoothService *self) {
    return self->manager ? g_dbus_object_manager_get_objects(self->manager)
                         : NULL;
}

gboolean bluetooth_service_ready(BluetoothService *self) {
    if (!self->manager) return FALSE;
    g_autofree char *owner = g_dbus_object_manager_client_get_name_owner(
        G_DBUS_OBJECT_MANAGER_CLIENT(self->manager));
    return owner != NULL;
}

gboolean bluetooth_service_available(BluetoothService *self) {
    if (g_hash_table_size(self->radios)) return TRUE;
    GList *list = objects(self);
    gboolean found = FALSE;
    for (GList *l = list; l; l = l->next) {
        g_autoptr(GDBusInterface) adapter =
            g_dbus_object_get_interface(l->data, ADAPTER);
        if (adapter) found = TRUE;
    }
    g_list_free_full(list, g_object_unref);
    return found;
}

gboolean bluetooth_service_hardware_blocked(BluetoothService *self) {
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, self->radios);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        struct rfkill_event *event = value;
        if (event->hard) return TRUE;
    }
    return FALSE;
}

gboolean bluetooth_service_powered(BluetoothService *self) {
    if (software_blocked(self) || bluetooth_service_hardware_blocked(self)) return FALSE;
    GList *list = objects(self);
    gboolean powered = FALSE;
    for (GList *l = list; l; l = l->next) {
        g_autoptr(GDBusInterface) adapter =
            g_dbus_object_get_interface(l->data, ADAPTER);
        if (adapter && boolean_property(G_DBUS_PROXY(adapter), "Powered"))
            powered = TRUE;
    }
    g_list_free_full(list, g_object_unref);
    return powered;
}

gboolean bluetooth_service_busy(BluetoothService *self) {
    if (self->power_timeout) return TRUE;
    GHashTableIter iter;
    gpointer key;
    g_hash_table_iter_init(&iter, self->pending);
    while (g_hash_table_iter_next(&iter, &key, NULL)) {
        if (!strstr(key, "/dev_")) return TRUE;
    }
    return FALSE;
}

gboolean bluetooth_service_target_powered(BluetoothService *self) {
    return self->power_timeout ? self->requested_power
                               : bluetooth_service_powered(self);
}

static gboolean software_blocked(BluetoothService *self) {
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, self->radios);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        struct rfkill_event *event = value;
        if (event->soft) return TRUE;
    }
    return FALSE;
}

/* GNOME Bluetooth's connectable profile set: audio, HID (classic and LE),
 * and MIDI. UUIDs are BlueZ's canonical strings, not localized type names. */
static gboolean connectable(GDBusProxy *device) {
    static const char *uuids[] = {
        "00001108-0000-1000-8000-00805f9b34fb", /* Headset */
        "0000110a-0000-1000-8000-00805f9b34fb", /* Audio source */
        "0000110b-0000-1000-8000-00805f9b34fb", /* Audio sink */
        "0000110c-0000-1000-8000-00805f9b34fb", /* AVRCP target */
        "0000110e-0000-1000-8000-00805f9b34fb", /* AVRCP */
        "00001112-0000-1000-8000-00805f9b34fb", /* Headset gateway */
        "0000111e-0000-1000-8000-00805f9b34fb", /* Handsfree */
        "0000111f-0000-1000-8000-00805f9b34fb", /* Handsfree gateway */
        "00001124-0000-1000-8000-00805f9b34fb", /* HID */
        "00001812-0000-1000-8000-00805f9b34fb", /* LE HID */
        "03b80e5a-ede8-4b33-a751-6ce34ec4c700", /* MIDI */
        NULL,
    };
    g_autoptr(GVariant) value =
        g_dbus_proxy_get_cached_property(device, "UUIDs");
    if (!value || !g_variant_is_of_type(value, G_VARIANT_TYPE_STRING_ARRAY))
        return FALSE;
    g_auto(GStrv) profiles = g_variant_dup_strv(value, NULL);
    for (guint i = 0; profiles[i]; i++)
        for (guint j = 0; uuids[j]; j++)
            if (!g_ascii_strcasecmp(profiles[i], uuids[j])) return TRUE;
    return FALSE;
}

static void device_free(gpointer data) {
    BluetoothDevice *device = data;
    g_free(device->path);
    g_free(device->alias);
    g_free(device->icon);
    g_free(device);
}

static gint compare_devices(gconstpointer a, gconstpointer b) {
    const BluetoothDevice *left = *(BluetoothDevice *const *)a;
    const BluetoothDevice *right = *(BluetoothDevice *const *)b;
    if (left->connected != right->connected)
        return right->connected - left->connected;
    int order = g_utf8_collate(left->alias, right->alias);
    return order ? order : g_strcmp0(left->path, right->path);
}

GPtrArray *bluetooth_service_get_devices(BluetoothService *self) {
    GPtrArray *devices = g_ptr_array_new_with_free_func(device_free);
    if (!bluetooth_service_powered(self)) return devices;
    GList *list = objects(self);
    for (GList *l = list; l; l = l->next) {
        g_autoptr(GDBusInterface) interface =
            g_dbus_object_get_interface(l->data, DEVICE);
        if (!interface) continue;
        GDBusProxy *proxy = G_DBUS_PROXY(interface);
        if ((!boolean_property(proxy, "Paired") &&
             !boolean_property(proxy, "Trusted")) ||
            (!boolean_property(proxy, "Connected") && !connectable(proxy)))
            continue;
        g_autofree char *adapter_path = string_property(proxy, "Adapter", "");
        g_autoptr(GDBusProxy) adapter = get_proxy(self, adapter_path, ADAPTER);
        if (!adapter || !boolean_property(adapter, "Powered")) continue;
        BluetoothDevice *device = g_new0(BluetoothDevice, 1);
        device->path = g_strdup(g_dbus_proxy_get_object_path(proxy));
        device->alias = string_property(proxy, "Alias", "Bluetooth device");
        device->icon = string_property(proxy, "Icon", "bluetooth-symbolic");
        device->connected = boolean_property(proxy, "Connected");
        device->busy = g_hash_table_contains(self->pending, device->path);
        g_ptr_array_add(devices, device);
    }
    g_list_free_full(list, g_object_unref);
    g_ptr_array_sort(devices, compare_devices);
    return devices;
}

static gboolean write_radio(BluetoothService *self, guint8 op, guint32 idx,
                            gboolean blocked) {
    struct rfkill_event event = {.idx = idx, .type = RFKILL_TYPE_BLUETOOTH,
                                 .op = op, .soft = blocked};
    if (self->rfkill_fd < 0) return FALSE;
    ssize_t count;
    do {
        count = write(self->rfkill_fd, &event, sizeof(event));
    } while (count < 0 && errno == EINTR);
    if (count == sizeof(event)) return TRUE;
    report_error(self, "Cannot change the Bluetooth radio block. Check "
                       "your session's access to /dev/rfkill.");
    return FALSE;
}

static gboolean read_radios(gint fd, GIOCondition condition, gpointer data) {
    BluetoothService *self = data;
    struct rfkill_event event;
    while (read(fd, &event, sizeof(event)) == sizeof(event)) {
        if (event.type != RFKILL_TYPE_BLUETOOTH) continue;
        if (self->airplane_mode && !bluetooth_service_busy(self) &&
            event.op == RFKILL_OP_CHANGE && !event.soft)
            self->airplane_override = TRUE;
        gpointer key = GUINT_TO_POINTER(event.idx);
        if (event.op == RFKILL_OP_DEL)
            g_hash_table_remove(self->radios, key);
        else
            g_hash_table_replace(self->radios, key,
                                 g_memdup2(&event, sizeof(event)));
    }
    changed(self);
    if (condition & (G_IO_HUP | G_IO_ERR | G_IO_NVAL)) {
        self->rfkill_watch = 0;
        close(self->rfkill_fd);
        self->rfkill_fd = -1;
        g_hash_table_remove_all(self->radios);
        return G_SOURCE_REMOVE;
    }
    return G_SOURCE_CONTINUE;
}

static void finish_power(BluetoothService *self, gboolean success) {
    g_clear_handle_id(&self->power_timeout, g_source_remove);
    g_clear_pointer(&self->power_targets, g_hash_table_unref);
    GHashTableIter iter;
    gpointer key, value;
    g_hash_table_iter_init(&iter, self->pending);
    while (g_hash_table_iter_next(&iter, &key, &value)) {
        if (strstr(key, "/dev_")) continue;
        g_cancellable_cancel(value);
        g_hash_table_iter_remove(&iter);
    }
    /* Power down gracefully before a platform rfkill switch can remove
     * the controller. Normal Bluetooth toggles leave the USB device up. */
    if (success && self->block_after_off && self->rfkill_fd >= 0)
        success = write_radio(self, RFKILL_OP_CHANGE_ALL, 0, TRUE);
    self->block_after_off = FALSE;
    if (success) g_signal_emit(self, signals[OPERATION_SUCCEEDED], 0);
    changed(self);
}

static gboolean power_expired(gpointer data) {
    BluetoothService *self = data;
    self->power_timeout = 0;
    finish_power(self, FALSE);
    report_error(self, !bluetooth_service_ready(self)
        ? "The Bluetooth service is unavailable"
        : "Bluetooth did not respond. Check that the adapter is available.");
    return G_SOURCE_REMOVE;
}

typedef struct {
    BluetoothService *service;
    GDBusProxy *proxy;
    GCancellable *cancel;
    char *path;
    char *description;
    char *method;
    GVariant *parameters;
    gboolean enabling;
    guint retries;
    gint64 deadline;
} Operation;

static gboolean perform_call(gpointer data);

static void operation_finished(GObject *source, GAsyncResult *result,
                               gpointer data) {
    Operation *op = data;
    BluetoothService *self = op->service;
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply =
        g_dbus_proxy_call_finish(G_DBUS_PROXY(source), result, &error);
    gboolean power = !g_strcmp0(g_dbus_proxy_get_interface_name(op->proxy), ADAPTER);
    g_autoptr(GDBusProxy) current = get_proxy(self, op->path, power ? ADAPTER : DEVICE);
    gboolean relevant = current == op->proxy && bluetooth_service_ready(self) &&
        g_hash_table_lookup(self->pending, op->path) == op->cancel &&
        !g_cancellable_is_cancelled(op->cancel);
    gboolean superseded = power && self->power_timeout &&
        adapter_target(self, op->path) != op->enabling;
    gboolean target = power ? op->enabling : !g_strcmp0(op->method, "Connect");
    gboolean reached = current &&
        boolean_property(current, power ? "Powered" : "Connected") == target;
    /* BlueZ can still be processing an rfkill unblock when Powered is set.
     * Retry only that transient error; the UI stays busy throughout. */
    g_autofree char *remote = error ? g_dbus_error_get_remote_error(error) : NULL;
    if (power && relevant && !superseded && !reached && op->retries++ < 40 &&
        g_get_monotonic_time() < op->deadline &&
        !g_cancellable_is_cancelled(op->cancel) &&
        (!g_strcmp0(remote, "org.bluez.Error.InProgress") ||
         !g_strcmp0(remote, "org.bluez.Error.NotReady") ||
         (!g_strcmp0(remote, "org.bluez.Error.Failed") &&
          (strstr(error->message, "Blocked") || strstr(error->message, "blocked"))))) {
        g_timeout_add(100, perform_call, op);
        return;
    }
    if (g_hash_table_lookup(self->pending, op->path) == op->cancel)
        g_hash_table_remove(self->pending, op->path);
    /* A vanished/replaced object cannot report an error for the current
     * device. PropertiesChanged is also authoritative when a late method
     * reply claims failure after the requested state was reached. */
    if (error && relevant && !superseded && !reached) {
        g_dbus_error_strip_remote_error(error);
        g_autofree char *message =
            g_strdup_printf("%s: %s", op->description, error->message);
        report_error(self, message);
        if (power) finish_power(self, FALSE);
    } else if (relevant && !superseded && (!error || reached)) {
        g_signal_emit(self, signals[OPERATION_SUCCEEDED], 0);
    }
    changed(self);
    g_object_unref(op->proxy);
    g_object_unref(op->cancel);
    g_object_unref(op->service);
    g_free(op->path);
    g_free(op->description);
    g_free(op->method);
    g_clear_pointer(&op->parameters, g_variant_unref);
    g_free(op);
}

static gboolean perform_call(gpointer data) {
    Operation *op = data;
    g_dbus_proxy_call(op->proxy, op->method, op->parameters,
        G_DBUS_CALL_FLAGS_NONE,
        CLAMP((op->deadline - g_get_monotonic_time()) / 1000, 1, OPERATION_TIMEOUT_MS),
        op->cancel,
        operation_finished, op);
    return G_SOURCE_REMOVE;
}

static void call(BluetoothService *self, GDBusProxy *proxy, const char *method,
                 GVariant *parameters, const char *description,
                 gboolean enabling) {
    const char *path = g_dbus_proxy_get_object_path(proxy);
    GCancellable *previous = g_hash_table_lookup(self->pending, path);
    if (previous) g_cancellable_cancel(previous);
    Operation *op = g_new0(Operation, 1);
    op->service = g_object_ref(self);
    op->proxy = g_object_ref(proxy);
    op->cancel = g_cancellable_new();
    op->path = g_strdup(path);
    op->description = g_strdup(description);
    op->method = g_strdup(method);
    op->parameters = parameters ? g_variant_ref_sink(parameters) : NULL;
    op->enabling = enabling;
    op->deadline = g_get_monotonic_time() +
        (strstr(path, "/dev_") ? OPERATION_TIMEOUT_MS : POWER_TIMEOUT_MS) * 1000LL;
    g_hash_table_replace(self->pending, g_strdup(path),
                         g_object_ref(op->cancel));
    perform_call(op);
    changed(self);
}

static void adapter_power(BluetoothService *self, GDBusProxy *adapter,
                          gboolean powered) {
    call(self, adapter, "org.freedesktop.DBus.Properties.Set",
         g_variant_new("(ssv)", ADAPTER, "Powered",
                       g_variant_new_boolean(powered)),
         powered ? "Could not turn Bluetooth on" : "Could not turn Bluetooth off",
         powered);
}

static gboolean adapter_target(BluetoothService *self, const char *path) {
    return self->power_targets
        ? GPOINTER_TO_INT(g_hash_table_lookup(self->power_targets, path)) == 2
        : self->requested_power;
}

static void reconcile_power(BluetoothService *self) {
    if (!self->power_timeout || !bluetooth_service_ready(self)) return;
    GList *list = objects(self);
    gboolean complete = TRUE;
    guint adapters = 0;
    for (GList *l = list; l; l = l->next) {
        const char *path = g_dbus_object_get_object_path(l->data);
        g_autoptr(GDBusInterface) adapter =
            g_dbus_object_get_interface(l->data, ADAPTER);
        if (!adapter || (self->power_targets &&
            !g_hash_table_contains(self->power_targets, path))) continue;
        adapters++;
        gboolean target = adapter_target(self, path);
        if (boolean_property(G_DBUS_PROXY(adapter), "Powered") != target) {
            complete = FALSE;
            if (!g_hash_table_contains(self->pending, path))
                adapter_power(self, G_DBUS_PROXY(adapter), target);
        }
    }
    g_list_free_full(list, g_object_unref);
    /* Unblocking can take seconds to bring an adapter back. Keep the
     * user's intent until ObjectManager announces it, with a finite timeout. */
    if (self->requested_power && (!adapters ||
        (!self->power_targets && software_blocked(self)))) complete = FALSE;
    /* A previous request may still be executing on BlueZ. Serialize a
     * reversal behind its reply rather than cancelling an in-flight Set. */
    GHashTableIter iter;
    gpointer key;
    g_hash_table_iter_init(&iter, self->pending);
    while (g_hash_table_iter_next(&iter, &key, NULL))
        if (!strstr(key, "/dev_")) complete = FALSE;
    if (complete) finish_power(self, TRUE);
}

static void request_power(BluetoothService *self, gboolean powered,
                           GHashTable *targets) {
    if (powered && bluetooth_service_hardware_blocked(self)) {
        report_error(self, "Bluetooth is disabled by a hardware switch");
        return;
    }
    if (!bluetooth_service_ready(self)) {
        report_error(self, "The Bluetooth service is unavailable");
        return;
    }
    g_clear_handle_id(&self->power_timeout, g_source_remove);
    g_clear_pointer(&self->power_targets, g_hash_table_unref);
    self->power_targets = targets ? g_hash_table_ref(targets) : NULL;
    self->requested_power = powered;
    self->block_after_off = !powered && self->airplane_mode && !self->airplane_override;
    self->power_timeout = g_timeout_add(POWER_TIMEOUT_MS, power_expired, self);
    /* Only enable through rfkill when blocked. Blocking for an ordinary
     * power-off can unplug the adapter and race BlueZ's Properties.Set. */
    if (powered && !targets && software_blocked(self) &&
        !write_radio(self, RFKILL_OP_CHANGE_ALL, 0, FALSE)) {
        finish_power(self, FALSE);
        return;
    }
    changed(self);
}

static void set_powered(BluetoothService *self, gboolean powered) {
    request_power(self, powered, NULL);
}

void bluetooth_service_set_powered(BluetoothService *self, gboolean powered) {
    if (self->airplane_mode) self->airplane_override = TRUE;
    set_powered(self, powered);
}

void bluetooth_service_toggle_device(BluetoothService *self, const char *path) {
    if (g_hash_table_contains(self->pending, path)) return;
    g_autoptr(GDBusProxy) device = get_proxy(self, path, DEVICE);
    if (!device) return;
    gboolean connected = boolean_property(device, "Connected");
    g_autofree char *alias = string_property(device, "Alias", "device");
    g_autofree char *description = g_strdup_printf("Could not %s %s",
        connected ? "disconnect" : "connect to", alias);
    call(self, device, connected ? "Disconnect" : "Connect", NULL, description, FALSE);
}

void bluetooth_service_set_airplane_mode(BluetoothService *self,
                                         gboolean enabled) {
    if (self->airplane_mode == enabled) return;
    self->airplane_mode = enabled;
    if (enabled) {
        self->airplane_override = FALSE;
        GList *list = objects(self);
        for (GList *l = list; l; l = l->next) {
            g_autoptr(GDBusInterface) adapter =
                g_dbus_object_get_interface(l->data, ADAPTER);
            if (adapter)
                g_hash_table_insert(self->restore_power,
                    g_strdup(g_dbus_object_get_object_path(l->data)),
                    GINT_TO_POINTER(1 + boolean_property(G_DBUS_PROXY(adapter),
                                                         "Powered")));
        }
        g_list_free_full(list, g_object_unref);
        GHashTableIter iter;
        gpointer key, value;
        g_hash_table_iter_init(&iter, self->radios);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            struct rfkill_event *event = value;
            g_hash_table_insert(self->restore_blocks, key,
                                 GINT_TO_POINTER(1 + event->soft));
        }
        if (bluetooth_service_available(self)) set_powered(self, FALSE);
    } else {
        if (!self->airplane_override) {
            self->block_after_off = FALSE;
            GHashTableIter iter;
            gpointer key, value;
            /* CHANGE_ALL also sets the default for subsequently plugged-in
             * radios. Restore that default before the per-radio exceptions. */
            gboolean all_blocked = TRUE;
            g_hash_table_iter_init(&iter, self->restore_blocks);
            while (g_hash_table_iter_next(&iter, NULL, &value))
                if (GPOINTER_TO_INT(value) == 1) all_blocked = FALSE;
            if (g_hash_table_size(self->restore_blocks))
                write_radio(self, RFKILL_OP_CHANGE_ALL, 0, all_blocked);
            g_hash_table_iter_init(&iter, self->restore_blocks);
            while (g_hash_table_iter_next(&iter, &key, &value))
                if (g_hash_table_contains(self->radios, key))
                    write_radio(self, RFKILL_OP_CHANGE, GPOINTER_TO_UINT(key),
                                 GPOINTER_TO_INT(value) - 1);
            GHashTable *targets = g_hash_table_new_full(g_str_hash, g_str_equal,
                                                        g_free, NULL);
            gboolean any_powered = FALSE;
            g_hash_table_iter_init(&iter, self->restore_power);
            while (g_hash_table_iter_next(&iter, &key, &value)) {
                g_hash_table_insert(targets, g_strdup(key), value);
                if (GPOINTER_TO_INT(value) == 2) any_powered = TRUE;
            }
            if (g_hash_table_size(targets)) request_power(self, any_powered, targets);
            g_hash_table_unref(targets);
        }
        g_hash_table_remove_all(self->restore_power);
        g_hash_table_remove_all(self->restore_blocks);
    }
}

static void prune_operations(BluetoothService *self) {
    GHashTableIter iter;
    gpointer key, value;
    g_hash_table_iter_init(&iter, self->pending);
    while (g_hash_table_iter_next(&iter, &key, &value)) {
        g_autoptr(GDBusProxy) proxy = get_proxy(self, key,
            strstr(key, "/dev_") ? DEVICE : ADAPTER);
        if (proxy) continue;
        g_cancellable_cancel(value);
        g_hash_table_iter_remove(&iter);
    }
}

static void object_changed(GDBusObjectManager *manager, GDBusObject *object,
                           BluetoothService *self) {
    prune_operations(self);
    changed(self);
}

static void interface_changed(GDBusObjectManager *manager, GDBusObject *object,
                              GDBusInterface *interface,
                              BluetoothService *self) {
    prune_operations(self);
    changed(self);
}

static void properties_changed(GDBusObjectManagerClient *manager,
                               GDBusObjectProxy *object, GDBusProxy *proxy,
                               GVariant *properties, const char *const *invalid,
                               BluetoothService *self) {
    gboolean powered;
    if (self->airplane_mode && !bluetooth_service_busy(self) &&
        !g_strcmp0(g_dbus_proxy_get_interface_name(proxy), ADAPTER) &&
        g_variant_lookup(properties, "Powered", "b", &powered) && powered)
        self->airplane_override = TRUE;
    changed(self);
}

static void owner_changed(GObject *manager, GParamSpec *pspec,
                          BluetoothService *self) {
    if (!bluetooth_service_ready(self)) {
        GHashTableIter iter;
        gpointer value;
        g_hash_table_iter_init(&iter, self->pending);
        while (g_hash_table_iter_next(&iter, NULL, &value))
            g_cancellable_cancel(value);
        g_hash_table_remove_all(self->pending);
    }
    changed(self);
}

static void manager_ready(GObject *source, GAsyncResult *result, gpointer data) {
    BluetoothService *self = data;
    g_autoptr(GError) error = NULL;
    self->manager = g_dbus_object_manager_client_new_finish(result, &error);
    if (self->manager) {
        g_signal_connect(self->manager, "object-added",
                          G_CALLBACK(object_changed), self);
        g_signal_connect(self->manager, "object-removed",
                          G_CALLBACK(object_changed), self);
        g_signal_connect(self->manager, "interface-added",
                          G_CALLBACK(interface_changed), self);
        g_signal_connect(self->manager, "interface-removed",
                          G_CALLBACK(interface_changed), self);
        g_signal_connect(self->manager, "interface-proxy-properties-changed",
                          G_CALLBACK(properties_changed), self);
        g_signal_connect(self->manager, "notify::name-owner",
                          G_CALLBACK(owner_changed), self);
    } else if (!g_error_matches(error, G_IO_ERROR, G_IO_ERROR_CANCELLED)) {
        report_error(self, error->message);
    }
    changed(self);
    g_object_unref(self);
}

static void bluetooth_service_dispose(GObject *object) {
    BluetoothService *self = BLUETOOTH_SERVICE(object);
    g_cancellable_cancel(self->initialization);
    g_clear_handle_id(&self->power_timeout, g_source_remove);
    g_clear_pointer(&self->power_targets, g_hash_table_unref);
    if (self->changed_idle) g_source_remove(self->changed_idle);
    self->changed_idle = 0;
    if (self->rfkill_watch) g_source_remove(self->rfkill_watch);
    self->rfkill_watch = 0;
    if (self->rfkill_fd >= 0) close(self->rfkill_fd);
    self->rfkill_fd = -1;
    if (self->manager) g_signal_handlers_disconnect_by_data(self->manager, self);
    g_clear_object(&self->manager);
    g_clear_object(&self->connection);
    G_OBJECT_CLASS(bluetooth_service_parent_class)->dispose(object);
}

static void bluetooth_service_finalize(GObject *object) {
    BluetoothService *self = BLUETOOTH_SERVICE(object);
    g_clear_object(&self->initialization);
    g_hash_table_unref(self->pending);
    g_hash_table_unref(self->radios);
    g_hash_table_unref(self->restore_power);
    g_hash_table_unref(self->restore_blocks);
    G_OBJECT_CLASS(bluetooth_service_parent_class)->finalize(object);
}

static void bluetooth_service_class_init(BluetoothServiceClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->dispose = bluetooth_service_dispose;
    object_class->finalize = bluetooth_service_finalize;
    signals[CHANGED] = g_signal_new("changed", G_TYPE_FROM_CLASS(klass),
        G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 0);
    signals[OPERATION_SUCCEEDED] = g_signal_new("operation-succeeded", G_TYPE_FROM_CLASS(klass),
        G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL, G_TYPE_NONE, 0);
    signals[OPERATION_ERROR] = g_signal_new("operation-error",
        G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL,
        G_TYPE_NONE, 1, G_TYPE_STRING);
}

static void bluetooth_service_init(BluetoothService *self) {
    self->rfkill_fd = -1;
    self->initialization = g_cancellable_new();
    self->pending = g_hash_table_new_full(g_str_hash, g_str_equal, g_free,
                                         g_object_unref);
    self->radios = g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL,
                                        g_free);
    self->restore_power = g_hash_table_new_full(g_str_hash, g_str_equal,
                                               g_free, NULL);
    self->restore_blocks = g_hash_table_new(g_direct_hash, g_direct_equal);
}

BluetoothService *bluetooth_service_new(GDBusConnection *connection,
                                         int rfkill_fd) {
    BluetoothService *self = g_object_new(BLUETOOTH_SERVICE_TYPE, NULL);
    self->connection = g_object_ref(connection);
    self->rfkill_fd = rfkill_fd;
    if (rfkill_fd >= 0) {
        read_radios(rfkill_fd, G_IO_IN, self);
        self->rfkill_watch = g_unix_fd_add(rfkill_fd,
            G_IO_IN | G_IO_HUP | G_IO_ERR, read_radios, self);
    }
    g_dbus_object_manager_client_new(connection,
        G_DBUS_OBJECT_MANAGER_CLIENT_FLAGS_DO_NOT_AUTO_START, "org.bluez", "/",
        NULL, NULL, NULL, self->initialization, manager_ready,
        g_object_ref(self));
    return self;
}

void bluetooth_service_global_init(GDBusConnection *connection) {
    int fd = open("/dev/rfkill", O_RDWR | O_NONBLOCK | O_CLOEXEC);
    if (fd < 0) fd = open("/dev/rfkill", O_RDONLY | O_NONBLOCK | O_CLOEXEC);
    global = bluetooth_service_new(connection, fd);
}

BluetoothService *bluetooth_service_get_global(void) {
    return global;
}
