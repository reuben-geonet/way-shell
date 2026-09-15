//! Temporary GObject signals and C entry points for the Rust activities view.
use glib::{prelude::*, subclass::prelude::*, translate::IntoGlib};
use std::{
    cell::RefCell,
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::OnceLock,
};
use way_shell::{
    services::apps::AppCatalog,
    ui::activities::{Activities as View, ActivitiesEvent},
};

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct Activities {
        pub view: RefCell<Option<View>>,
        pub catalog: RefCell<Option<AppCatalog>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for Activities {
        const NAME: &'static str = "Activities";
        type Type = super::Activities;
    }
    impl ObjectImpl for Activities {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                [
                    "activities-will-show",
                    "activities-visible",
                    "activities-will-hide",
                    "activities-hidden",
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
glib::wrapper! { pub struct Activities(ObjectSubclass<imp::Activities>); }
impl Activities {
    fn new() -> Result<Self, String> {
        let catalog = AppCatalog::new();
        let view = View::new(catalog.clone())?;
        let object: Self = glib::Object::new();
        let weak = object.downgrade();
        view.on_event(move |event| {
            if let Some(object) = weak.upgrade() {
                object.emit_by_name::<()>(signal_name(event), &[]);
            }
        });
        object.imp().view.replace(Some(view));
        object.imp().catalog.replace(Some(catalog));
        Ok(object)
    }
    fn view(&self) -> Option<View> {
        self.imp().view.borrow().clone()
    }
    fn stop(&self) {
        let view = self.imp().view.borrow_mut().take();
        let catalog = self.imp().catalog.borrow_mut().take();
        if let Some(view) = view {
            view.close();
        }
        if let Some(catalog) = catalog {
            catalog.stop();
        }
    }
}
fn signal_name(event: ActivitiesEvent) -> &'static str {
    match event {
        ActivitiesEvent::WillShow => "activities-will-show",
        ActivitiesEvent::Visible => "activities-visible",
        ActivitiesEvent::WillHide => "activities-will-hide",
        ActivitiesEvent::Hidden => "activities-hidden",
    }
}
thread_local! {
    static GLOBAL: RefCell<Option<Activities>> = const { RefCell::new(None) };
}
pub fn shutdown() {
    let object = GLOBAL.with(|global| global.borrow_mut().take());
    if let Some(object) = object {
        object.stop();
    }
}
fn current(handle: *const c_void) -> Option<Activities> {
    GLOBAL.with(|global| {
        global
            .borrow()
            .as_ref()
            .filter(|object| object.as_ptr().cast::<c_void>() == handle.cast_mut())
            .cloned()
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn activities_get_type() -> glib::ffi::GType {
    Activities::static_type().into_glib()
}
#[unsafe(no_mangle)]
pub extern "C" fn activities_activate(_app: *mut c_void, _data: *mut c_void) {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
        shutdown();
        let object = Activities::new()?;
        GLOBAL.with(|global| global.replace(Some(object)));
        Ok(())
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => glib::g_warning!("way-shell", "Could not start activities: {error}"),
        Err(_) => glib::g_warning!("way-shell", "Could not start activities"),
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn activities_get_global() -> *mut c_void {
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
                if let Some(view) = current(handle).and_then(|object| object.view()) {
                    view.$method();
                }
            }));
        }
    };
}
command!(activities_show, show);
command!(activities_hide, hide);
command!(activities_toggle, toggle);

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    #[test]
    fn c_signal_contract_and_retained_global_are_inert_after_shutdown() {
        let object: Activities = glib::Object::new();
        assert_eq!(Activities::static_type().name(), "Activities");
        assert_eq!(activities_get_type(), Activities::static_type().into_glib());
        let seen = Rc::new(RefCell::new(Vec::new()));
        for event in [
            ActivitiesEvent::WillShow,
            ActivitiesEvent::Visible,
            ActivitiesEvent::WillHide,
            ActivitiesEvent::Hidden,
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
        let handle = activities_get_global();
        assert!(!handle.is_null());
        assert_eq!(current(handle).as_ref(), Some(&object));
        shutdown();
        assert!(activities_get_global().is_null());
        assert!(current(handle).is_none());
        activities_show(handle);
        activities_hide(handle);
        activities_toggle(handle);
        activities_show(std::ptr::null());
        assert_eq!(seen.borrow().len(), 4);
        assert!(object.view().is_none());
    }
}
