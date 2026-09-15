//! Temporary C lifecycle signals around the composed Rust quick-settings view.
use glib::{prelude::*, subclass::prelude::*, translate::IntoGlib};
use std::{
    cell::RefCell,
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
    sync::OnceLock,
};
use way_shell::{
    services::settings,
    ui::quick_settings::{
        QuickSettingsEvent, QuickSettingsWindow,
        controller::{QuickSettings as View, QuickSettingsServices},
        controls::SystemServices,
    },
};

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct QuickSettings {
        pub view: RefCell<Option<Rc<View>>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for QuickSettings {
        const NAME: &'static str = "QuickSettings";
        type Type = super::QuickSettings;
    }
    impl ObjectImpl for QuickSettings {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                [
                    "quick-settings-will-show",
                    "quick-settings-visible",
                    "quick-settings-hidden",
                ]
                .into_iter()
                .map(|name| glib::subclass::Signal::builder(name).build())
                .collect()
            })
        }
        fn dispose(&self) {
            self.obj().stop();
        }
    }
}
glib::wrapper! { pub struct QuickSettings(ObjectSubclass<imp::QuickSettings>); }
impl QuickSettings {
    fn new() -> Result<Self, String> {
        let services = QuickSettingsServices {
            system: SystemServices {
                theme: crate::theme::service().ok_or("Theme service has not started")?,
                logind: crate::logind::service().ok_or("Session service has not started")?,
                profiles: crate::power_profiles::service()
                    .ok_or("Power profiles have not started")?,
                brightness: crate::brightness::service()
                    .ok_or("Brightness service has not started")?,
                wayland: crate::wayland::service().ok_or("Wayland service has not started")?,
            },
            audio: crate::audio::service().ok_or("Audio service has not started")?,
            network: crate::network::service().ok_or("Network service has not started")?,
            power: crate::power::service().ok_or("Power service has not started")?,
            notifications: crate::notifications::service()
                .ok_or("Notifications have not started")?,
        };
        let view = View::new(
            services,
            settings::open("org.ldelossa.way-shell.system").map_err(|error| error.to_string())?,
            |confirmation| {
                crate::dialog::present(
                    confirmation.action.title(),
                    confirmation.action.body(),
                    confirmation.respond,
                );
            },
        )?;
        let object: Self = glib::Object::new();
        let weak = object.downgrade();
        view.window().on_event(move |event| {
            if let Some(object) = weak.upgrade() {
                object.emit_by_name::<()>(signal_name(event), &[]);
            }
        });
        object.imp().view.replace(Some(view));
        Ok(object)
    }
    fn window(&self) -> Option<Rc<QuickSettingsWindow>> {
        self.imp()
            .view
            .borrow()
            .as_ref()
            .map(|view| view.window().clone())
    }
    fn stop(&self) {
        let view = self.imp().view.borrow_mut().take();
        if let Some(view) = view {
            view.close();
        }
    }
}
fn signal_name(event: QuickSettingsEvent) -> &'static str {
    match event {
        QuickSettingsEvent::WillShow => "quick-settings-will-show",
        QuickSettingsEvent::Visible => "quick-settings-visible",
        QuickSettingsEvent::Hidden => "quick-settings-hidden",
    }
}
thread_local! {
    static GLOBAL: RefCell<Option<QuickSettings>> = const { RefCell::new(None) };
}
pub fn shutdown() {
    let object = GLOBAL.with(|global| global.borrow_mut().take());
    if let Some(object) = object {
        object.stop();
    }
}
fn current(handle: *const c_void) -> Option<QuickSettings> {
    GLOBAL.with(|global| {
        global
            .borrow()
            .as_ref()
            .filter(|object| object.as_ptr().cast::<c_void>() == handle.cast_mut())
            .cloned()
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn quick_settings_get_type() -> glib::ffi::GType {
    QuickSettings::static_type().into_glib()
}
#[unsafe(no_mangle)]
pub extern "C" fn quick_settings_activate(_app: *mut c_void, _data: *mut c_void) {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
        shutdown();
        let object = QuickSettings::new()?;
        GLOBAL.with(|global| global.replace(Some(object)));
        Ok(())
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => glib::g_warning!("way-shell", "Could not start quick settings: {error}"),
        Err(_) => glib::g_warning!("way-shell", "Could not start quick settings"),
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn quick_settings_get_global() -> *mut c_void {
    GLOBAL.with(|global| {
        global
            .borrow()
            .as_ref()
            .map_or(std::ptr::null_mut(), |object| object.as_ptr().cast())
    })
}
macro_rules! command {
    ($name:ident, $method:ident) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(handle: *const c_void) {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                if let Some(window) = current(handle).and_then(|object| object.window()) {
                    window.$method();
                }
            }));
        }
    };
}
command!(quick_settings_set_visible, show);
command!(quick_settings_set_hidden, hide);
command!(quick_settings_toggle, toggle);
command!(quick_settings_shrink, shrink);
#[unsafe(no_mangle)]
pub extern "C" fn quick_settings_set_focused(handle: *const c_void, focused: i32) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(window) = current(handle).and_then(|object| object.window()) {
            window.set_focused(focused != 0);
        }
    }));
}
#[unsafe(no_mangle)]
pub extern "C" fn quick_settings_is_visible(handle: *const c_void) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        current(handle)
            .and_then(|object| object.window())
            .is_some_and(|window| window.is_visible())
            .into()
    }))
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn c_lifecycle_signals_and_stale_handles_keep_the_contract() {
        let object: QuickSettings = glib::Object::new();
        assert_eq!(QuickSettings::static_type().name(), "QuickSettings");
        let seen = Rc::new(RefCell::new(Vec::new()));
        for event in [
            QuickSettingsEvent::WillShow,
            QuickSettingsEvent::Visible,
            QuickSettingsEvent::Hidden,
        ] {
            let seen = seen.clone();
            object.connect_local(signal_name(event), false, move |values| {
                assert_eq!(values.len(), 1);
                seen.borrow_mut().push(event);
                None
            });
            object.emit_by_name::<()>(signal_name(event), &[]);
        }
        assert_eq!(seen.borrow().len(), 3);
        GLOBAL.with(|global| global.replace(Some(object.clone())));
        let handle = quick_settings_get_global();
        assert!(!handle.is_null());
        shutdown();
        assert!(quick_settings_get_global().is_null());
        quick_settings_toggle(handle);
        quick_settings_set_visible(handle);
        quick_settings_set_hidden(handle);
        quick_settings_shrink(handle);
        quick_settings_set_focused(handle, 1);
        assert_eq!(quick_settings_is_visible(handle), 0);
        assert!(object.window().is_none());
    }
}
