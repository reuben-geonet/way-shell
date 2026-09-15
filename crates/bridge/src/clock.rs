use glib::{prelude::*, translate::*};
use std::{
    cell::RefCell,
    panic::{AssertUnwindSafe, catch_unwind},
};
use way_shell::services::clock::ClockService;

thread_local! {
    static GLOBAL: RefCell<Option<ClockService>> = const { RefCell::new(None) };
}

pub fn shutdown() {
    GLOBAL.with(|global| {
        if let Some(clock) = global.borrow_mut().take() {
            clock.set_enabled(false);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn clock_service_get_type() -> glib::ffi::GType {
    catch_unwind(|| ClockService::static_type().into_glib()).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn clock_service_global_init() -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        GLOBAL.with(|global| {
            if global.borrow().is_none() {
                global.replace(Some(ClockService::new()));
            }
            0
        })
    }))
    .unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub extern "C" fn clock_service_get_global() -> *mut glib::gobject_ffi::GObject {
    catch_unwind(AssertUnwindSafe(|| {
        GLOBAL.with(|global| {
            global
                .borrow()
                .as_ref()
                .map_or(std::ptr::null_mut(), |clock| clock.as_ptr().cast())
        })
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// `service` is null or a live, borrowed GObject on the application thread.
unsafe fn borrowed(service: *mut glib::gobject_ffi::GObject) -> Option<ClockService> {
    if service.is_null() {
        return None;
    }
    let object: Borrowed<glib::Object> = unsafe { from_glib_borrow(service) };
    object.downcast_ref::<ClockService>().cloned()
}

/// # Safety
/// `service` is null or a live, borrowed GObject on the application thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clock_service_get_enabled(
    service: *mut glib::gobject_ffi::GObject,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(service) }.is_some_and(|clock| clock.is_enabled()) as i32
    }))
    .unwrap_or(0)
}

/// # Safety
/// `service` is null or a live, borrowed GObject on the application thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clock_service_set_enabled(
    service: *mut glib::gobject_ffi::GObject,
    enabled: i32,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if let Some(clock) = unsafe { borrowed(service) } {
            clock.set_enabled(enabled != 0);
            i32::from(clock.is_enabled())
        } else {
            0
        }
    }))
    .unwrap_or(0)
}

/// Share the running service with Rust UI during the startup migration.
pub(crate) fn service() -> Option<ClockService> {
    GLOBAL.with(|global| global.borrow().clone())
}
