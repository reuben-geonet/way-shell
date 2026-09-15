//! Temporary C entry points for login1. GTK consumers keep the same GObject.
use glib::{prelude::*, translate::*};
use std::{
    cell::RefCell,
    ffi::{CStr, CString, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
};
use way_shell::services::logind::{LogindService, PowerAction};
thread_local! { static GLOBAL: RefCell<Option<LogindService>> = const { RefCell::new(None) }; }
pub fn service() -> Option<LogindService> {
    GLOBAL.with(|global| global.borrow().clone())
}
pub fn shutdown() {
    GLOBAL.with(|global| {
        if let Some(service) = global.borrow_mut().take() {
            service.stop();
        }
    });
}
#[unsafe(no_mangle)]
pub extern "C" fn logind_service_get_type() -> glib::ffi::GType {
    catch_unwind(|| LogindService::static_type().into_glib()).unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "C" fn logind_service_global_init() -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        GLOBAL.with(|global| {
            if global.borrow().is_none() {
                match LogindService::new() {
                    Ok(service) => {
                        global.replace(Some(service));
                    }
                    Err(error) => {
                        glib::g_message!("way-shell", "Could not initialize login1: {error}");
                        return -1;
                    }
                }
            }
            0
        })
    }))
    .unwrap_or(-1)
}
#[unsafe(no_mangle)]
pub extern "C" fn logind_service_get_global() -> *mut glib::gobject_ffi::GObject {
    catch_unwind(AssertUnwindSafe(|| {
        service().map_or(std::ptr::null_mut(), |service| service.as_ptr().cast())
    }))
    .unwrap_or(std::ptr::null_mut())
}
unsafe fn borrowed(pointer: *mut glib::gobject_ffi::GObject) -> Option<LogindService> {
    if pointer.is_null() {
        return None;
    }
    let object: Borrowed<glib::Object> = unsafe { from_glib_borrow(pointer) };
    object.downcast_ref::<LogindService>().cloned()
}
fn report(result: Result<(), glib::Error>) {
    if let Err(error) = result {
        glib::g_message!("way-shell", "login1 action failed: {error}");
    }
}
macro_rules! power_action {
    ($can:ident, $perform:ident, $action:ident) => {
        /// # Safety
        /// The pointer is null or a live borrowed service on the application thread.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $can(pointer: *mut glib::gobject_ffi::GObject) -> i32 {
            catch_unwind(AssertUnwindSafe(|| {
                unsafe { borrowed(pointer) }
                    .is_some_and(|service| service.can(PowerAction::$action)) as i32
            }))
            .unwrap_or(0)
        }
        /// # Safety
        /// The pointer is null or a live borrowed service on the application thread.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $perform(pointer: *mut glib::gobject_ffi::GObject) {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                if let Some(service) = unsafe { borrowed(pointer) } {
                    service.perform(PowerAction::$action, report);
                }
            }));
        }
    };
}
power_action!(logind_service_can_reboot, logind_service_reboot, Reboot);
power_action!(
    logind_service_can_power_off,
    logind_service_power_off,
    PowerOff
);
power_action!(logind_service_can_suspend, logind_service_suspend, Suspend);
power_action!(
    logind_service_can_hibernate,
    logind_service_hibernate,
    Hibernate
);
power_action!(
    logind_service_can_hybrid_sleep,
    logind_service_hybrid_sleep,
    HybridSleep
);
power_action!(
    logind_service_can_suspendthenhibernate,
    logind_service_suspendthenhibernate,
    SuspendThenHibernate
);
/// # Safety
/// The pointer is null or a live borrowed service on the application thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logind_service_kill_session(pointer: *mut glib::gobject_ffi::GObject) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(service) = unsafe { borrowed(pointer) } {
            service.terminate_session(report);
        }
    }));
}
/// # Safety
/// The pointer is null or a live borrowed service on the application thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logind_service_get_enabled(
    pointer: *mut glib::gobject_ffi::GObject,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(pointer) }.is_some_and(|service| service.state().available) as i32
    }))
    .unwrap_or(0)
}
/// # Safety
/// The pointer is null or a live borrowed service on the application thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logind_service_get_session_enabled(
    pointer: *mut glib::gobject_ffi::GObject,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(pointer) }.is_some_and(|service| service.state().session.is_some()) as i32
    }))
    .unwrap_or(0)
}
/// # Safety
/// The pointer is null or a live borrowed service on the application thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logind_service_get_idle_inhibit(
    pointer: *mut glib::gobject_ffi::GObject,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(pointer) }.is_some_and(|service| service.state().inhibited) as i32
    }))
    .unwrap_or(0)
}
/// # Safety
/// The pointer is null or a live borrowed service on the application thread.
/// Returns whether the request can be submitted; completion uses the existing
/// idle-inhibitor-changed signal after a descriptor is actually acquired.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logind_service_set_idle_inhibit(
    pointer: *mut glib::gobject_ffi::GObject,
    enabled: i32,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(service) = (unsafe { borrowed(pointer) }) else {
            return 0;
        };
        let available = service.state().available || enabled == 0;
        service.set_idle_inhibit(enabled != 0);
        i32::from(available)
    }))
    .unwrap_or(0)
}
type Completion = Option<unsafe extern "C" fn(i32, *const c_char, *mut c_void)>;
struct Callback {
    done: Completion,
    data: *mut c_void,
    destroy: glib::ffi::GDestroyNotify,
}
impl Callback {
    fn complete(self, result: Result<(), glib::Error>) {
        if let Some(done) = self.done {
            let error = result
                .as_ref()
                .err()
                .map(|error| CString::new(error.to_string().replace('\0', " ")).unwrap());
            unsafe {
                done(
                    result.is_ok() as i32,
                    error
                        .as_ref()
                        .map_or(std::ptr::null(), |error| error.as_ptr()),
                    self.data,
                );
            }
        } else {
            report(result);
        }
    }
}
impl Drop for Callback {
    fn drop(&mut self) {
        if let Some(destroy) = self.destroy {
            unsafe {
                destroy(self.data);
            }
        }
    }
}
/// # Safety
/// Inputs are borrowed for this call. C strings are readable and NUL-terminated.
/// Completion and destroy may use data until destroy runs exactly once; errors
/// are borrowed only for the duration of completion. Callbacks run on the GLib
/// main thread, including immediate validation errors.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logind_service_session_set_brightness_async(
    pointer: *mut glib::gobject_ffi::GObject,
    subsystem: *const c_char,
    name: *const c_char,
    brightness: u32,
    done: Completion,
    data: *mut c_void,
    destroy: glib::ffi::GDestroyNotify,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let callback = Callback {
            done,
            data,
            destroy,
        };
        let Some(service) = (unsafe { borrowed(pointer) }) else {
            callback.complete(Err(glib::Error::new(
                gio::IOErrorEnum::NotConnected,
                "login1 unavailable",
            )));
            return;
        };
        if subsystem.is_null() || name.is_null() {
            callback.complete(Err(glib::Error::new(
                gio::IOErrorEnum::InvalidArgument,
                "Missing brightness device",
            )));
            return;
        }
        let subsystem = unsafe { CStr::from_ptr(subsystem) }.to_str();
        let name = unsafe { CStr::from_ptr(name) }.to_str();
        let (Ok(subsystem), Ok(name)) = (subsystem, name) else {
            callback.complete(Err(glib::Error::new(
                gio::IOErrorEnum::InvalidArgument,
                "Invalid brightness device encoding",
            )));
            return;
        };
        service.set_brightness(subsystem, name, brightness, move |result| {
            callback.complete(result)
        });
    }));
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn null_brightness_callback_completes_and_destroys_once() {
        unsafe extern "C" fn done(success: i32, message: *const c_char, data: *mut c_void) {
            assert_eq!(success, 0);
            assert!(!message.is_null());
            unsafe {
                (*data.cast::<[u32; 2]>())[0] += 1;
            }
        }
        unsafe extern "C" fn destroy(data: *mut c_void) {
            unsafe {
                (*data.cast::<[u32; 2]>())[1] += 1;
            }
        }
        let mut counts = [0_u32; 2];
        unsafe {
            logind_service_session_set_brightness_async(
                std::ptr::null_mut(),
                c"backlight".as_ptr(),
                c"intel".as_ptr(),
                1,
                Some(done),
                (&mut counts as *mut [u32; 2]).cast(),
                Some(destroy),
            );
        }
        assert_eq!(counts, [1, 1]);
    }
}
