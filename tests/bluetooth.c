#include <gio/gio.h>
#include <linux/rfkill.h>
#include <sys/socket.h>
#include <unistd.h>

#include "bluetooth_service.h"
#include "bluetooth_settings.h"

#define ADAPTER "org.bluez.Adapter1"
#define DEVICE "org.bluez.Device1"
#define MOUSE "/org/bluez/hci0/dev_01"

typedef struct {
    const char *path;
    const char *alias;
    const char *uuid;
    gboolean paired, trusted, connected;
    guint registration;
} MockDevice;

typedef struct {
    GDBusConnection *server, *client;
    BluetoothService *service;
    GDBusNodeInfo *info;
    gboolean powered;
    guint power_calls;
    gboolean fail_connect;
    guint calls;
    guint errors;
    guint root_registration, adapter_registration;
    MockDevice devices[4];
    GDBusMethodInvocation *delayed;
    gboolean delay_connect;
    int radio_peer;
} Fixture;

static GTestDBus *bus;
static const char xml[] =
    "<node>"
    "<interface name='org.freedesktop.DBus.ObjectManager'>"
    "<method name='GetManagedObjects'><arg type='a{oa{sa{sv}}}' direction='out'/></method>"
    "<signal name='InterfacesAdded'><arg type='o'/><arg type='a{sa{sv}}'/></signal>"
    "<signal name='InterfacesRemoved'><arg type='o'/><arg type='as'/></signal>"
    "</interface>"
    "<interface name='org.bluez.Adapter1'>"
    "<property name='Powered' type='b' access='readwrite'/>"
    "</interface>"
    "<interface name='org.bluez.Device1'>"
    "<method name='Connect'/><method name='Disconnect'/>"
    "<property name='Alias' type='s' access='read'/>"
    "<property name='Icon' type='s' access='read'/>"
    "<property name='Adapter' type='o' access='read'/>"
    "<property name='UUIDs' type='as' access='read'/>"
    "<property name='Paired' type='b' access='read'/>"
    "<property name='Trusted' type='b' access='read'/>"
    "<property name='Connected' type='b' access='read'/>"
    "</interface></node>";

static void pump(void) {
    gint64 end = g_get_monotonic_time() + 100 * G_TIME_SPAN_MILLISECOND;
    do {
        while (g_main_context_iteration(NULL, FALSE));
        g_usleep(1000);
    } while (g_get_monotonic_time() < end);
}

static void wait_ready(Fixture *f) {
    gint64 end = g_get_monotonic_time() + 3 * G_TIME_SPAN_SECOND;
    while (!bluetooth_service_ready(f->service) && g_get_monotonic_time() < end)
        pump();
    g_assert_true(bluetooth_service_ready(f->service));
}

static MockDevice *lookup(Fixture *f, const char *path) {
    for (guint i = 0; i < G_N_ELEMENTS(f->devices); i++)
        if (!g_strcmp0(f->devices[i].path, path)) return &f->devices[i];
    return NULL;
}

static GVariant *properties(Fixture *f, const char *path) {
    GVariantBuilder b;
    g_variant_builder_init(&b, G_VARIANT_TYPE_VARDICT);
    MockDevice *d = lookup(f, path);
    if (!d) {
        g_variant_builder_add(&b, "{sv}", "Powered", g_variant_new_boolean(f->powered));
    } else {
        g_variant_builder_add(&b, "{sv}", "Alias", g_variant_new_string(d->alias));
        g_variant_builder_add(&b, "{sv}", "Icon", g_variant_new_string("input-mouse"));
        g_variant_builder_add(&b, "{sv}", "Adapter", g_variant_new_object_path("/org/bluez/hci0"));
        g_variant_builder_add(&b, "{sv}", "UUIDs", g_variant_new_strv(&d->uuid, 1));
        g_variant_builder_add(&b, "{sv}", "Paired", g_variant_new_boolean(d->paired));
        g_variant_builder_add(&b, "{sv}", "Trusted", g_variant_new_boolean(d->trusted));
        g_variant_builder_add(&b, "{sv}", "Connected", g_variant_new_boolean(d->connected));
    }
    return g_variant_builder_end(&b);
}

static GVariant *interfaces(Fixture *f, const char *path) {
    GVariantBuilder b;
    g_variant_builder_init(&b, G_VARIANT_TYPE("a{sa{sv}}"));
    g_variant_builder_add(&b, "{s@a{sv}}", lookup(f, path) ? DEVICE : ADAPTER,
                          properties(f, path));
    return g_variant_builder_end(&b);
}

static void notify(Fixture *f, const char *path) {
    g_dbus_connection_emit_signal(f->server, NULL, path,
        "org.freedesktop.DBus.Properties", "PropertiesChanged",
        g_variant_new("(s@a{sv}@as)", lookup(f, path) ? DEVICE : ADAPTER,
                       properties(f, path), g_variant_new_strv(NULL, 0)), NULL);
}

static void method_call(GDBusConnection *connection, const char *sender,
                        const char *path, const char *interface,
                        const char *method, GVariant *params,
                        GDBusMethodInvocation *invocation, gpointer data) {
    Fixture *f = data;
    if (!g_strcmp0(method, "GetManagedObjects")) {
        GVariantBuilder b;
        g_variant_builder_init(&b, G_VARIANT_TYPE("a{oa{sa{sv}}}"));
        if (f->adapter_registration)
            g_variant_builder_add(&b, "{o@a{sa{sv}}}", "/org/bluez/hci0",
                                   interfaces(f, "/org/bluez/hci0"));
        for (guint i = 0; i < G_N_ELEMENTS(f->devices); i++)
            if (f->devices[i].registration)
                g_variant_builder_add(&b, "{o@a{sa{sv}}}", f->devices[i].path,
                                       interfaces(f, f->devices[i].path));
        g_dbus_method_invocation_return_value(invocation,
            g_variant_new("(@a{oa{sa{sv}}})", g_variant_builder_end(&b)));
        return;
    }
    f->calls++;
    if (f->delay_connect) {
        f->delayed = g_object_ref(invocation);
        return;
    }
    if (f->fail_connect) {
        g_dbus_method_invocation_return_dbus_error(invocation,
            "org.bluez.Error.Failed", "Device is out of range");
        return;
    }
    MockDevice *d = lookup(f, path);
    d->connected = !g_strcmp0(method, "Connect");
    notify(f, path);
    g_dbus_method_invocation_return_value(invocation, NULL);
}

static GVariant *get_property(GDBusConnection *connection, const char *sender,
                              const char *path, const char *interface,
                              const char *name, GError **error, gpointer data) {
    g_autoptr(GVariant) props = g_variant_ref_sink(properties(data, path));
    return g_variant_lookup_value(props, name, NULL);
}

static gboolean set_property(GDBusConnection *connection, const char *sender,
                              const char *path, const char *interface,
                              const char *name, GVariant *value, GError **error,
                              gpointer data) {
    Fixture *f = data;
    f->power_calls++;
    f->powered = g_variant_get_boolean(value);
    notify(f, path);
    return TRUE;
}

static const GDBusInterfaceVTable vtable = {method_call, get_property, set_property};

static void error_seen(BluetoothService *service, const char *message, Fixture *f) {
    f->errors++;
}

static void bus_name(Fixture *f, gboolean own) {
    g_autoptr(GVariant) reply = g_dbus_connection_call_sync(f->server,
        "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
        own ? "RequestName" : "ReleaseName",
        own ? g_variant_new("(su)", "org.bluez", 0) : g_variant_new("(s)", "org.bluez"),
        NULL, G_DBUS_CALL_FLAGS_NONE, 3000, NULL, NULL);
    g_assert_nonnull(reply);
}

static void setup(Fixture *f, gconstpointer data) {
    f->powered = TRUE;
    f->info = g_dbus_node_info_new_for_xml(xml, NULL);
    GDBusConnectionFlags flags = G_DBUS_CONNECTION_FLAGS_AUTHENTICATION_CLIENT |
                                G_DBUS_CONNECTION_FLAGS_MESSAGE_BUS_CONNECTION;
    f->server = g_dbus_connection_new_for_address_sync(g_test_dbus_get_bus_address(bus),
                                                       flags, NULL, NULL, NULL);
    f->client = g_dbus_connection_new_for_address_sync(g_test_dbus_get_bus_address(bus),
                                                       flags, NULL, NULL, NULL);
    f->root_registration = g_dbus_connection_register_object(f->server, "/",
        f->info->interfaces[0], &vtable, f, NULL, NULL);
    f->adapter_registration = g_dbus_connection_register_object(f->server,
        "/org/bluez/hci0", f->info->interfaces[1], &vtable, f, NULL, NULL);
    f->devices[0] = (MockDevice){MOUSE, "Zebra mouse",
        "00001812-0000-1000-8000-00805f9b34fb", TRUE, FALSE, TRUE};
    f->devices[1] = (MockDevice){"/org/bluez/hci0/dev_02", "Alpha headphones",
        "0000110b-0000-1000-8000-00805f9b34fb", FALSE, TRUE, FALSE};
    f->devices[2] = (MockDevice){"/org/bluez/hci0/dev_03", "Unpaired mouse",
        "00001812-0000-1000-8000-00805f9b34fb", FALSE, FALSE, FALSE};
    f->devices[3] = (MockDevice){"/org/bluez/hci0/dev_04", "File transfer phone",
        "00001105-0000-1000-8000-00805f9b34fb", TRUE, TRUE, FALSE};
    for (guint i = 0; i < G_N_ELEMENTS(f->devices); i++)
        f->devices[i].registration = g_dbus_connection_register_object(f->server,
            f->devices[i].path, f->info->interfaces[2], &vtable, f, NULL, NULL);
    bus_name(f, TRUE);
    int sockets[2];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_NONBLOCK | SOCK_CLOEXEC,
                              0, sockets), ==, 0);
    f->radio_peer = sockets[1];
    struct rfkill_event radio = {.idx = 4, .type = RFKILL_TYPE_BLUETOOTH,
                                  .op = RFKILL_OP_ADD};
    g_assert_cmpint(write(f->radio_peer, &radio, sizeof(radio)), ==, sizeof(radio));
    f->service = bluetooth_service_new(f->client, sockets[0]);
    g_signal_connect(f->service, "operation-error", G_CALLBACK(error_seen), f);
    wait_ready(f);
}

static void teardown(Fixture *f, gconstpointer data) {
    if (f->delayed) {
        g_dbus_method_invocation_return_dbus_error(f->delayed,
            "org.bluez.Error.Failed", "Device removed");
        g_clear_object(&f->delayed);
    }
    pump();
    g_signal_handlers_disconnect_by_data(f->service, f);
    g_clear_object(&f->service);
    close(f->radio_peer);
    g_dbus_connection_unregister_object(f->server, f->root_registration);
    if (f->adapter_registration)
        g_dbus_connection_unregister_object(f->server, f->adapter_registration);
    for (guint i = 0; i < G_N_ELEMENTS(f->devices); i++)
        if (f->devices[i].registration)
            g_dbus_connection_unregister_object(f->server, f->devices[i].registration);
    g_dbus_connection_close_sync(f->server, NULL, NULL);
    g_dbus_connection_close_sync(f->client, NULL, NULL);
    g_object_unref(f->server);
    g_object_unref(f->client);
    g_dbus_node_info_unref(f->info);
}

static void test_devices(Fixture *f, gconstpointer data) {
    g_autoptr(GPtrArray) devices = bluetooth_service_get_devices(f->service);
    g_assert_cmpuint(devices->len, ==, 2);
    BluetoothDevice *mouse = devices->pdata[0];
    g_assert_cmpstr(mouse->alias, ==, "Zebra mouse");
    g_assert_true(mouse->connected);
    f->devices[0].connected = FALSE;
    notify(f, MOUSE);
    pump();
    g_clear_pointer(&devices, g_ptr_array_unref);
    devices = bluetooth_service_get_devices(f->service);
    g_assert_cmpstr(((BluetoothDevice *)devices->pdata[0])->alias, ==, "Alpha headphones");
    f->devices[1].trusted = FALSE;
    notify(f, f->devices[1].path);
    pump();
    g_clear_pointer(&devices, g_ptr_array_unref);
    devices = bluetooth_service_get_devices(f->service);
    g_assert_cmpuint(devices->len, ==, 1);
}

static void test_connection(Fixture *f, gconstpointer data) {
    bluetooth_service_toggle_device(f->service, MOUSE);
    bluetooth_service_toggle_device(f->service, MOUSE);
    pump();
    g_assert_cmpuint(f->calls, ==, 1);
    g_assert_false(f->devices[0].connected);
    bluetooth_service_toggle_device(f->service, MOUSE);
    pump();
    g_assert_true(f->devices[0].connected);
    f->fail_connect = TRUE;
    bluetooth_service_toggle_device(f->service, MOUSE);
    pump();
    g_assert_cmpuint(f->errors, ==, 1);
    g_autoptr(GPtrArray) devices = bluetooth_service_get_devices(f->service);
    g_assert_false(((BluetoothDevice *)devices->pdata[0])->busy);
    g_assert_true(((BluetoothDevice *)devices->pdata[0])->connected);
}

static void test_airplane(Fixture *f, gconstpointer data) {
    bluetooth_service_set_airplane_mode(f->service, TRUE);
    pump();
    g_assert_false(f->powered);
    bluetooth_service_set_airplane_mode(f->service, TRUE); /* do not overwrite snapshot */
    bluetooth_service_set_airplane_mode(f->service, FALSE);
    pump();
    g_assert_true(f->powered);
    bluetooth_service_set_powered(f->service, FALSE);
    pump();
    bluetooth_service_set_airplane_mode(f->service, TRUE);
    pump();
    bluetooth_service_set_airplane_mode(f->service, FALSE);
    pump();
    g_assert_false(f->powered); /* originally off stays off */
    bluetooth_service_set_airplane_mode(f->service, TRUE);
    pump();
    bluetooth_service_set_powered(f->service, TRUE);
    pump();
    bluetooth_service_set_airplane_mode(f->service, FALSE);
    pump();
    g_assert_true(f->powered); /* explicit override wins */
}

static void test_external_override(Fixture *f, gconstpointer data) {
    bluetooth_service_set_airplane_mode(f->service, TRUE);
    pump();
    f->powered = TRUE;
    notify(f, "/org/bluez/hci0");
    pump();
    f->powered = FALSE;
    notify(f, "/org/bluez/hci0");
    pump();
    bluetooth_service_set_airplane_mode(f->service, FALSE);
    pump();
    g_assert_false(f->powered);
}

static void test_power_cycles(Fixture *f, gconstpointer data) {
    for (guint i = 0; i < 20; i++) {
        bluetooth_service_set_powered(f->service, FALSE);
        pump();
        g_assert_false(f->powered);
        bluetooth_service_set_powered(f->service, TRUE);
        pump();
        g_assert_true(f->powered);
        g_assert_false(bluetooth_service_busy(f->service));
    }
    /* Fast reversal must preserve the final intent even before a reply. */
    bluetooth_service_set_powered(f->service, FALSE);
    bluetooth_service_set_powered(f->service, TRUE);
    pump();
    g_assert_true(f->powered);
    g_assert_false(bluetooth_service_busy(f->service));
    g_assert_cmpuint(f->errors, ==, 0);
    guint calls = f->power_calls;
    bluetooth_service_set_powered(f->service, FALSE);
    gint64 end = g_get_monotonic_time() + G_TIME_SPAN_SECOND;
    while (f->power_calls == calls && g_get_monotonic_time() < end)
        g_main_context_iteration(NULL, FALSE);
    g_assert_cmpuint(f->power_calls, >, calls);
    g_assert_true(bluetooth_service_busy(f->service));
    bluetooth_service_set_powered(f->service, TRUE);
    pump();
    g_assert_true(f->powered);
    g_assert_false(bluetooth_service_busy(f->service));
    g_assert_cmpuint(f->errors, ==, 0);
    /* Ordinary off must not tear the USB controller down through rfkill. */
    struct rfkill_event radio;
    g_assert_cmpint(read(f->radio_peer, &radio, sizeof(radio)), ==, -1);
}

static void test_delayed_adapter(Fixture *f, gconstpointer data) {
    const char *names[] = {ADAPTER, NULL};
    g_dbus_connection_unregister_object(f->server, f->adapter_registration);
    f->adapter_registration = 0;
    g_dbus_connection_emit_signal(f->server, NULL, "/",
        "org.freedesktop.DBus.ObjectManager", "InterfacesRemoved",
        g_variant_new("(o^as)", "/org/bluez/hci0", names), NULL);
    struct rfkill_event radio = {.idx = 4, .type = RFKILL_TYPE_BLUETOOTH,
                                  .op = RFKILL_OP_CHANGE, .soft = 1};
    write(f->radio_peer, &radio, sizeof(radio));
    pump();
    bluetooth_service_set_powered(f->service, TRUE);
    g_assert_true(bluetooth_service_busy(f->service));
    pump();
    g_assert_cmpint(read(f->radio_peer, &radio, sizeof(radio)), ==, sizeof(radio));
    g_assert_cmpuint(radio.soft, ==, 0);
    /* The unblocked adapter arrives later, initially powered off. */
    radio.idx = 4;
    radio.op = RFKILL_OP_CHANGE;
    write(f->radio_peer, &radio, sizeof(radio));
    f->powered = FALSE;
    f->adapter_registration = g_dbus_connection_register_object(f->server,
        "/org/bluez/hci0", f->info->interfaces[1], &vtable, f, NULL, NULL);
    g_dbus_connection_emit_signal(f->server, NULL, "/",
        "org.freedesktop.DBus.ObjectManager", "InterfacesAdded",
        g_variant_new("(o@a{sa{sv}})", "/org/bluez/hci0",
                       interfaces(f, "/org/bluez/hci0")), NULL);
    pump();
    g_assert_true(f->powered);
    g_assert_false(bluetooth_service_busy(f->service));
    g_assert_cmpuint(f->errors, ==, 0);
}

static void test_radio(Fixture *f, gconstpointer data) {
    struct rfkill_event radio = {.idx = 4, .type = RFKILL_TYPE_BLUETOOTH,
                                  .op = RFKILL_OP_CHANGE, .hard = 1};
    g_assert_cmpint(write(f->radio_peer, &radio, sizeof(radio)), ==, sizeof(radio));
    pump();
    g_assert_true(bluetooth_service_hardware_blocked(f->service));
    bluetooth_service_set_powered(f->service, TRUE);
    pump();
    g_assert_cmpuint(f->errors, ==, 1);
    radio.hard = 0;
    write(f->radio_peer, &radio, sizeof(radio));
    pump();
    bluetooth_service_set_airplane_mode(f->service, TRUE);
    pump();
    g_assert_cmpint(read(f->radio_peer, &radio, sizeof(radio)), ==, sizeof(radio));
    g_assert_cmpuint(radio.type, ==, RFKILL_TYPE_BLUETOOTH);
    g_assert_cmpuint(radio.op, ==, RFKILL_OP_CHANGE_ALL);
    g_assert_cmpuint(radio.soft, ==, 1);
}

static void test_owner_and_removal(Fixture *f, gconstpointer data) {
    f->delay_connect = TRUE;
    bluetooth_service_toggle_device(f->service, MOUSE);
    pump();
    g_assert_nonnull(f->delayed);
    const char *names[] = {DEVICE, NULL};
    g_dbus_connection_emit_signal(f->server, NULL, "/",
        "org.freedesktop.DBus.ObjectManager", "InterfacesRemoved",
        g_variant_new("(o^as)", MOUSE, names), NULL);
    pump();
    g_autoptr(GPtrArray) devices = bluetooth_service_get_devices(f->service);
    g_assert_cmpuint(devices->len, ==, 1);
    bus_name(f, FALSE);
    pump();
    g_assert_false(bluetooth_service_ready(f->service));
    g_clear_pointer(&devices, g_ptr_array_unref);
    devices = bluetooth_service_get_devices(f->service);
    g_assert_cmpuint(devices->len, ==, 0);
    bus_name(f, TRUE);
    wait_ready(f);
    g_clear_pointer(&devices, g_ptr_array_unref);
    devices = bluetooth_service_get_devices(f->service);
    g_assert_cmpuint(devices->len, ==, 2);
}

static void test_connected_without_profiles(Fixture *f, gconstpointer data) {
    /* Reconnecting LE devices can report Connected before UUID discovery. */
    f->devices[0].uuid = "";
    notify(f, MOUSE);
    pump();
    g_autoptr(GPtrArray) devices = bluetooth_service_get_devices(f->service);
    g_assert_cmpuint(devices->len, ==, 2);
    BluetoothDevice *mouse = devices->pdata[0];
    g_assert_cmpstr(mouse->path, ==, MOUSE);
    g_assert_true(mouse->connected);
}

static void test_removed_operation(Fixture *f, gconstpointer data) {
    f->delay_connect = TRUE;
    bluetooth_service_toggle_device(f->service, MOUSE);
    pump();
    const char *names[] = {DEVICE, NULL};
    g_dbus_connection_emit_signal(f->server, NULL, "/",
        "org.freedesktop.DBus.ObjectManager", "InterfacesRemoved",
        g_variant_new("(o^as)", MOUSE, names), NULL);
    pump();
    g_dbus_method_invocation_return_dbus_error(f->delayed,
        "org.freedesktop.DBus.Error.UnknownMethod", "Device no longer exists");
    g_clear_object(&f->delayed);
    pump();
    g_assert_cmpuint(f->errors, ==, 0);
}

static void test_completed_operation_error(Fixture *f, gconstpointer data) {
    f->delay_connect = TRUE;
    bluetooth_service_toggle_device(f->service, MOUSE);
    pump();
    /* The requested disconnect succeeded, but the method reply failed. */
    f->devices[0].connected = FALSE;
    notify(f, MOUSE);
    pump();
    g_dbus_method_invocation_return_dbus_error(f->delayed,
        "org.freedesktop.DBus.Error.NoReply", "Timeout was reached");
    g_clear_object(&f->delayed);
    pump();
    g_assert_cmpuint(f->errors, ==, 0);
    g_autoptr(GPtrArray) devices = bluetooth_service_get_devices(f->service);
    for (guint i = 0; i < devices->len; i++)
        g_assert_false(((BluetoothDevice *)devices->pdata[i])->busy);
}

static void test_power_timeout(Fixture *f, gconstpointer data) {
    const char *names[] = {ADAPTER, NULL};
    g_dbus_connection_unregister_object(f->server, f->adapter_registration);
    f->adapter_registration = 0;
    g_dbus_connection_emit_signal(f->server, NULL, "/",
        "org.freedesktop.DBus.ObjectManager", "InterfacesRemoved",
        g_variant_new("(o^as)", "/org/bluez/hci0", names), NULL);
    pump();
    bluetooth_service_set_powered(f->service, TRUE);
    g_assert_true(bluetooth_service_busy(f->service));
    gint64 end = g_get_monotonic_time() + 6 * G_TIME_SPAN_SECOND;
    while (bluetooth_service_busy(f->service) && g_get_monotonic_time() < end)
        pump();
    g_assert_false(bluetooth_service_busy(f->service));
    g_assert_cmpuint(f->errors, ==, 1);
    /* An unsuccessful attempt must not leave the next click disabled. */
    bluetooth_service_set_powered(f->service, FALSE);
    pump();
    g_assert_false(bluetooth_service_busy(f->service));
    g_assert_cmpuint(f->errors, ==, 1);
}

static void test_airplane_reversal(Fixture *f, gconstpointer data) {
    bluetooth_service_set_airplane_mode(f->service, TRUE);
    gint64 end = g_get_monotonic_time() + G_TIME_SPAN_SECOND;
    while (!f->power_calls && g_get_monotonic_time() < end)
        g_main_context_iteration(NULL, FALSE);
    g_assert_cmpuint(f->power_calls, ==, 1);
    bluetooth_service_set_airplane_mode(f->service, FALSE);
    pump();
    g_assert_true(f->powered);
    g_assert_false(bluetooth_service_busy(f->service));
    g_assert_cmpuint(f->errors, ==, 0);
}

static void test_settings(void) {
    g_autoptr(GError) error = NULL;
    g_auto(GStrv) argv = bluetooth_settings_parse_command("true 'a b' '$(false)'", &error);
    g_assert_no_error(error);
    g_assert_cmpstr(argv[1], ==, "a b");
    g_assert_cmpstr(argv[2], ==, "$(false)");
    g_assert_null(bluetooth_settings_parse_command("no-such-bluetooth-manager-92841", &error));
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND);
    g_clear_error(&error);
    g_assert_null(bluetooth_settings_parse_command("'unterminated", &error));
    g_assert_error(error, G_SHELL_ERROR, G_SHELL_ERROR_BAD_QUOTING);
    g_clear_error(&error);
    g_assert_null(bluetooth_settings_parse_command("", &error));
    g_assert_error(error, G_SHELL_ERROR, G_SHELL_ERROR_EMPTY_STRING);
    g_clear_error(&error);
    g_assert_true(bluetooth_settings_launch("true", &error));
    g_assert_no_error(error);
    g_autoptr(GSettings) settings = g_settings_new("org.ldelossa.way-shell.system");
    g_autofree char *command = g_settings_get_string(settings, "bluetooth-settings-command");
    g_assert_cmpstr(command, ==, "blueman-manager");
}

static void test_adapter_hotplug(Fixture *f, gconstpointer data) {
    const char *names[] = {ADAPTER, NULL};
    g_dbus_connection_unregister_object(f->server, f->adapter_registration);
    f->adapter_registration = 0;
    g_dbus_connection_emit_signal(f->server, NULL, "/",
        "org.freedesktop.DBus.ObjectManager", "InterfacesRemoved",
        g_variant_new("(o^as)", "/org/bluez/hci0", names), NULL);
    struct rfkill_event radio = {.idx = 4, .type = RFKILL_TYPE_BLUETOOTH,
                                  .op = RFKILL_OP_DEL};
    write(f->radio_peer, &radio, sizeof(radio));
    pump();
    g_assert_false(bluetooth_service_available(f->service));
    g_autoptr(GPtrArray) devices = bluetooth_service_get_devices(f->service);
    g_assert_cmpuint(devices->len, ==, 0);
    f->adapter_registration = g_dbus_connection_register_object(f->server,
        "/org/bluez/hci0", f->info->interfaces[1], &vtable, f, NULL, NULL);
    g_dbus_connection_emit_signal(f->server, NULL, "/",
        "org.freedesktop.DBus.ObjectManager", "InterfacesAdded",
        g_variant_new("(o@a{sa{sv}})", "/org/bluez/hci0",
                       interfaces(f, "/org/bluez/hci0")), NULL);
    pump();
    g_assert_true(bluetooth_service_available(f->service));
    g_clear_pointer(&devices, g_ptr_array_unref);
    devices = bluetooth_service_get_devices(f->service);
    g_assert_cmpuint(devices->len, ==, 2);
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    bus = g_test_dbus_new(G_TEST_DBUS_NONE);
    g_test_dbus_up(bus);
    g_test_add("/bluetooth/devices", Fixture, NULL, setup, test_devices, teardown);
    g_test_add("/bluetooth/connection", Fixture, NULL, setup, test_connection, teardown);
    g_test_add("/bluetooth/airplane", Fixture, NULL, setup, test_airplane, teardown);
    g_test_add("/bluetooth/external-override", Fixture, NULL, setup, test_external_override, teardown);
    g_test_add("/bluetooth/power-cycles", Fixture, NULL, setup, test_power_cycles, teardown);
    g_test_add("/bluetooth/delayed-adapter", Fixture, NULL, setup, test_delayed_adapter, teardown);
    g_test_add("/bluetooth/radio", Fixture, NULL, setup, test_radio, teardown);
    g_test_add("/bluetooth/owner-and-removal", Fixture, NULL, setup, test_owner_and_removal, teardown);
    g_test_add("/bluetooth/removed-operation", Fixture, NULL, setup, test_removed_operation, teardown);
    g_test_add("/bluetooth/completed-operation-error", Fixture, NULL, setup, test_completed_operation_error, teardown);
    g_test_add("/bluetooth/power-timeout", Fixture, NULL, setup, test_power_timeout, teardown);
    g_test_add("/bluetooth/airplane-reversal", Fixture, NULL, setup, test_airplane_reversal, teardown);
    g_test_add_func("/bluetooth/settings", test_settings);
    g_test_add("/bluetooth/connected-without-profiles", Fixture, NULL, setup, test_connected_without_profiles, teardown);
    g_test_add("/bluetooth/adapter-hotplug", Fixture, NULL, setup, test_adapter_hotplug, teardown);
    int result = g_test_run();
    g_test_dbus_down(bus);
    g_object_unref(bus);
    return result;
}
