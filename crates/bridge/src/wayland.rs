//! Temporary startup and gamma-control facade for the remaining C consumers.
use gio::prelude::*;
use glib::{ffi, subclass::prelude::*, translate::*};
use std::{
    cell::RefCell,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::OnceLock,
    time::Duration,
};
use way_shell::services::wayland::WaylandService;

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct WaylandAdapter {
        pub service: RefCell<Option<WaylandService>>,
        pub handlers: RefCell<Vec<glib::SignalHandlerId>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for WaylandAdapter {
        const NAME: &'static str = "WayShellWaylandAdapter";
        type Type = super::WaylandAdapter;
    }
    impl ObjectImpl for WaylandAdapter {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("gamma-control-enabled").build(),
                    glib::subclass::Signal::builder("gamma-control-disabled").build(),
                    glib::subclass::Signal::builder("availability-changed").build(),
                ]
            })
        }
        fn dispose(&self) {
            if let Some(service) = self.service.borrow_mut().take() {
                for handler in self.handlers.borrow_mut().drain(..) {
                    service.disconnect(handler);
                }
                service.restore_shortcuts();
            }
        }
    }
}
glib::wrapper! { pub struct WaylandAdapter(ObjectSubclass<imp::WaylandAdapter>); }
impl WaylandAdapter {
    fn new(service: WaylandService) -> Self {
        let adapter: Self = glib::Object::new();
        for signal in [
            "gamma-control-enabled",
            "gamma-control-disabled",
            "capabilities-changed",
        ] {
            let weak = adapter.downgrade();
            let handler = service.connect_local(signal, false, move |_| {
                if let Some(adapter) = weak.upgrade() {
                    adapter.emit_by_name::<()>(
                        if signal == "capabilities-changed" {
                            "availability-changed"
                        } else {
                            signal
                        },
                        &[],
                    );
                }
                None
            });
            adapter.imp().handlers.borrow_mut().push(handler);
        }
        let handler = service.connect_local("failed", false, move |values| {
            let error = values[1].get::<String>().unwrap();
            glib::g_warning!("way-shell", "{error}");
            if let Some(application) = gio::Application::default() {
                application.quit();
            }
            None
        });
        adapter.imp().handlers.borrow_mut().push(handler);
        adapter.imp().service.replace(Some(service));
        adapter
    }
    fn service(&self) -> Option<WaylandService> {
        self.imp().service.borrow().clone()
    }
}
thread_local! { static GLOBAL: RefCell<Option<WaylandAdapter>> = const { RefCell::new(None) }; }
pub fn shutdown() {
    GLOBAL.with(|global| {
        global.borrow_mut().take();
    });
}
fn get() -> *mut glib::gobject_ffi::GObject {
    GLOBAL.with(|global| {
        global
            .borrow()
            .as_ref()
            .map_or(std::ptr::null_mut(), |value| value.as_ptr().cast())
    })
}
unsafe fn borrowed(pointer: *mut glib::gobject_ffi::GObject) -> Option<WaylandAdapter> {
    if pointer.is_null() {
        return None;
    }
    let object: Borrowed<glib::Object> = unsafe { from_glib_borrow(pointer) };
    object.downcast_ref::<WaylandAdapter>().cloned()
}
macro_rules! service_type {
    ($($function:ident),+ $(,)?) => {$(
        #[unsafe(no_mangle)] pub extern "C" fn $function() -> ffi::GType { catch_unwind(|| WaylandAdapter::static_type().into_glib()).unwrap_or(0) }
    )+};
}
service_type!(
    wayland_core_service_get_type,
    wayland_gamma_control_service_get_type,
);
macro_rules! service_global {
    ($($function:ident),+ $(,)?) => {$(
        #[unsafe(no_mangle)] pub extern "C" fn $function() -> *mut glib::gobject_ffi::GObject { catch_unwind(AssertUnwindSafe(get)).unwrap_or(std::ptr::null_mut()) }
    )+};
}
service_global!(
    wayland_core_service_get_global,
    wayland_gamma_control_service_get_global,
);
#[unsafe(no_mangle)]
pub extern "C" fn wayland_core_service_global_init() -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if !get().is_null() {
            return 0;
        }
        let initialize = || -> Result<WaylandAdapter, String> {
            gtk::init().map_err(|e| e.to_string())?;
            let service = WaylandService::connect()?;
            let adapter = WaylandAdapter::new(service.clone());
            // C startup expects initialized inventories. Drive the single GLib
            // loop while the asynchronous registry handshake completes.
            let context = glib::MainContext::ref_thread_default();
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !service.is_ready() {
                if let Some(error) = service.error() {
                    return Err(error);
                }
                if std::time::Instant::now() >= deadline {
                    return Err("Wayland compositor did not initialize within two seconds".into());
                }
                context.block_on(glib::timeout_future(Duration::from_millis(2)));
            }
            if service.outputs().is_empty() {
                return Err("Wayland compositor has no outputs".into());
            }
            Ok(adapter)
        };
        match initialize() {
            Ok(adapter) => {
                GLOBAL.with(|global| global.replace(Some(adapter)));
                0
            }
            Err(error) => {
                glib::g_warning!("way-shell", "Could not initialize Wayland: {error}");
                -1
            }
        }
    }))
    .unwrap_or(-1)
}
/// # Safety
/// The service is null or a live borrowed adapter on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wayland_gamma_control_service_set_temperature(
    pointer: *mut glib::gobject_ffi::GObject,
    temperature: f64,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !temperature.is_finite() || !(1000.0..=25000.0).contains(&temperature) {
            glib::g_warning!(
                "way-shell",
                "Gamma temperature must be between 1000 and 25000 K"
            );
            return;
        }
        if let Some(service) = unsafe { borrowed(pointer) }.and_then(|adapter| adapter.service())
            && let Err(error) = service.set_temperature(temperature as u32)
        {
            glib::g_warning!("way-shell", "{error}");
        }
    }));
}
/// # Safety
/// The service is null or a live borrowed adapter on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wayland_gamma_control_service_destroy(
    pointer: *mut glib::gobject_ffi::GObject,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(service) = unsafe { borrowed(pointer) }.and_then(|adapter| adapter.service()) {
            service.disable_gamma();
        }
    }));
}
/// # Safety
/// The service is null or a live borrowed adapter on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wayland_gamma_control_service_enabled(
    pointer: *mut glib::gobject_ffi::GObject,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(pointer) }
            .and_then(|adapter| adapter.service())
            .is_some_and(|service| service.gamma_enabled())
            .into()
    }))
    .unwrap_or(0)
}
/// # Safety
/// The service is null or a live borrowed adapter on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wayland_gamma_control_service_available(
    pointer: *mut glib::gobject_ffi::GObject,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(pointer) }
            .and_then(|adapter| adapter.service())
            .is_some_and(|service| service.gamma_available())
            .into()
    }))
    .unwrap_or(0)
}
/// Share the running service with Rust UI during the startup migration.
pub(crate) fn service() -> Option<WaylandService> {
    GLOBAL.with(|global| {
        global
            .borrow()
            .as_ref()
            .and_then(|adapter| adapter.service())
    })
}
