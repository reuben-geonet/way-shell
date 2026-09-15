//! Temporary C lifecycle signals around the Rust message tray.
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
    ui::message_tray::{
        MessageTray as View, MessageTrayServices,
        window::{MessageTrayEvent, MessageTrayWindow},
    },
};

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct MessageTray {
        pub view: RefCell<Option<Rc<View>>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for MessageTray {
        const NAME: &'static str = "MessageTray";
        type Type = super::MessageTray;
    }
    impl ObjectImpl for MessageTray {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                [
                    "message-tray-will-show",
                    "message-tray-visible",
                    "message-tray-will-hide",
                    "message-tray-hidden",
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
glib::wrapper! { pub struct MessageTray(ObjectSubclass<imp::MessageTray>); }
impl MessageTray {
    fn new() -> Result<Self, String> {
        let view = View::new(
            MessageTrayServices {
                clock: crate::clock::service().ok_or("Clock service has not started")?,
                notifications: crate::notifications::service()
                    .ok_or("Notifications have not started")?,
                media: crate::media::service().ok_or("Media service has not started")?,
            },
            settings::open("org.ldelossa.way-shell.notifications")
                .map_err(|error| error.to_string())?,
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
    fn window(&self) -> Option<Rc<MessageTrayWindow>> {
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
fn signal_name(event: MessageTrayEvent) -> &'static str {
    match event {
        MessageTrayEvent::WillShow => "message-tray-will-show",
        MessageTrayEvent::Visible => "message-tray-visible",
        MessageTrayEvent::WillHide => "message-tray-will-hide",
        MessageTrayEvent::Hidden => "message-tray-hidden",
    }
}
thread_local! { static GLOBAL: RefCell<Option<MessageTray>> = const { RefCell::new(None) }; }
pub fn shutdown() {
    let object = GLOBAL.with(|global| global.borrow_mut().take());
    if let Some(object) = object {
        object.stop();
    }
}
fn current(handle: *const c_void) -> Option<MessageTray> {
    GLOBAL.with(|global| {
        global
            .borrow()
            .as_ref()
            .filter(|object| object.as_ptr().cast::<c_void>() == handle.cast_mut())
            .cloned()
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn message_tray_get_type() -> glib::ffi::GType {
    MessageTray::static_type().into_glib()
}
#[unsafe(no_mangle)]
pub extern "C" fn message_tray_activate(_app: *mut c_void, _data: *mut c_void) {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
        shutdown();
        let object = MessageTray::new()?;
        GLOBAL.with(|global| global.replace(Some(object)));
        Ok(())
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => glib::g_warning!("way-shell", "Could not start message tray: {error}"),
        Err(_) => glib::g_warning!("way-shell", "Could not start message tray"),
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn message_tray_get_global() -> *mut c_void {
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
command!(message_tray_set_visible, show);
command!(message_tray_set_hidden, hide);
command!(message_tray_toggle, toggle);
command!(message_tray_shrink, shrink);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_signals_preserve_c_contract_and_stale_handles_are_inert() {
        let object: MessageTray = glib::Object::new();
        assert_eq!(MessageTray::static_type().name(), "MessageTray");
        let seen = Rc::new(RefCell::new(Vec::new()));
        for event in [
            MessageTrayEvent::WillShow,
            MessageTrayEvent::Visible,
            MessageTrayEvent::WillHide,
            MessageTrayEvent::Hidden,
        ] {
            let seen = seen.clone();
            object.connect_local(signal_name(event), false, move |values| {
                assert_eq!(values.len(), 1);
                seen.borrow_mut().push(event);
                None
            });
            object.emit_by_name::<()>(signal_name(event), &[]);
        }
        assert_eq!(seen.borrow().len(), 4);
        GLOBAL.with(|global| global.replace(Some(object.clone())));
        let handle = message_tray_get_global();
        assert!(!handle.is_null());
        shutdown();
        message_tray_set_visible(handle);
        message_tray_set_hidden(handle);
        message_tray_toggle(handle);
        message_tray_shrink(handle);
        assert!(message_tray_get_global().is_null());
        assert!(object.window().is_none());
    }
}
