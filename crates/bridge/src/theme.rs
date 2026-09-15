use glib::{prelude::*, translate::*};
use std::{
    cell::RefCell,
    panic::{AssertUnwindSafe, catch_unwind},
};
use way_shell::services::theme::{Theme, ThemeService};

thread_local! {
    static GLOBAL: RefCell<Option<ThemeService>> = const { RefCell::new(None) };
}

fn initialize() -> Result<ThemeService, glib::BoolError> {
    GLOBAL.with(|global| {
        if let Some(service) = global.borrow().as_ref() {
            return Ok(service.clone());
        }
        // C has already initialized GTK on the application thread. Calling the
        // safe initializer also registers that thread with gtk-rs.
        gtk::init()?;
        let display = gtk::gdk::Display::default()
            .ok_or_else(|| glib::bool_error!("No GTK display for theme service"))?;
        let service = ThemeService::new()?;
        service.attach_display(&display);
        global.replace(Some(service.clone()));
        Ok(service)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn theme_service_get_type() -> glib::ffi::GType {
    catch_unwind(|| ThemeService::static_type().into_glib()).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn theme_service_global_init() -> i32 {
    catch_unwind(AssertUnwindSafe(|| match initialize() {
        Ok(_) => 0,
        Err(error) => {
            glib::g_warning!("way-shell", "Could not initialize theme service: {error}");
            -1
        }
    }))
    .unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub extern "C" fn theme_service_get_global() -> *mut glib::gobject_ffi::GObject {
    catch_unwind(AssertUnwindSafe(|| {
        initialize().map_or(std::ptr::null_mut(), |service| service.as_ptr().cast())
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// `service` is null or a live, borrowed GObject on the GTK thread.
unsafe fn borrowed(service: *mut glib::gobject_ffi::GObject) -> Option<ThemeService> {
    if service.is_null() {
        return None;
    }
    let object: Borrowed<glib::Object> = unsafe { from_glib_borrow(service) };
    object.downcast_ref::<ThemeService>().cloned()
}

/// # Safety
/// `service` is null or a live, borrowed GObject on the GTK thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn theme_service_get_theme(service: *mut glib::gobject_ffi::GObject) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(service) }.map_or(Theme::Dark as i32, |s| s.theme() as i32)
    }))
    .unwrap_or(Theme::Dark as i32)
}

unsafe fn set_theme(service: *mut glib::gobject_ffi::GObject, theme: Theme) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(service) = unsafe { borrowed(service) }
            && let Err(error) = service.set_theme(theme)
        {
            glib::g_warning!("way-shell", "Could not change theme: {error}");
        }
    }));
}

/// # Safety
/// `service` is null or a live, borrowed GObject on the GTK thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn theme_service_set_light_theme(
    service: *mut glib::gobject_ffi::GObject,
    _enabled: i32,
) {
    unsafe {
        set_theme(service, Theme::Light);
    }
}

/// # Safety
/// `service` is null or a live, borrowed GObject on the GTK thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn theme_service_set_dark_theme(
    service: *mut glib::gobject_ffi::GObject,
    _enabled: i32,
) {
    unsafe {
        set_theme(service, Theme::Dark);
    }
}

/// Borrowed resource valid until this application thread exits.
#[unsafe(no_mangle)]
pub extern "C" fn way_shell_get_resource() -> *mut gio::ffi::GResource {
    catch_unwind(AssertUnwindSafe(|| {
        way_shell::resources::get().to_glib_none().0
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "C" fn way_shell_rust_shutdown() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        crate::clock::shutdown();
        crate::wayland::shutdown();
        GLOBAL.with(|global| {
            global.borrow_mut().take();
        })
    }));
}
