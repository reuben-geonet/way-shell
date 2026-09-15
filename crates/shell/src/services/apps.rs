//! Owned application discovery with GIO desktop-entry launching.
use gio::prelude::*;
use glib::subclass::prelude::*;
use std::{
    cell::{Cell, RefCell},
    ffi::CString,
    sync::OnceLock,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub id: String,
    pub name: String,
    /// GIO's portable icon string, reconstructed only by presentation code.
    pub icon: Option<String>,
}

/// Match exactly like the former activities search, including token prefixes,
/// Unicode case folding and ASCII transliteration. No native pointer escapes.
pub fn matches(query: &str, name: &str) -> bool {
    let (Ok(query), Ok(name)) = (CString::new(query), CString::new(name)) else {
        return false;
    };
    // Both owned C strings are valid throughout the non-owning GLib call.
    unsafe { glib::ffi::g_str_match_string(query.as_ptr(), name.as_ptr(), 1) != 0 }
}
fn enumerate() -> Vec<Application> {
    gio::AppInfo::all()
        .into_iter()
        .filter_map(|info| {
            let desktop = info.downcast::<gio::DesktopAppInfo>().ok()?;
            if desktop.is_nodisplay() || desktop.string("Type").as_deref() != Some("Application") {
                return None;
            }
            Some(Application {
                id: desktop.id()?.into(),
                name: desktop.display_name().into(),
                icon: desktop
                    .icon()
                    .and_then(|icon| icon.to_string())
                    .map(Into::into),
            })
        })
        .collect()
}
mod imp {
    use super::*;
    #[derive(Default)]
    pub struct AppCatalog {
        pub entries: RefCell<Vec<Application>>,
        pub monitor: RefCell<Option<gio::AppInfoMonitor>>,
        pub handler: RefCell<Option<glib::SignalHandlerId>>,
        pub task: RefCell<Option<glib::JoinHandle<()>>>,
        pub running: Cell<bool>,
        pub dirty: Cell<bool>,
        pub generation: Cell<u64>,
        pub launches: RefCell<Vec<gio::Cancellable>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for AppCatalog {
        const NAME: &'static str = "WayShellAppCatalog";
        type Type = super::AppCatalog;
    }
    impl ObjectImpl for AppCatalog {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("changed").build()])
        }
        fn dispose(&self) {
            self.obj().stop();
        }
    }
}
glib::wrapper! {pub struct AppCatalog(ObjectSubclass<imp::AppCatalog>);}
impl Default for AppCatalog {
    fn default() -> Self {
        Self::new()
    }
}
impl AppCatalog {
    pub fn new() -> Self {
        let catalog: Self = glib::Object::new();
        catalog.imp().running.set(true);
        catalog.imp().dirty.set(true);
        let monitor = gio::AppInfoMonitor::get();
        let weak = catalog.downgrade();
        let handler = monitor.connect_changed(move |_| {
            if let Some(catalog) = weak.upgrade() {
                catalog.imp().dirty.set(true);
            }
        });
        catalog.imp().monitor.replace(Some(monitor));
        catalog.imp().handler.replace(Some(handler));
        catalog.refresh();
        catalog
    }
    pub fn entries(&self) -> Vec<Application> {
        self.imp().entries.borrow().clone()
    }
    pub fn is_dirty(&self) -> bool {
        self.imp().dirty.get()
    }
    pub fn is_loading(&self) -> bool {
        self.imp().task.borrow().is_some()
    }
    /// Refresh when activities next opens. Enumeration does not block GTK and
    /// detached worker results cannot revive a stopped catalog.
    pub fn refresh(&self) {
        if !self.imp().running.get() || self.is_loading() {
            return;
        }
        self.imp().dirty.set(false);
        let generation = self.imp().generation.get();
        let weak = self.downgrade();
        let task = glib::MainContext::ref_thread_default().spawn_local(async move {
            let result = gio::spawn_blocking(enumerate).await;
            let Some(catalog) = weak.upgrade().filter(|catalog| {
                catalog.imp().running.get() && catalog.imp().generation.get() == generation
            }) else {
                return;
            };
            catalog.imp().task.borrow_mut().take();
            match result {
                Ok(entries) => {
                    catalog.imp().entries.replace(entries);
                    catalog.emit_by_name::<()>("changed", &[]);
                }
                Err(_) => {
                    catalog.imp().dirty.set(true);
                    glib::g_message!(
                        "way-shell",
                        "Application discovery failed; retrying when activities opens"
                    );
                }
            }
        });
        self.imp().task.replace(Some(task));
    }
    pub fn launch(&self, id: &str, callback: impl FnOnce(Result<(), glib::Error>) + 'static) {
        if !self.imp().running.get()
            || !self
                .imp()
                .entries
                .borrow()
                .iter()
                .any(|entry| entry.id == id)
        {
            callback(Err(glib::Error::new(
                gio::IOErrorEnum::NotFound,
                "Application is no longer in the current inventory",
            )));
            return;
        }
        let Some(info) = gio::DesktopAppInfo::new(id) else {
            callback(Err(glib::Error::new(
                gio::IOErrorEnum::NotFound,
                "Desktop entry is no longer available",
            )));
            return;
        };
        let cancel = gio::Cancellable::new();
        self.imp().launches.borrow_mut().push(cancel.clone());
        let weak = self.downgrade();
        let check = cancel.clone();
        info.launch_uris_async(
            &[],
            gio::AppLaunchContext::NONE,
            Some(&cancel),
            move |result| {
                if let Some(catalog) = weak.upgrade() {
                    catalog
                        .imp()
                        .launches
                        .borrow_mut()
                        .retain(|pending| pending != &check);
                }
                callback(result);
            },
        );
    }
    pub fn stop(&self) {
        self.imp().running.set(false);
        self.imp()
            .generation
            .set(self.imp().generation.get().wrapping_add(1));
        let monitor = self.imp().monitor.borrow_mut().take();
        let handler = self.imp().handler.borrow_mut().take();
        if let (Some(monitor), Some(handler)) = (monitor, handler) {
            monitor.disconnect(handler);
        }
        if let Some(task) = self.imp().task.borrow_mut().take() {
            task.abort();
        }
        for pending in self.imp().launches.take() {
            pending.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matching_keeps_glib_prefix_case_and_transliteration_rules() {
        assert!(matches("cal", "Calculator"));
        assert!(matches("CAFE", "Café Editor"));
        assert!(matches("edi caf", "Café Editor"));
        assert!(!matches("alc", "Calculator"));
        assert!(!matches("bad\0query", "Calculator"));
    }
}
