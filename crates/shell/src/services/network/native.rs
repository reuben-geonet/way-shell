//! The small libnm surface needed for inventory and asynchronous radio actions.
//! Every returned native object is retained before control returns to GLib.
use super::{Device, NetworkState};
use gio::prelude::*;
use glib::gobject_ffi::GObject;
use glib::translate::*;
use std::{
    ffi::{CStr, CString, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
};
unsafe extern "C" {
    fn nm_client_get_type() -> glib::ffi::GType;
    fn nm_client_get_devices(client: *mut GObject) -> *const glib::ffi::GPtrArray;
    fn nm_client_get_state(client: *mut GObject) -> u32;
    fn nm_client_get_primary_connection(client: *mut GObject) -> *mut GObject;
    fn nm_client_get_activating_connection(client: *mut GObject) -> *mut GObject;
    fn nm_active_connection_get_devices(connection: *mut GObject) -> *const glib::ffi::GPtrArray;
    fn nm_device_get_device_type(device: *mut GObject) -> u32;
    fn nm_device_get_state(device: *mut GObject) -> u32;
    fn nm_object_get_path(object: *mut GObject) -> *const c_char;
    fn nm_client_dbus_call(
        client: *mut GObject,
        path: *const c_char,
        interface: *const c_char,
        method: *const c_char,
        parameters: *mut glib::ffi::GVariant,
        reply_type: *const glib::ffi::GVariantType,
        timeout: i32,
        cancel: *mut gio::ffi::GCancellable,
        callback: gio::ffi::GAsyncReadyCallback,
        data: *mut c_void,
    );
    fn nm_client_dbus_call_finish(
        client: *mut GObject,
        result: *mut gio::ffi::GAsyncResult,
        error: *mut *mut glib::ffi::GError,
    ) -> *mut glib::ffi::GVariant;
}
#[derive(Clone)]
pub(super) struct Client {
    object: glib::Object,
}
pub(super) fn create(
    connection: Option<&gio::DBusConnection>,
    cancel: &gio::Cancellable,
    done: impl FnOnce(Result<Client, glib::Error>) + 'static,
) {
    // libnm owns this GType; construction and initialization stay on our GLib context.
    let mut builder =
        gio::AsyncInitable::builder_with_type(unsafe { from_glib(nm_client_get_type()) });
    if let Some(connection) = connection {
        builder = builder.property("dbus-connection", connection);
    }
    builder.property("instance-flags", 1_u32).build(
        glib::Priority::DEFAULT,
        Some(cancel),
        move |result| done(result.map(|object| Client { object })),
    );
}
unsafe fn objects(array: *const glib::ffi::GPtrArray) -> Vec<glib::Object> {
    if array.is_null() {
        return Vec::new();
    }
    let array = unsafe { &*array };
    if array.len == 0 {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(array.pdata, array.len as usize) }
        .iter()
        .filter_map(|&pointer| {
            if pointer.is_null() {
                None
            } else {
                Some(unsafe { from_glib_none(pointer.cast::<GObject>()) })
            }
        })
        .collect()
}
pub(super) fn path(object: &glib::Object) -> String {
    let value = unsafe { nm_object_get_path(object.as_ptr()) };
    if value.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned()
    }
}
impl Client {
    pub fn object(&self) -> glib::Object {
        self.object.clone()
    }
    pub fn owner(&self) -> Option<String> {
        self.object.property("dbus-name-owner")
    }
    pub fn connection(&self) -> Option<gio::DBusConnection> {
        self.object.property("dbus-connection")
    }
    pub fn devices(&self) -> Vec<glib::Object> {
        unsafe { objects(nm_client_get_devices(self.object.as_ptr())) }
    }
    pub fn active(&self) -> Vec<glib::Object> {
        unsafe {
            [
                nm_client_get_primary_connection(self.object.as_ptr()),
                nm_client_get_activating_connection(self.object.as_ptr()),
            ]
        }
        .into_iter()
        .filter_map(|pointer| {
            if pointer.is_null() {
                None
            } else {
                Some(unsafe { from_glib_none(pointer) })
            }
        })
        .collect()
    }
    pub fn snapshot(&self) -> NetworkState {
        if !self.object.property::<bool>("nm-running") {
            return NetworkState::default();
        }
        let state = unsafe { nm_client_get_state(self.object.as_ptr()) };
        let connection = unsafe {
            match state {
                40 => nm_client_get_activating_connection(self.object.as_ptr()),
                70 => nm_client_get_primary_connection(self.object.as_ptr()),
                _ => std::ptr::null_mut(),
            }
        };
        let primary = if connection.is_null() {
            None
        } else {
            unsafe { objects(nm_active_connection_get_devices(connection)) }
                .first()
                .map(path)
        };
        NetworkState {
            available: true,
            state,
            networking_enabled: self.object.property("networking-enabled"),
            wireless_enabled: self.object.property("wireless-enabled"),
            wireless_hardware_enabled: self.object.property("wireless-hardware-enabled"),
            primary,
            devices: self
                .devices()
                .iter()
                .map(|device| Device {
                    id: path(device),
                    interface: device
                        .property::<Option<String>>("interface")
                        .unwrap_or_default(),
                    kind: unsafe { nm_device_get_device_type(device.as_ptr()) },
                    state: unsafe { nm_device_get_state(device.as_ptr()) },
                })
                .collect(),
        }
    }
    pub fn call(
        &self,
        interface: &str,
        method: &str,
        parameters: &glib::Variant,
        cancel: &gio::Cancellable,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) {
        struct Callback {
            client: Client,
            done: Box<dyn FnOnce(Result<(), glib::Error>)>,
        }
        unsafe extern "C" fn finish(
            _source: *mut GObject,
            result: *mut gio::ffi::GAsyncResult,
            data: *mut c_void,
        ) {
            let result = catch_unwind(AssertUnwindSafe(|| {
                let callback = unsafe { Box::from_raw(data.cast::<Callback>()) };
                let mut error = std::ptr::null_mut();
                let result = unsafe {
                    nm_client_dbus_call_finish(callback.client.object.as_ptr(), result, &mut error)
                };
                let reply = if !error.is_null() {
                    Err(unsafe { from_glib_full(error) })
                } else if result.is_null() {
                    Err(glib::Error::new(
                        gio::IOErrorEnum::Failed,
                        "NetworkManager returned no result",
                    ))
                } else {
                    let _: glib::Variant = unsafe { from_glib_full(result) };
                    Ok(())
                };
                (callback.done)(reply);
            }));
            if result.is_err() {
                glib::g_critical!("way-shell", "NetworkManager callback panicked");
            }
        }
        let interface = CString::new(interface).unwrap();
        let method = CString::new(method).unwrap();
        let data = Box::into_raw(Box::new(Callback {
            client: self.clone(),
            done: Box::new(done),
        }));
        unsafe {
            nm_client_dbus_call(
                self.object.as_ptr(),
                c"/org/freedesktop/NetworkManager".as_ptr(),
                interface.as_ptr(),
                method.as_ptr(),
                parameters.to_glib_none().0,
                glib::VariantTy::UNIT.as_ptr(),
                2000,
                cancel.to_glib_none().0,
                Some(finish),
                data.cast(),
            );
        }
    }
}
