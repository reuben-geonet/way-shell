#pragma once

#include <gio/gio.h>

G_BEGIN_DECLS

#define BLUETOOTH_SERVICE_TYPE (bluetooth_service_get_type())
G_DECLARE_FINAL_TYPE(BluetoothService, bluetooth_service, BLUETOOTH, SERVICE,
                     GObject)

typedef struct {
    char *path;
    char *alias;
    char *icon;
    gboolean connected;
    gboolean busy;
} BluetoothDevice;

/* Signals: changed(); operation-error(const char *message).
 * The constructor takes ownership of a nonblocking rfkill fd, or -1.
 * Accepting a connection and fd also permits isolated, hardware-free tests. */
BluetoothService *bluetooth_service_new(GDBusConnection *connection,
                                         int rfkill_fd);
void bluetooth_service_global_init(GDBusConnection *connection);
BluetoothService *bluetooth_service_get_global(void);
gboolean bluetooth_service_available(BluetoothService *self);
gboolean bluetooth_service_ready(BluetoothService *self);
gboolean bluetooth_service_powered(BluetoothService *self);
gboolean bluetooth_service_busy(BluetoothService *self);
gboolean bluetooth_service_hardware_blocked(BluetoothService *self);
/* Caller owns the array and its device snapshots. Sorted like GNOME's menu. */
GPtrArray *bluetooth_service_get_devices(BluetoothService *self);
void bluetooth_service_set_powered(BluetoothService *self, gboolean powered);
void bluetooth_service_toggle_device(BluetoothService *self, const char *path);
void bluetooth_service_set_airplane_mode(BluetoothService *self,
                                         gboolean enabled);

G_END_DECLS
