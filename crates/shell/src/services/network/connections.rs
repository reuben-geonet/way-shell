//! Owned access-point and connection snapshots over the retained libnm cache.
use super::{AccessPoint, ActiveConnection, SavedConnection, native};
use gio::prelude::*;
use glib::{gobject_ffi::GObject, translate::*};
use std::{
    collections::HashMap,
    ffi::{CStr, c_char},
};

unsafe extern "C" {
    fn nm_client_get_connections(client: *mut GObject) -> *const glib::ffi::GPtrArray;
    fn nm_client_get_active_connections(client: *mut GObject) -> *const glib::ffi::GPtrArray;
    fn nm_device_wifi_get_access_points(device: *mut GObject) -> *const glib::ffi::GPtrArray;
    fn nm_device_wifi_get_active_access_point(device: *mut GObject) -> *mut GObject;
    fn nm_device_get_active_connection(device: *mut GObject) -> *mut GObject;
    fn nm_access_point_get_ssid(ap: *mut GObject) -> *mut glib::ffi::GBytes;
    fn nm_access_point_get_strength(ap: *mut GObject) -> u8;
    fn nm_access_point_get_flags(ap: *mut GObject) -> u32;
    fn nm_access_point_get_wpa_flags(ap: *mut GObject) -> u32;
    fn nm_access_point_get_rsn_flags(ap: *mut GObject) -> u32;
    fn nm_utils_ssid_to_utf8(bytes: *const u8, len: usize) -> *mut c_char;
    fn nm_connection_get_path(connection: *mut GObject) -> *const c_char;
    fn nm_connection_get_id(connection: *mut GObject) -> *const c_char;
    fn nm_connection_get_uuid(connection: *mut GObject) -> *const c_char;
    fn nm_connection_get_connection_type(connection: *mut GObject) -> *const c_char;
    fn nm_connection_to_dbus(connection: *mut GObject, flags: u32) -> *mut glib::ffi::GVariant;
    fn nm_active_connection_get_connection(connection: *mut GObject) -> *mut GObject;
    fn nm_active_connection_get_id(connection: *mut GObject) -> *const c_char;
    fn nm_active_connection_get_connection_type(connection: *mut GObject) -> *const c_char;
    fn nm_active_connection_get_state(connection: *mut GObject) -> u32;
    fn nm_active_connection_get_devices(connection: *mut GObject) -> *const glib::ffi::GPtrArray;
}
// libnm strings are borrowed; snapshots copy them before yielding the main context.
unsafe fn string(pointer: *const c_char) -> String {
    if pointer.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
    }
}
pub(super) fn saved_path(connection: &glib::Object) -> String {
    unsafe { string(nm_connection_get_path(connection.as_ptr())) }
}
pub(super) type Settings = HashMap<String, HashMap<String, glib::Variant>>;
pub(super) fn settings(connection: &glib::Object) -> Option<Settings> {
    let pointer = unsafe { nm_connection_to_dbus(connection.as_ptr(), 0) };
    if pointer.is_null() {
        return None;
    }
    let value: glib::Variant = unsafe { from_glib_full(pointer) };
    value.get()
}
pub(super) fn device_active(device: &glib::Object) -> Option<String> {
    let pointer = unsafe { nm_device_get_active_connection(device.as_ptr()) };
    if pointer.is_null() {
        None
    } else {
        let object: glib::Object = unsafe { from_glib_none(pointer) };
        Some(native::path(&object))
    }
}
impl native::Client {
    pub fn connections(&self) -> Vec<glib::Object> {
        unsafe { native::objects(nm_client_get_connections(self.object().as_ptr())) }
    }
    pub fn all_active(&self) -> Vec<glib::Object> {
        unsafe { native::objects(nm_client_get_active_connections(self.object().as_ptr())) }
    }
    pub fn access_points(&self) -> Vec<glib::Object> {
        self.devices()
            .iter()
            .filter(|device| native::device_kind(device) == 2)
            .flat_map(|device| unsafe {
                native::objects(nm_device_wifi_get_access_points(device.as_ptr()))
            })
            .collect()
    }
    pub fn saved_snapshot(&self) -> Vec<SavedConnection> {
        self.connections()
            .iter()
            .map(|connection| {
                let settings = settings(connection);
                SavedConnection {
                    id: saved_path(connection),
                    name: unsafe { string(nm_connection_get_id(connection.as_ptr())) },
                    uuid: unsafe { string(nm_connection_get_uuid(connection.as_ptr())) },
                    kind: unsafe { string(nm_connection_get_connection_type(connection.as_ptr())) },
                    ssid: settings
                        .as_ref()
                        .and_then(|settings| settings.get("802-11-wireless")?.get("ssid")?.get()),
                    last_used: settings
                        .as_ref()
                        .and_then(|settings| settings.get("connection")?.get("timestamp")?.get())
                        .unwrap_or(0),
                }
            })
            .collect()
    }
    pub fn active_snapshot(&self) -> Vec<ActiveConnection> {
        self.all_active()
            .iter()
            .map(|active| {
                let connection = unsafe { nm_active_connection_get_connection(active.as_ptr()) };
                ActiveConnection {
                    id: native::path(active),
                    connection: if connection.is_null() {
                        None
                    } else {
                        let object: glib::Object = unsafe { from_glib_none(connection) };
                        Some(saved_path(&object))
                    },
                    name: unsafe { string(nm_active_connection_get_id(active.as_ptr())) },
                    kind: unsafe {
                        string(nm_active_connection_get_connection_type(active.as_ptr()))
                    },
                    state: unsafe { nm_active_connection_get_state(active.as_ptr()) },
                    devices: unsafe {
                        native::objects(nm_active_connection_get_devices(active.as_ptr()))
                    }
                    .iter()
                    .map(native::path)
                    .collect(),
                }
            })
            .collect()
    }
    pub fn access_point_snapshot(&self) -> Vec<AccessPoint> {
        let mut points = Vec::new();
        for device in self
            .devices()
            .iter()
            .filter(|device| native::device_kind(device) == 2)
        {
            let active = unsafe { nm_device_wifi_get_active_access_point(device.as_ptr()) };
            for ap in unsafe { native::objects(nm_device_wifi_get_access_points(device.as_ptr())) }
            {
                let pointer = unsafe { nm_access_point_get_ssid(ap.as_ptr()) };
                let ssid = if pointer.is_null() {
                    Vec::new()
                } else {
                    let bytes: glib::Bytes = unsafe { from_glib_none(pointer) };
                    bytes.to_vec()
                };
                let name = if ssid.is_empty() {
                    String::new()
                } else {
                    let pointer = unsafe { nm_utils_ssid_to_utf8(ssid.as_ptr(), ssid.len()) };
                    if pointer.is_null() {
                        String::new()
                    } else {
                        let text: glib::GString = unsafe { from_glib_full(pointer) };
                        text.into()
                    }
                };
                points.push(AccessPoint {
                    id: native::path(&ap),
                    device: native::path(device),
                    ssid,
                    name,
                    strength: unsafe { nm_access_point_get_strength(ap.as_ptr()) },
                    flags: unsafe { nm_access_point_get_flags(ap.as_ptr()) },
                    wpa_flags: unsafe { nm_access_point_get_wpa_flags(ap.as_ptr()) },
                    rsn_flags: unsafe { nm_access_point_get_rsn_flags(ap.as_ptr()) },
                    active: active == ap.as_ptr(),
                });
            }
        }
        points
    }
}
