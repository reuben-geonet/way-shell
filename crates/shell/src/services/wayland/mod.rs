//! One independent Wayland connection owns all protocol objects. Widgets use
//! owned output/window snapshots and GDK for shortcut inhibition.
mod connection;
pub mod model;
mod protocol;
use connection::Driver;
use gio::prelude::*;
use glib::subclass::prelude::*;
use gtk::prelude::*;
pub use model::{Output, Seat, Toplevel};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::OnceLock,
};

#[derive(Clone, Copy, Debug)]
pub enum ToplevelAction {
    Activate,
    Close,
    Maximize,
}

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct WaylandService {
        pub(super) driver: RefCell<Option<Rc<Driver>>>,
        pub settings: RefCell<Option<(gio::Settings, glib::SignalHandlerId)>>,
        pub ready: Cell<bool>,
        pub error: RefCell<Option<String>>,
        pub inhibitor: RefCell<Option<gtk::gdk::Toplevel>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for WaylandService {
        const NAME: &'static str = "WayShellWaylandService";
        type Type = super::WaylandService;
    }
    impl ObjectImpl for WaylandService {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("ready").build(),
                    glib::subclass::Signal::builder("output-changed")
                        .param_types([Output::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("output-removed")
                        .param_types([Output::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("toplevel-changed")
                        .param_types([Toplevel::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("toplevel-removed")
                        .param_types([Toplevel::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("capabilities-changed").build(),
                    glib::subclass::Signal::builder("gamma-control-enabled").build(),
                    glib::subclass::Signal::builder("gamma-control-disabled").build(),
                    glib::subclass::Signal::builder("failed")
                        .param_types([String::static_type()])
                        .build(),
                ]
            })
        }
        fn dispose(&self) {
            if let Some((settings, handler)) = self.settings.borrow_mut().take() {
                settings.disconnect(handler);
            }
            if let Some(inhibitor) = self.inhibitor.borrow_mut().take() {
                inhibitor.restore_system_shortcuts();
            }
            self.driver.borrow_mut().take();
        }
    }
}
glib::wrapper! { pub struct WaylandService(ObjectSubclass<imp::WaylandService>); }
impl WaylandService {
    pub fn connect() -> Result<Self, String> {
        Self::with_settings(
            super::settings::open("org.ldelossa.way-shell.window-manager")
                .map_err(|e| e.to_string())?,
        )
    }
    pub fn with_settings(settings: gio::Settings) -> Result<Self, String> {
        let service: Self = glib::Object::new();
        let weak = service.downgrade();
        let failed = weak.clone();
        let driver = Driver::connect(
            &settings.string("ignored-toplevels-app-ids"),
            &settings.string("ignored-toplevels-titles"),
            move |changes| {
                if let Some(service) = weak.upgrade() {
                    service.changes(changes);
                }
            },
            move |error| {
                if let Some(service) = failed.upgrade() {
                    service.imp().error.replace(Some(error.clone()));
                    service.imp().ready.set(false);
                    service.emit_by_name::<()>("failed", &[&error]);
                    service.emit_by_name::<()>("capabilities-changed", &[]);
                }
            },
        )?;
        service.imp().driver.replace(Some(driver));
        let weak = service.downgrade();
        let handler = settings.connect_changed(None, move |settings, key| {
            if matches!(
                key,
                "ignored-toplevels-app-ids" | "ignored-toplevels-titles"
            ) && let Some(service) = weak.upgrade()
            {
                let driver = service.imp().driver.borrow().clone();
                if let Some(driver) = driver {
                    driver.ignored(
                        &settings.string("ignored-toplevels-app-ids"),
                        &settings.string("ignored-toplevels-titles"),
                    );
                }
            }
        });
        service.imp().settings.replace(Some((settings, handler)));
        Ok(service)
    }
    fn changes(&self, changes: Vec<protocol::Change>) {
        for change in changes {
            match change {
                protocol::Change::Ready => {
                    self.imp().ready.set(true);
                    self.emit_by_name::<()>("ready", &[]);
                }
                protocol::Change::Output(value) => {
                    self.emit_by_name::<()>("output-changed", &[&value])
                }
                protocol::Change::OutputRemoved(value) => {
                    self.emit_by_name::<()>("output-removed", &[&value])
                }
                protocol::Change::Toplevel(value) => {
                    self.emit_by_name::<()>("toplevel-changed", &[&value])
                }
                protocol::Change::ToplevelRemoved(value) => {
                    self.emit_by_name::<()>("toplevel-removed", &[&value])
                }
                protocol::Change::Capabilities => {
                    self.emit_by_name::<()>("capabilities-changed", &[])
                }
                protocol::Change::Gamma(true) => {
                    self.emit_by_name::<()>("gamma-control-enabled", &[])
                }
                protocol::Change::Gamma(false) => {
                    self.emit_by_name::<()>("gamma-control-disabled", &[])
                }
                protocol::Change::Diagnostic(error) => glib::g_warning!("way-shell", "{error}"),
            }
        }
    }
    pub fn is_ready(&self) -> bool {
        self.imp().ready.get()
    }
    pub fn error(&self) -> Option<String> {
        self.imp().error.borrow().clone()
    }
    pub fn outputs(&self) -> Vec<Output> {
        self.imp()
            .driver
            .borrow()
            .as_ref()
            .map_or_else(Vec::new, |driver| driver.outputs())
    }
    pub fn seats(&self) -> Vec<Seat> {
        self.imp()
            .driver
            .borrow()
            .as_ref()
            .map_or_else(Vec::new, |driver| driver.seats())
    }
    pub fn toplevels(&self) -> Vec<Toplevel> {
        self.imp()
            .driver
            .borrow()
            .as_ref()
            .map_or_else(Vec::new, |driver| driver.toplevels())
    }
    pub fn has_foreign_toplevel(&self) -> bool {
        self.imp()
            .driver
            .borrow()
            .as_ref()
            .is_some_and(|driver| driver.has_foreign_toplevel())
    }
    pub fn has_gamma(&self) -> bool {
        self.imp()
            .driver
            .borrow()
            .as_ref()
            .is_some_and(|driver| driver.has_gamma())
    }
    pub fn gamma_available(&self) -> bool {
        self.imp()
            .driver
            .borrow()
            .as_ref()
            .is_some_and(|driver| driver.gamma_available())
    }
    pub fn has_shortcut_inhibition(&self) -> bool {
        self.imp()
            .driver
            .borrow()
            .as_ref()
            .is_some_and(|driver| driver.has_shortcut_inhibition())
    }
    pub fn gamma_enabled(&self) -> bool {
        self.imp()
            .driver
            .borrow()
            .as_ref()
            .is_some_and(|driver| driver.gamma_enabled())
    }
    pub fn action(&self, id: u64, action: ToplevelAction) -> Result<(), String> {
        let driver = self
            .imp()
            .driver
            .borrow()
            .clone()
            .ok_or("Wayland connection is closed")?;
        driver.action(id, action)
    }
    pub fn set_temperature(&self, temperature: u32) -> Result<(), String> {
        let driver = self
            .imp()
            .driver
            .borrow()
            .clone()
            .ok_or("Wayland connection is closed")?;
        driver.set_temperature(temperature)
    }
    pub fn disable_gamma(&self) {
        let driver = self.imp().driver.borrow().clone();
        if let Some(driver) = driver {
            driver.disable_gamma();
        }
    }
    pub fn inhibit_shortcuts(&self, widget: &impl IsA<gtk::Widget>) -> Result<(), String> {
        if self.imp().inhibitor.borrow().is_some() {
            return Err("Another window already inhibits compositor shortcuts".into());
        }
        if !self.has_shortcut_inhibition() {
            return Err("The compositor does not support shortcut inhibition".into());
        }
        let native = widget.native().ok_or("The widget has no native window")?;
        let surface = native.surface().ok_or("The widget has no surface")?;
        let toplevel = surface
            .downcast::<gtk::gdk::Toplevel>()
            .map_err(|_| "The widget is not a toplevel")?;
        toplevel.inhibit_system_shortcuts(None::<&gtk::gdk::Event>);
        self.imp().inhibitor.replace(Some(toplevel));
        Ok(())
    }
    pub fn restore_shortcuts(&self) -> bool {
        if let Some(toplevel) = self.imp().inhibitor.borrow_mut().take() {
            toplevel.restore_system_shortcuts();
            true
        } else {
            false
        }
    }
}
