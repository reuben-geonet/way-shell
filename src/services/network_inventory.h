#pragma once
#include <NetworkManager.h>
#include <glib-object.h>
/* Temporary Rust inventory facade. Objects/arrays are borrowed until changed;
 * retain them with g_object_ref/g_ptr_array_ref when needed across updates. */
GObject *way_shell_network_inventory_new(void);
NMClient *way_shell_network_inventory_client(GObject *inventory);
const GPtrArray *way_shell_network_inventory_devices(GObject *inventory);
NMDevice *way_shell_network_inventory_primary(GObject *inventory);
NMState way_shell_network_inventory_state(GObject *inventory);
NMDeviceState way_shell_network_inventory_wifi_state(GObject *inventory);
gboolean way_shell_network_inventory_wifi_available(GObject *inventory);
gboolean way_shell_network_inventory_ethernet_available(GObject *inventory);
gboolean way_shell_network_inventory_networking_enabled(GObject *inventory);
gboolean way_shell_network_inventory_available(GObject *inventory);
void way_shell_network_inventory_set_wireless(GObject *inventory, gboolean enabled);
void way_shell_network_inventory_set_networking(GObject *inventory, gboolean enabled);
