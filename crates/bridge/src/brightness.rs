//! Temporary C API around the Rust brightness service.
use glib::{prelude::*, translate::*};
use std::{
    cell::RefCell,
    ffi::c_char,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
};
use way_shell::services::brightness::{BrightnessService, ControlKind};

struct Binding {
    service: BrightnessService,
    logind: way_shell::services::logind::LogindService,
    handler: Option<glib::SignalHandlerId>,
}
impl Drop for Binding {
    fn drop(&mut self) {
        if let Some(handler) = self.handler.take() {
            self.logind.disconnect(handler);
        }
        self.service.set_backend_available(false);
    }
}
thread_local! { static GLOBAL: RefCell<Option<Binding>> = const { RefCell::new(None) }; }
pub(crate) fn service() -> Option<BrightnessService> {
    GLOBAL.with(|global| {
        global
            .borrow()
            .as_ref()
            .map(|binding| binding.service.clone())
    })
}
pub fn shutdown() {
    GLOBAL.with(|global| {
        global.borrow_mut().take();
    });
}
#[unsafe(no_mangle)]
pub extern "C" fn brightness_service_get_type() -> glib::ffi::GType {
    catch_unwind(|| BrightnessService::static_type().into_glib()).unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "C" fn brightness_service_global_init() -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        GLOBAL.with(|global| {
            if global.borrow().is_some() {
                return 0;
            }
            let Some(logind) = super::logind::service() else {
                glib::g_warning!(
                    "way-shell",
                    "Brightness needs the initialized logind service"
                );
                return -1;
            };
            let controller = logind.clone();
            let apply = Rc::new(move |kind: ControlKind, name: &str, value, done| {
                controller.set_brightness(kind.subsystem(), name, value, done)
            });
            match BrightnessService::new(apply) {
                Ok(service) => {
                    service.set_backend_available(logind.state().session.is_some());
                    let weak = service.downgrade();
                    let handler = logind.connect_local("changed", false, move |values| {
                        if let Some(service) = weak.upgrade() {
                            let logind = values[0]
                                .get::<way_shell::services::logind::LogindService>()
                                .unwrap();
                            service.set_backend_available(logind.state().session.is_some());
                        }
                        None
                    });
                    global.replace(Some(Binding {
                        service,
                        logind,
                        handler: Some(handler),
                    }));
                    0
                }
                Err(error) => {
                    glib::g_warning!("way-shell", "Could not initialize brightness: {error}");
                    -1
                }
            }
        })
    }))
    .unwrap_or(-1)
}
#[unsafe(no_mangle)]
pub extern "C" fn brightness_service_get_global() -> *mut glib::gobject_ffi::GObject {
    catch_unwind(AssertUnwindSafe(|| {
        GLOBAL.with(|global| {
            global
                .borrow()
                .as_ref()
                .map_or(std::ptr::null_mut(), |binding| {
                    binding.service.as_ptr().cast()
                })
        })
    }))
    .unwrap_or(std::ptr::null_mut())
}
/// # Safety
/// The pointer is null or a borrowed live GObject on the application thread.
unsafe fn borrowed(service: *mut glib::gobject_ffi::GObject) -> Option<BrightnessService> {
    if service.is_null() {
        return None;
    }
    let object: Borrowed<glib::Object> = unsafe { from_glib_borrow(service) };
    object.downcast_ref::<BrightnessService>().cloned()
}
fn completed(result: Result<(), glib::Error>) {
    if let Err(error) = result {
        glib::g_warning!("way-shell", "Brightness operation failed: {error}");
    }
}
macro_rules! adjust {
    ($name:ident, $kind:expr, $up:expr) => {
        /// # Safety
        /// The service is null or live on the application thread.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(service: *mut glib::gobject_ffi::GObject) {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                if let Some(service) = unsafe { borrowed(service) } {
                    service.adjust($kind, $up, completed);
                }
            }));
        }
    };
}
adjust!(
    brightness_service_backlight_up,
    ControlKind::Backlight,
    true
);
adjust!(
    brightness_service_backlight_down,
    ControlKind::Backlight,
    false
);
adjust!(brightness_service_keyboard_up, ControlKind::Keyboard, true);
adjust!(
    brightness_service_keyboard_down,
    ControlKind::Keyboard,
    false
);
/// # Safety
/// The service is null or live on the application thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn brightness_service_set_backlight(
    service: *mut glib::gobject_ffi::GObject,
    fraction: f32,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(service) = unsafe { borrowed(service) } {
            service.set_backlight(f64::from(fraction), completed);
        }
    }));
}
/// # Safety
/// The service is null or live on the application thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn brightness_service_set_keyboard(
    service: *mut glib::gobject_ffi::GObject,
    value: u32,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(service) = unsafe { borrowed(service) } {
            service.set_keyboard(value, completed);
        }
    }));
}
macro_rules! getter {
    ($name:ident, $result:ty, $kind:expr, $value:expr) => {
        /// # Safety
        /// The service is null or live on the application thread.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(service: *mut glib::gobject_ffi::GObject) -> $result {
            catch_unwind(AssertUnwindSafe(|| {
                unsafe { borrowed(service) }
                    .and_then(|service| service.snapshot($kind))
                    .map_or(0 as $result, $value)
            }))
            .unwrap_or(0 as $result)
        }
    };
}
getter!(
    brightness_service_get_backlight,
    f32,
    ControlKind::Backlight,
    |device| device.fraction() as f32
);
getter!(
    brightness_service_get_keyboard,
    u32,
    ControlKind::Keyboard,
    |device| device.current
);
getter!(
    brightness_service_get_keyboard_max,
    u32,
    ControlKind::Keyboard,
    |device| device.maximum
);
/// # Safety
/// The service is null or live on the application thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn brightness_service_has_backlight_brightness(
    service: *mut glib::gobject_ffi::GObject,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(service) }
            .is_some_and(|service| service.is_available(ControlKind::Backlight)) as i32
    }))
    .unwrap_or(0)
}
/// # Safety
/// The service is null or live on the application thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn brightness_service_has_keyboard_brightness(
    service: *mut glib::gobject_ffi::GObject,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(service) }
            .is_some_and(|service| service.is_available(ControlKind::Keyboard)) as i32
    }))
    .unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "C" fn brightness_service_map_icon(
    _service: *mut glib::gobject_ffi::GObject,
) -> *const c_char {
    c"display-brightness-symbolic".as_ptr()
}
