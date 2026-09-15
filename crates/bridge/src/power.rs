//! Temporary UpDevice mirrors for the remaining C widgets. No libupower
//! connection is created; Rust owns D-Bus subscriptions and device snapshots.
use glib::{prelude::*, subclass::prelude::*, translate::*};
use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::{CString, c_char},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::OnceLock,
};
use way_shell::services::power::{DeviceKind, PowerDevice, PowerService};

#[link(name = "upower-glib")]
unsafe extern "C" {
    fn up_device_new() -> *mut glib::gobject_ffi::GObject;
}

pub(super) struct Mirror {
    object: glib::Object,
    icon: CString,
}

impl Mirror {
    fn new() -> Self {
        // UpDevice's constructor returns one owned reference. It does not
        // connect to a bus until explicitly assigned an object path.
        Self {
            object: unsafe { from_glib_full(up_device_new()) },
            icon: CString::new("battery-missing-symbolic").unwrap(),
        }
    }

    fn update(&mut self, snapshot: &PowerDevice) {
        self.icon = CString::new(snapshot.preferred_icon_name()).unwrap();
        let _freeze = self.object.freeze_notify();
        self.object.set_properties(&[
            ("native-path", &snapshot.native_path),
            ("vendor", &snapshot.vendor),
            ("model", &snapshot.model),
            ("serial", &snapshot.serial),
            ("kind", &snapshot.kind.code()),
            ("state", &snapshot.state.code()),
            ("power-supply", &snapshot.power_supply),
            ("is-present", &snapshot.present),
            ("is-rechargeable", &snapshot.rechargeable),
            ("online", &snapshot.online),
            ("percentage", &snapshot.percentage.unwrap_or(0.0)),
            ("time-to-empty", &snapshot.time_to_empty),
            ("time-to-full", &snapshot.time_to_full),
            ("icon-name", &snapshot.icon_name),
        ]);
    }
}

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct Adapter {
        pub service: RefCell<Option<PowerService>>,
        pub handler: RefCell<Option<glib::SignalHandlerId>>,
        pub(super) devices: RefCell<HashMap<String, Mirror>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for Adapter {
        const NAME: &'static str = "WayShellUPowerAdapter";
        type Type = super::Adapter;
    }
    impl ObjectImpl for Adapter {
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
            self.devices.borrow_mut().clear();
        }
    }
}

glib::wrapper! { pub struct Adapter(ObjectSubclass<imp::Adapter>); }
impl Adapter {
    fn new(service: PowerService) -> Self {
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
        let Some(service) = self.imp().service.borrow().clone() else {
            return;
        };
        let snapshots = service.devices();
        {
            let mut devices = self.imp().devices.borrow_mut();
            devices.retain(|path, _| snapshots.iter().any(|snapshot| snapshot.path == *path));
            for snapshot in snapshots {
                devices
                    .entry(snapshot.path.clone())
                    .or_insert_with(Mirror::new)
                    .update(&snapshot);
            }
        }
        self.emit_by_name::<()>("changed", &[]);
    }
    fn primary(&self) -> Option<PowerDevice> {
        self.imp().service.borrow().as_ref()?.primary_device()
    }
}

thread_local! {
    static GLOBAL: RefCell<Option<Adapter>> = const { RefCell::new(None) };
}
pub fn shutdown() {
    GLOBAL.with(|global| {
        global.borrow_mut().take();
    });
}
#[unsafe(no_mangle)]
pub extern "C" fn upower_service_get_type() -> glib::ffi::GType {
    catch_unwind(|| Adapter::static_type().into_glib()).unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "C" fn upower_service_global_init() -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        GLOBAL.with(|global| {
            if global.borrow().is_none() {
                global.replace(Some(Adapter::new(PowerService::new())));
            }
        });
        0
    }))
    .unwrap_or(-1)
}
#[unsafe(no_mangle)]
pub extern "C" fn upower_service_get_global() -> *mut glib::gobject_ffi::GObject {
    catch_unwind(AssertUnwindSafe(|| {
        GLOBAL.with(|global| {
            global
                .borrow()
                .as_ref()
                .map_or(std::ptr::null_mut(), |value| value.as_ptr().cast())
        })
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// The pointer is null or a borrowed live GObject on the application thread.
unsafe fn borrowed(service: *mut glib::gobject_ffi::GObject) -> Option<Adapter> {
    if service.is_null() {
        return None;
    }
    let object: Borrowed<glib::Object> = unsafe { from_glib_borrow(service) };
    object.downcast_ref::<Adapter>().cloned()
}

/// # Safety
/// The service is null or live on the application thread. The result is borrowed
/// until the next service update; C may retain it with g_object_ref if needed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn upower_service_get_primary_device(
    service: *mut glib::gobject_ffi::GObject,
) -> *mut glib::gobject_ffi::GObject {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(adapter) = (unsafe { borrowed(service) }) else {
            return std::ptr::null_mut();
        };
        let Some(primary) = adapter.primary() else {
            return std::ptr::null_mut();
        };
        adapter
            .imp()
            .devices
            .borrow()
            .get(&primary.path)
            .map_or(std::ptr::null_mut(), |mirror| mirror.object.as_ptr())
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// The service is null or live on the application thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn upower_service_primary_is_bat(
    service: *mut glib::gobject_ffi::GObject,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(service) }
            .and_then(|adapter| adapter.primary())
            .is_some_and(|device| device.kind == DeviceKind::Battery) as i32
    }))
    .unwrap_or(0)
}

/// # Safety
/// The service is null or live on the application thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn upower_service_primary_has_percentage(
    service: *mut glib::gobject_ffi::GObject,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(service) }
            .and_then(|adapter| adapter.primary())
            .is_some_and(|device| device.percentage.is_some()) as i32
    }))
    .unwrap_or(0)
}

/// # Safety
/// The device is null or a live borrowed mirror on the application thread. The
/// returned string is borrowed until the next update and must not be freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn upower_device_map_icon_name(
    device: *mut glib::gobject_ffi::GObject,
) -> *const c_char {
    catch_unwind(AssertUnwindSafe(|| {
        GLOBAL.with(|global| {
            global
                .borrow()
                .as_ref()
                .and_then(|adapter| {
                    adapter
                        .imp()
                        .devices
                        .borrow()
                        .values()
                        .find(|mirror| mirror.object.as_ptr() == device)
                        .map(|mirror| mirror.icon.as_ptr())
                })
                .unwrap_or(c"battery-missing-symbolic".as_ptr())
        })
    }))
    .unwrap_or(c"battery-missing-symbolic".as_ptr())
}

/// Share the running service with Rust UI during the startup migration.
pub(crate) fn service() -> Option<PowerService> {
    GLOBAL.with(|global| {
        global
            .borrow()
            .as_ref()
            .and_then(|adapter| adapter.imp().service.borrow().clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_mirror_properties_preserve_native_types_and_release() {
        let mut mirror = Mirror::new();
        let snapshot = PowerDevice {
            path: "/fixture".into(),
            native_path: "BAT0".into(),
            vendor: "Fixture".into(),
            model: "日本語".into(),
            serial: "1".into(),
            kind: DeviceKind::Battery,
            state: way_shell::services::power::DeviceState::Discharging,
            power_supply: true,
            present: true,
            rechargeable: true,
            online: false,
            percentage: Some(90.),
            time_to_empty: 3600,
            time_to_full: 0,
            icon_name: "original".into(),
        };
        mirror.update(&snapshot);
        assert_eq!(mirror.object.property::<f64>("percentage"), 90.);
        assert_eq!(mirror.object.property::<i64>("time-to-empty"), 3600);
        assert_eq!(mirror.object.property::<String>("model"), "日本語");
        assert_eq!(mirror.icon.to_str().unwrap(), "battery-level-90-symbolic");
        let address = mirror.object.as_ptr();
        let retained = mirror.object.clone();
        let adapter: Adapter = glib::Object::new();
        adapter
            .imp()
            .devices
            .borrow_mut()
            .insert(snapshot.path.clone(), mirror);
        GLOBAL.with(|global| {
            global.replace(Some(adapter.clone()));
        });
        assert_eq!(
            unsafe { std::ffi::CStr::from_ptr(upower_device_map_icon_name(address)) }
                .to_str()
                .unwrap(),
            "battery-level-90-symbolic"
        );
        let mut changed = snapshot.clone();
        changed.percentage = Some(50.);
        adapter
            .imp()
            .devices
            .borrow_mut()
            .get_mut(&snapshot.path)
            .unwrap()
            .update(&changed);
        assert_eq!(
            retained.as_ptr(),
            address,
            "updates must preserve the C mirror object"
        );
        assert_eq!(retained.property::<f64>("percentage"), 50.);
        let weak = retained.downgrade();
        shutdown();
        drop(adapter);
        assert!(
            weak.upgrade().is_some(),
            "C may retain an old device snapshot"
        );
        drop(retained);
        assert!(weak.upgrade().is_none());
    }
}
