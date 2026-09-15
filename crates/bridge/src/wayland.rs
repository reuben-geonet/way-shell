//! Temporary C GObject facade. C widgets receive owned snapshots and opaque
//! identifiers; every actual Wayland proxy stays inside the Rust service.
use gio::prelude::*;
use glib::{ffi, subclass::prelude::*, translate::*};
use std::{
    cell::RefCell,
    ffi::{c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::OnceLock,
    time::Duration,
};
use way_shell::services::wayland::{Toplevel, ToplevelAction, WaylandService};

#[repr(C)]
pub struct CTop {
    header: i32,
    toplevel: *mut c_void,
    app_id: *mut c_char,
    title: *mut c_char,
    entered: i32,
    activated: i32,
    closed: i32,
    state: i32,
}
unsafe extern "C" fn free_top(pointer: *mut c_void) {
    if pointer.is_null() {
        return;
    }
    unsafe {
        let top = &*pointer.cast::<CTop>();
        ffi::g_free(top.app_id.cast());
        ffi::g_free(top.title.cast());
        ffi::g_free(pointer);
    }
}
fn new_top(top: &Toplevel) -> *mut CTop {
    unsafe {
        let pointer = ffi::g_malloc0(std::mem::size_of::<CTop>()).cast::<CTop>();
        *pointer = CTop {
            header: 3,
            toplevel: top.id as usize as *mut c_void,
            app_id: top.app_id.as_ref().map_or(std::ptr::null_mut(), |v| {
                ffi::g_strndup(v.as_ptr().cast(), v.len())
            }),
            title: top.title.as_ref().map_or(std::ptr::null_mut(), |v| {
                ffi::g_strndup(v.as_ptr().cast(), v.len())
            }),
            entered: top.entered_event.into(),
            activated: top.activation_event.into(),
            closed: 0,
            state: if top.active {
                2
            } else if top.maximized {
                0
            } else if top.minimized {
                1
            } else if top.fullscreen {
                3
            } else {
                0
            },
        };
        pointer
    }
}
glib::wrapper! {
    pub struct TopTable(Shared<ffi::GHashTable>);
    match fn { ref => |ptr| unsafe { ffi::g_hash_table_ref(ptr) }, unref => |ptr| unsafe { ffi::g_hash_table_unref(ptr) }, type_ => || ffi::g_hash_table_get_type(), }
}
impl Default for TopTable {
    fn default() -> Self {
        unsafe {
            from_glib_full(ffi::g_hash_table_new_full(
                Some(ffi::g_direct_hash),
                Some(ffi::g_direct_equal),
                None,
                Some(free_top),
            ))
        }
    }
}
impl TopTable {
    fn pointer(&self) -> *mut ffi::GHashTable {
        self.to_glib_none().0
    }
    fn upsert(&self, top: &Toplevel) {
        unsafe {
            ffi::g_hash_table_replace(
                self.pointer(),
                top.id as usize as *mut c_void,
                new_top(top).cast(),
            );
        }
    }
    fn remove(&self, id: u64) {
        unsafe {
            ffi::g_hash_table_remove(self.pointer(), id as usize as *mut c_void);
        }
    }
}
mod imp {
    use super::*;
    #[derive(Default)]
    pub struct WaylandAdapter {
        pub service: RefCell<Option<WaylandService>>,
        pub handlers: RefCell<Vec<glib::SignalHandlerId>>,
        pub tops: TopTable,
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
                    glib::subclass::Signal::builder("top-level-changed")
                        .param_types([TopTable::static_type(), glib::Type::POINTER])
                        .build(),
                    glib::subclass::Signal::builder("top-level-removed")
                        .param_types([glib::Type::POINTER])
                        .build(),
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
        for signal in ["toplevel-changed", "toplevel-removed"] {
            let weak = adapter.downgrade();
            let handler = service.connect_local(signal, false, move |values| {
                if let Some(adapter) = weak.upgrade() {
                    let top = values[1].get::<Toplevel>().unwrap();
                    let pointer = new_top(&top);
                    // A signal snapshot is separate from the table allocation,
                    // so reentrant widget callbacks cannot free its arguments.
                    let argument: *mut c_void = pointer.cast();
                    if signal == "toplevel-changed" {
                        adapter.imp().tops.upsert(&top);
                        adapter.emit_by_name::<()>(
                            "top-level-changed",
                            &[&adapter.imp().tops, &argument],
                        );
                    } else {
                        adapter.emit_by_name::<()>("top-level-removed", &[&argument]);
                        adapter.imp().tops.remove(top.id);
                    }
                    unsafe {
                        free_top(pointer.cast());
                    }
                }
                None
            });
            adapter.imp().handlers.borrow_mut().push(handler);
        }
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
        for top in service.toplevels() {
            adapter.imp().tops.upsert(&top);
        }
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
    wayland_foreign_toplevel_service_get_type,
    wayland_gamma_control_service_get_type,
    wayland_ksi_service_get_type
);
macro_rules! service_global {
    ($($function:ident),+ $(,)?) => {$(
        #[unsafe(no_mangle)] pub extern "C" fn $function() -> *mut glib::gobject_ffi::GObject { catch_unwind(AssertUnwindSafe(get)).unwrap_or(std::ptr::null_mut()) }
    )+};
}
service_global!(
    wayland_core_service_get_global,
    wayland_foreign_toplevel_service_get_global,
    wayland_gamma_control_service_get_global,
    wayland_ksi_service_get_global
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
/// The service is null or a live borrowed adapter on the main thread. The
/// returned table is borrowed; retain it with g_hash_table_ref if needed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wayland_foreign_toplevel_service_get_toplevels(
    pointer: *mut glib::gobject_ffi::GObject,
) -> *mut ffi::GHashTable {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(pointer) }
            .map_or(std::ptr::null_mut(), |adapter| adapter.imp().tops.pointer())
    }))
    .unwrap_or(std::ptr::null_mut())
}
unsafe fn action(
    pointer: *mut glib::gobject_ffi::GObject,
    top: *const CTop,
    action: ToplevelAction,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(adapter) = (unsafe { borrowed(pointer) }) else {
            return;
        };
        if top.is_null() {
            return;
        }
        let id = unsafe { (*top).toplevel as usize as u64 };
        if let Some(service) = adapter.service()
            && let Err(error) = service.action(id, action)
        {
            glib::g_warning!("way-shell", "Window action failed: {error}");
        }
    }));
}
macro_rules! top_action {
    ($name:ident,$action:ident) => {
        /// # Safety
        /// The service is live or null; top is null or a borrowed C snapshot.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(pointer: *mut glib::gobject_ffi::GObject, top: *const CTop) {
            unsafe {
                action(pointer, top, ToplevelAction::$action);
            }
        }
    };
}
top_action!(wayland_foreign_toplevel_service_activate, Activate);
top_action!(wayland_foreign_toplevel_service_close, Close);
top_action!(wayland_foreign_toplevel_service_maximize, Maximize);
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
/// # Safety
/// The service and widget are null or live borrowed objects on the GTK thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wayland_ksi_inhibit(
    pointer: *mut glib::gobject_ffi::GObject,
    widget: *mut gtk::ffi::GtkWidget,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if widget.is_null() {
            return 0;
        }
        let Some(service) = unsafe { borrowed(pointer) }.and_then(|adapter| adapter.service())
        else {
            return 0;
        };
        let widget: Borrowed<gtk::Widget> = unsafe { from_glib_borrow(widget) };
        match service.inhibit_shortcuts(&*widget) {
            Ok(()) => 1,
            Err(error) => {
                glib::g_debug!(
                    "way-shell",
                    "Could not inhibit compositor shortcuts: {error}"
                );
                0
            }
        }
    }))
    .unwrap_or(0)
}
/// # Safety
/// The service is null or a live borrowed adapter on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wayland_ksi_inhibit_destroy(
    pointer: *mut glib::gobject_ffi::GObject,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(pointer) }
            .and_then(|adapter| adapter.service())
            .is_some_and(|service| service.restore_shortcuts())
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_snapshot_table_owns_strings_and_opaque_identity() {
        let top = Toplevel {
            id: 123,
            title: Some("日本語".into()),
            app_id: Some("org.example.App".into()),
            ..Default::default()
        };
        let table = TopTable::default();
        table.upsert(&top);
        let retained = table.clone();
        drop(table);
        drop(top);
        unsafe {
            let pointer = ffi::g_hash_table_lookup(retained.pointer(), 123_usize as *mut c_void)
                .cast::<CTop>();
            assert!(!pointer.is_null());
            assert_eq!((*pointer).toplevel, 123_usize as *mut c_void);
            assert_eq!(
                std::ffi::CStr::from_ptr((*pointer).title).to_str().unwrap(),
                "日本語"
            );
        }
    }
}
