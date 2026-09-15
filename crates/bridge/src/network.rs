//! Temporary inventory adapter while C still implements connection operations.
use gio::prelude::*;
use glib::{ffi, subclass::prelude::*, translate::*};
use std::{
    cell::RefCell,
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::OnceLock,
};
use way_shell::services::network::{NetworkService, NetworkState};
unsafe extern "C" fn release_object(pointer: *mut c_void) {
    unsafe {
        glib::gobject_ffi::g_object_unref(pointer.cast());
    }
}
glib::wrapper! {
    pub struct ObjectArray(Shared<ffi::GPtrArray>);
    match fn {ref=>|pointer|unsafe {ffi::g_ptr_array_ref(pointer)},unref=>|pointer|unsafe {ffi::g_ptr_array_unref(pointer)},type_=>||ffi::g_ptr_array_get_type(),}
}
impl Default for ObjectArray {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}
impl ObjectArray {
    fn new(objects: Vec<glib::Object>) -> Self {
        unsafe {
            let array = ffi::g_ptr_array_new_with_free_func(Some(release_object));
            for object in objects {
                let pointer: *mut glib::gobject_ffi::GObject = object.into_glib_ptr();
                ffi::g_ptr_array_add(array, pointer.cast());
            }
            from_glib_full(array)
        }
    }
    fn pointer(&self) -> *mut ffi::GPtrArray {
        self.to_glib_none().0
    }
}
mod imp {
    use super::*;
    #[derive(Default)]
    pub struct InventoryAdapter {
        pub service: RefCell<Option<NetworkService>>,
        pub handler: RefCell<Option<glib::SignalHandlerId>>,
        pub devices: RefCell<ObjectArray>,
        pub primary: RefCell<Option<glib::Object>>,
        pub client: RefCell<Option<glib::Object>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for InventoryAdapter {
        const NAME: &'static str = "WayShellNetworkInventoryAdapter";
        type Type = super::InventoryAdapter;
    }
    impl ObjectImpl for InventoryAdapter {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("changed").build()])
        }
        fn dispose(&self) {
            if let Some(service) = self.service.borrow_mut().take()
                && let Some(handler) = self.handler.borrow_mut().take()
            {
                service.disconnect(handler);
            }
            self.devices.replace(ObjectArray::default());
            self.primary.borrow_mut().take();
            self.client.borrow_mut().take();
        }
    }
}
glib::wrapper! {pub struct InventoryAdapter(ObjectSubclass<imp::InventoryAdapter>);}
impl InventoryAdapter {
    fn new(service: NetworkService) -> Self {
        let adapter: Self = glib::Object::new();
        let weak = adapter.downgrade();
        let handler = service.connect_local("changed", false, move |_| {
            if let Some(adapter) = weak.upgrade() {
                adapter.refresh();
            }
            None
        });
        adapter.imp().service.replace(Some(service));
        adapter.imp().handler.replace(Some(handler));
        adapter.refresh();
        adapter
    }
    fn refresh(&self) {
        let Some(service) = self.service() else {
            return;
        };
        self.imp()
            .devices
            .replace(ObjectArray::new(service.compatibility_devices()));
        self.imp().primary.replace(service.compatibility_primary());
        self.imp().client.replace(if service.state().available {
            service.compatibility_client()
        } else {
            None
        });
        self.emit_by_name::<()>("changed", &[]);
    }
    fn service(&self) -> Option<NetworkService> {
        self.imp().service.borrow().clone()
    }
    fn state(&self) -> NetworkState {
        self.service()
            .map_or_else(NetworkState::default, |service| service.state())
    }
}
thread_local! {static GLOBAL:RefCell<Option<glib::WeakRef<InventoryAdapter>>>=const{RefCell::new(None)};}
pub fn shutdown() {
    GLOBAL.with(|global| {
        if let Some(adapter) = global.borrow_mut().take().and_then(|weak| weak.upgrade())
            && let Some(service) = adapter.service()
        {
            service.stop();
        }
    });
}
#[unsafe(no_mangle)]
pub extern "C" fn way_shell_network_inventory_new() -> *mut glib::gobject_ffi::GObject {
    catch_unwind(AssertUnwindSafe(|| {
        let adapter = InventoryAdapter::new(NetworkService::new());
        GLOBAL.with(|global| global.replace(Some(adapter.downgrade())));
        adapter.upcast::<glib::Object>().into_glib_ptr()
    }))
    .unwrap_or(std::ptr::null_mut())
}
unsafe fn borrowed(pointer: *mut glib::gobject_ffi::GObject) -> Option<InventoryAdapter> {
    if pointer.is_null() {
        return None;
    }
    let object: Borrowed<glib::Object> = unsafe { from_glib_borrow(pointer) };
    object.downcast_ref::<InventoryAdapter>().cloned()
}
macro_rules! state_getter {
    ($name:ident,$type:ty,$get:expr) => {
        /// # Safety
        /// The pointer is null or a live borrowed adapter on the GLib thread.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(pointer: *mut glib::gobject_ffi::GObject) -> $type {
            catch_unwind(AssertUnwindSafe(|| {
                unsafe { borrowed(pointer) }.map_or(0, |adapter| ($get)(adapter.state()) as $type)
            }))
            .unwrap_or(0)
        }
    };
}
state_getter!(
    way_shell_network_inventory_state,
    u32,
    |state: NetworkState| state.state
);
state_getter!(
    way_shell_network_inventory_wifi_state,
    u32,
    |state: NetworkState| state.wifi_state()
);
state_getter!(
    way_shell_network_inventory_wifi_available,
    i32,
    |state: NetworkState| state.has_wifi()
);
state_getter!(
    way_shell_network_inventory_ethernet_available,
    i32,
    |state: NetworkState| state.has_ethernet()
);
state_getter!(
    way_shell_network_inventory_networking_enabled,
    i32,
    |state: NetworkState| state.networking_enabled
);
state_getter!(
    way_shell_network_inventory_available,
    i32,
    |state: NetworkState| state.available
);
/// # Safety
/// The pointer is a live adapter. The result is borrowed until its next changed
/// signal. g_ptr_array_ref retains all entries independently of the adapter.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn way_shell_network_inventory_devices(
    pointer: *mut glib::gobject_ffi::GObject,
) -> *mut ffi::GPtrArray {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(pointer) }.map_or(std::ptr::null_mut(), |adapter| {
            adapter.imp().devices.borrow().pointer()
        })
    }))
    .unwrap_or(std::ptr::null_mut())
}
macro_rules! object_getter {
    ($name:ident,$field:ident) => {
        /// # Safety
        /// The adapter is null or live; the returned native object is borrowed.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(
            pointer: *mut glib::gobject_ffi::GObject,
        ) -> *mut glib::gobject_ffi::GObject {
            catch_unwind(AssertUnwindSafe(|| {
                unsafe { borrowed(pointer) }
                    .and_then(|adapter| adapter.imp().$field.borrow().clone())
                    .map_or(std::ptr::null_mut(), |object| object.as_ptr())
            }))
            .unwrap_or(std::ptr::null_mut())
        }
    };
}
object_getter!(way_shell_network_inventory_client, client);
object_getter!(way_shell_network_inventory_primary, primary);
macro_rules! action {
    ($name:ident,$method:ident) => {
        /// # Safety
        /// The adapter is null or live on the GLib thread.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(pointer: *mut glib::gobject_ffi::GObject, enabled: i32) {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                if let Some(service) =
                    unsafe { borrowed(pointer) }.and_then(|adapter| adapter.service())
                {
                    service.$method(enabled != 0, |result| {
                        if let Err(error) = result {
                            glib::g_message!("way-shell", "Network change failed: {error}");
                        }
                    });
                }
            }));
        }
    };
}
action!(way_shell_network_inventory_set_wireless, set_wireless);
action!(way_shell_network_inventory_set_networking, set_networking);

fn report_action(result: Result<(), glib::Error>) {
    if let Err(error) = result {
        glib::g_message!("way-shell", "Network connection change failed: {error}");
    }
}
unsafe fn text_argument(pointer: *const std::ffi::c_char) -> Option<String> {
    if pointer.is_null() {
        None
    } else {
        unsafe { std::ffi::CStr::from_ptr(pointer) }
            .to_str()
            .ok()
            .map(str::to_owned)
    }
}
/// # Safety
/// The adapter is live on the GLib thread. Strings are null or readable,
/// NUL-terminated UTF-8; each request copies them before returning.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn way_shell_network_inventory_join(
    pointer: *mut glib::gobject_ffi::GObject,
    device: *const std::ffi::c_char,
    access_point: *const std::ffi::c_char,
    password: *const std::ffi::c_char,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(service) = (unsafe { borrowed(pointer) }).and_then(|adapter| adapter.service())
        else {
            return;
        };
        let (Some(device), Some(access_point)) = (unsafe { text_argument(device) }, unsafe {
            text_argument(access_point)
        }) else {
            return;
        };
        let supplied_password = unsafe { text_argument(password) };
        if !password.is_null() && supplied_password.is_none() {
            glib::g_message!("way-shell", "Wi-Fi password is not valid UTF-8");
            return;
        }
        service.join_access_point(
            &device,
            &access_point,
            supplied_password.as_deref(),
            report_action,
        );
    }));
}
/// # Safety
/// The adapter is live and device is null or a readable NUL-terminated path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn way_shell_network_inventory_disconnect(
    pointer: *mut glib::gobject_ffi::GObject,
    device: *const std::ffi::c_char,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(service) = (unsafe { borrowed(pointer) }).and_then(|adapter| adapter.service())
        else {
            return;
        };
        if let Some(device) = unsafe { text_argument(device) } {
            service.disconnect_device(&device, report_action);
        }
    }));
}
/// # Safety
/// The adapter is live and name is null or a readable NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn way_shell_network_inventory_set_vpn(
    pointer: *mut glib::gobject_ffi::GObject,
    name: *const std::ffi::c_char,
    enabled: i32,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(service) = (unsafe { borrowed(pointer) }).and_then(|adapter| adapter.service())
        else {
            return;
        };
        let Some(name) = (unsafe { text_argument(name) }) else {
            return;
        };
        // C widgets identify profiles by display name. Permanent Rust widgets
        // use the stable object path and can distinguish duplicate names.
        if let Some(connection) = service
            .state()
            .connections
            .iter()
            .rev()
            .find(|connection| connection.is_vpn() && connection.name == name)
        {
            service.set_vpn(&connection.id, enabled != 0, report_action);
        }
    }));
}

/// Share the running service with Rust UI during the startup migration.
pub(crate) fn service() -> Option<NetworkService> {
    GLOBAL.with(|global| {
        global
            .borrow()
            .as_ref()
            .and_then(|weak| weak.upgrade())
            .and_then(|adapter| adapter.service())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_array_owns_entries_and_adapter_disconnects() {
        let object = glib::Object::new::<glib::Object>();
        let weak = object.downgrade();
        let array = ObjectArray::new(vec![object]);
        let retained = array.clone();
        drop(array);
        assert!(weak.upgrade().is_some());
        drop(retained);
        assert!(weak.upgrade().is_none());
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                for _ in 0..20 {
                    let service = NetworkService::new();
                    let weak = service.downgrade();
                    let adapter = InventoryAdapter::new(service);
                    drop(adapter);
                    assert!(weak.upgrade().is_none());
                }
                context.block_on(glib::timeout_future(std::time::Duration::from_millis(30)));
            })
            .unwrap();
    }
}

#[cfg(test)]
mod action_tests {
    use super::*;
    use std::{collections::HashMap, ffi::CString, rc::Rc, time::Duration};
    mod fixture {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../shell/tests/common/network.rs"
        ));
    }
    #[test]
    fn c_request_copies_strings_before_returning() {
        let (_bus, address) = fixture::Bus::start();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| exercise(&context, &address))
            .unwrap();
    }
    fn exercise(context: &glib::MainContext, address: &str) {
        let daemon = fixture::connect(address);
        let connection = fixture::connect(address);
        let ap = "/org/freedesktop/NetworkManager/AccessPoint/1";
        let mut objects = fixture::inventory();
        objects
            .get_mut(&fixture::path(fixture::DEVICE))
            .unwrap()
            .get_mut(&format!("{}.Wireless", fixture::DEVICE_IFACE))
            .unwrap()
            .insert("AccessPoints".into(), vec![fixture::path(ap)].to_variant());
        objects.insert(
            fixture::path(ap),
            HashMap::from([(
                format!("{}.AccessPoint", fixture::NAME),
                HashMap::from([
                    ("Ssid".into(), b"adapter-fixture".to_vec().to_variant()),
                    ("Strength".into(), 60_u8.to_variant()),
                ]),
            )]),
        );
        let manager = gio::DBusNodeInfo::for_xml("<node><interface name='org.freedesktop.DBus.ObjectManager'><method name='GetManagedObjects'><arg type='a{oa{sa{sv}}}' direction='out'/></method></interface></node>").unwrap();
        let mut registrations = Vec::new();
        for root in ["/org/freedesktop", fixture::ROOT] {
            let objects = objects.clone();
            registrations.push(
                daemon
                    .register_object(root, &manager.interfaces()[0])
                    .method_call(move |_, _, _, _, _, _, call| {
                        call.return_value(Some(&(objects.clone(),).to_variant()))
                    })
                    .build()
                    .unwrap(),
            );
        }
        let node = gio::DBusNodeInfo::for_xml("<node><interface name='org.freedesktop.NetworkManager'><method name='AddAndActivateConnection'><arg type='a{sa{sv}}' direction='in'/><arg type='o' direction='in'/><arg type='o' direction='in'/><arg type='o' direction='out'/><arg type='o' direction='out'/></method></interface></node>").unwrap();
        let requests = Rc::new(RefCell::new(Vec::new()));
        let recorded = requests.clone();
        registrations.push(
            daemon
                .register_object(fixture::ROOT, &node.interfaces()[0])
                .method_call(move |_, _, _, _, _, args, call| {
                    recorded.borrow_mut().push(args);
                    call.return_value(Some(
                        &(
                            fixture::path("/org/freedesktop/NetworkManager/Settings/1"),
                            fixture::path(fixture::ACTIVE),
                        )
                            .to_variant(),
                    ));
                })
                .build()
                .unwrap(),
        );
        fixture::ownership(&daemon, true);
        let service = NetworkService::on_connection(&connection);
        let weak = service.downgrade();
        let adapter = InventoryAdapter::new(service);
        fixture::wait(context, || {
            adapter.state().available && !adapter.state().access_points.is_empty()
        });
        {
            let device = CString::new(fixture::DEVICE).unwrap();
            let ap = CString::new(ap).unwrap();
            let password = CString::new("adapter-dummy-password").unwrap();
            unsafe {
                way_shell_network_inventory_join(
                    adapter.as_ptr().cast(),
                    device.as_ptr(),
                    ap.as_ptr(),
                    password.as_ptr(),
                );
            }
        }
        fixture::wait(context, || !requests.borrow().is_empty());
        let (settings, device, _) = requests.borrow()[0]
            .get::<(
                HashMap<String, fixture::Properties>,
                glib::variant::ObjectPath,
                glib::variant::ObjectPath,
            )>()
            .unwrap();
        assert_eq!(device.as_str(), fixture::DEVICE);
        assert_eq!(
            settings["802-11-wireless-security"]["psk"]
                .get::<String>()
                .as_deref(),
            Some("adapter-dummy-password")
        );
        fixture::properties(
            &daemon,
            fixture::ROOT,
            fixture::NAME,
            HashMap::from([("WirelessEnabled".into(), false.to_variant())]),
        );
        fixture::wait(context, || !adapter.state().wireless_enabled);
        context.block_on(glib::timeout_future(Duration::from_millis(20)));
        drop(adapter);
        assert!(weak.upgrade().is_none());
        for registration in registrations {
            daemon.unregister_object(registration).unwrap();
        }
        connection.close_sync(gio::Cancellable::NONE).unwrap();
        daemon.close_sync(gio::Cancellable::NONE).unwrap();
    }
}
