//! Temporary C GObject and borrowed GArray compatibility for profile widgets.
use gio::prelude::*;
use glib::{ffi, subclass::prelude::*, translate::*};
use std::{
    cell::RefCell,
    ffi::{CStr, CString, c_char},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::OnceLock,
};
use way_shell::services::power_profiles::{PowerProfilesService, ProfilesState};

glib::wrapper! {
    pub struct ProfileArray(Shared<ffi::GArray>);
    match fn {
        ref => |ptr| unsafe { ffi::g_array_ref(ptr) },
        unref => |ptr| unsafe { ffi::g_array_unref(ptr) },
        type_ => || ffi::g_array_get_type(),
    }
}
unsafe extern "C" fn clear_profile(pointer: *mut std::ffi::c_void) {
    unsafe {
        ffi::g_free((*pointer.cast::<*mut c_char>()).cast());
    }
}
impl Default for ProfileArray {
    fn default() -> Self {
        Self::new(&[])
    }
}
impl ProfileArray {
    fn new(profiles: &[String]) -> Self {
        unsafe {
            let array = ffi::g_array_new(0, 0, std::mem::size_of::<*mut c_char>() as u32);
            ffi::g_array_set_clear_func(array, Some(clear_profile));
            for profile in profiles {
                let mut string = ffi::g_strndup(profile.as_ptr().cast(), profile.len());
                ffi::g_array_append_vals(array, (&mut string as *mut *mut c_char).cast(), 1);
            }
            from_glib_full(array)
        }
    }
    fn pointer(&self) -> *mut ffi::GArray {
        self.to_glib_none().0
    }
}
mod imp {
    use super::*;
    #[derive(Default)]
    pub struct ProfileAdapter {
        pub service: RefCell<Option<PowerProfilesService>>,
        pub handler: RefCell<Option<glib::SignalHandlerId>>,
        pub state: RefCell<ProfilesState>,
        pub profiles: RefCell<ProfileArray>,
        pub active: RefCell<Option<CString>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for ProfileAdapter {
        const NAME: &'static str = "WayShellPowerProfilesAdapter";
        type Type = super::ProfileAdapter;
    }
    impl ObjectImpl for ProfileAdapter {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("profiles-changed")
                        .param_types([ProfileArray::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("active-profile-changed")
                        .param_types([String::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("availability-changed")
                        .param_types([bool::static_type()])
                        .build(),
                ]
            })
        }
        fn dispose(&self) {
            if let Some(service) = self.service.borrow_mut().take()
                && let Some(handler) = self.handler.borrow_mut().take()
            {
                service.disconnect(handler);
            }
        }
    }
}
glib::wrapper! { pub struct ProfileAdapter(ObjectSubclass<imp::ProfileAdapter>); }
impl ProfileAdapter {
    fn new(service: PowerProfilesService) -> Self {
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
        let state = self
            .imp()
            .service
            .borrow()
            .as_ref()
            .map(PowerProfilesService::state)
            .unwrap_or_default();
        let previous = self.imp().state.replace(state.clone());
        if state.profiles != previous.profiles {
            let array = ProfileArray::new(&state.profiles);
            self.imp().profiles.replace(array.clone());
            self.emit_by_name::<()>("profiles-changed", &[&array]);
        }
        if state.active != previous.active {
            self.imp().active.replace(
                state.active.as_ref().map(|value| {
                    CString::new(value.as_bytes()).expect("D-Bus strings exclude NUL")
                }),
            );
            self.emit_by_name::<()>(
                "active-profile-changed",
                &[&state.active.as_deref().unwrap_or("")],
            );
        }
        if state.available() != previous.available() {
            self.emit_by_name::<()>("availability-changed", &[&state.available()]);
        }
    }
}
thread_local! { static GLOBAL: RefCell<Option<ProfileAdapter>> = const { RefCell::new(None) }; }
pub(crate) fn service() -> Option<PowerProfilesService> {
    GLOBAL.with(|global| {
        global
            .borrow()
            .as_ref()
            .and_then(|adapter| adapter.imp().service.borrow().clone())
    })
}
pub fn shutdown() {
    GLOBAL.with(|global| {
        global.borrow_mut().take();
    });
}
#[unsafe(no_mangle)]
pub extern "C" fn power_profiles_service_get_type() -> glib::ffi::GType {
    catch_unwind(|| ProfileAdapter::static_type().into_glib()).unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "C" fn power_profiles_service_global_init() -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        GLOBAL.with(|global| {
            if global.borrow().is_none() {
                global.replace(Some(ProfileAdapter::new(PowerProfilesService::new())));
            }
            0
        })
    }))
    .unwrap_or(-1)
}
#[unsafe(no_mangle)]
pub extern "C" fn power_profiles_service_get_global() -> *mut glib::gobject_ffi::GObject {
    catch_unwind(AssertUnwindSafe(|| {
        GLOBAL.with(|global| {
            global
                .borrow()
                .as_ref()
                .map_or(std::ptr::null_mut(), |service| service.as_ptr().cast())
        })
    }))
    .unwrap_or(std::ptr::null_mut())
}
unsafe fn borrowed(pointer: *mut glib::gobject_ffi::GObject) -> Option<ProfileAdapter> {
    if pointer.is_null() {
        return None;
    }
    let object: Borrowed<glib::Object> = unsafe { from_glib_borrow(pointer) };
    object.downcast_ref::<ProfileAdapter>().cloned()
}
/// # Safety
/// `pointer` is null or a live borrowed service on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn power_profiles_service_get_enabled(
    pointer: *mut glib::gobject_ffi::GObject,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(pointer) }.is_some_and(|adapter| adapter.imp().state.borrow().available())
            as i32
    }))
    .unwrap_or(0)
}
/// # Safety
/// `pointer` is null or a live borrowed service. The result is borrowed until
/// the next profiles-changed notification; retain it with g_array_ref if needed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn power_profiles_service_get_profiles(
    pointer: *mut glib::gobject_ffi::GObject,
) -> *mut ffi::GArray {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(pointer) }.map_or(std::ptr::null_mut(), |adapter| {
            adapter.imp().profiles.borrow().pointer()
        })
    }))
    .unwrap_or(std::ptr::null_mut())
}
/// # Safety
/// `pointer` is null or a live borrowed service. The result is borrowed until
/// the next active-profile-changed notification, and must not be freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn power_profiles_service_get_active_profile(
    pointer: *mut glib::gobject_ffi::GObject,
) -> *const c_char {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(pointer) }.map_or(std::ptr::null(), |adapter| {
            adapter
                .imp()
                .active
                .borrow()
                .as_ref()
                .map_or(std::ptr::null(), |active| active.as_ptr())
        })
    }))
    .unwrap_or(std::ptr::null())
}
/// # Safety
/// `pointer` is null or a live borrowed service; `profile` is null or a readable
/// NUL-terminated string, copied before this function returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn power_profiles_service_set_profile(
    pointer: *mut glib::gobject_ffi::GObject,
    profile: *const c_char,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(adapter) = (unsafe { borrowed(pointer) }) else {
            return;
        };
        if profile.is_null() {
            return;
        }
        let Ok(profile) = (unsafe { CStr::from_ptr(profile) }).to_str() else {
            return;
        };
        let service = adapter.imp().service.borrow().clone();
        if let Some(service) = service {
            service.set_profile(profile, |result| {
                if let Err(error) = result {
                    glib::g_message!("way-shell", "Could not change power profile: {error}");
                }
            });
        }
    }));
}
/// # Safety
/// `profile` is null or a readable NUL-terminated string. The result is static.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn power_profiles_service_profile_to_icon(
    profile: *const c_char,
) -> *const c_char {
    catch_unwind(AssertUnwindSafe(|| {
        let profile = if profile.is_null() {
            None
        } else {
            (unsafe { CStr::from_ptr(profile) }).to_str().ok()
        };
        match profile {
            Some("performance") => c"power-profile-performance-symbolic".as_ptr(),
            Some("power-saver") => c"power-profile-power-saver-symbolic".as_ptr(),
            _ => c"power-profile-balanced-symbolic".as_ptr(),
        }
    }))
    .unwrap_or(c"power-profile-balanced-symbolic".as_ptr())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adapter_disconnects_before_releasing_service() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                for _ in 0..20 {
                    let service = PowerProfilesService::new();
                    let weak_service = service.downgrade();
                    let adapter = ProfileAdapter::new(service);
                    let weak_adapter = adapter.downgrade();
                    let array = ProfileArray::new(&["balanced".into()]);
                    adapter.emit_by_name::<()>("profiles-changed", &[&array]);
                    drop(adapter);
                    assert!(weak_adapter.upgrade().is_none());
                    assert!(weak_service.upgrade().is_none());
                }
                context.block_on(glib::timeout_future(std::time::Duration::from_millis(20)));
            })
            .unwrap();
    }
    #[test]
    fn array_snapshot_retains_strings_and_null_getters_are_safe() {
        let original = ProfileArray::new(&["balanced".into(), "performance".into()]);
        let retained = original.clone();
        drop(original);
        unsafe {
            assert_eq!((*retained.pointer()).len, 2);
            let entries =
                std::slice::from_raw_parts((*retained.pointer()).data.cast::<*mut c_char>(), 2);
            assert_eq!(CStr::from_ptr(entries[1]).to_str().unwrap(), "performance");
            assert_eq!(power_profiles_service_get_enabled(std::ptr::null_mut()), 0);
            assert!(power_profiles_service_get_profiles(std::ptr::null_mut()).is_null());
        }
    }
}
