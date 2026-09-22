//! Persistent Bluetooth rows, content-sized layout and recoverable operation errors.
use super::{QuickSettingsWindow, grid::GridButton, menu::Menu};
use crate::services::bluetooth::{BluetoothDevice, BluetoothService, settings};
use gtk::{accessible::Property, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::{Rc, Weak},
};
struct Row {
    button: gtk::Button,
    icon: gtk::Image,
    name: gtk::Label,
    action: gtk::Label,
    spinner: gtk::Spinner,
}
/// Paired devices shown before the device list starts scrolling.
const MAX_VISIBLE_DEVICES: usize = 5;
pub struct BluetoothControls {
    service: BluetoothService,
    settings: gio::Settings,
    window: Weak<QuickSettingsWindow>,
    tile: Rc<GridButton>,
    menu: Menu,
    placeholder: gtk::Label,
    error: gtk::Label,
    rows: RefCell<HashMap<String, Row>>,
    handlers: RefCell<Vec<glib::SignalHandlerId>>,
    observers: RefCell<Vec<Rc<dyn Fn()>>>,
    available: Cell<bool>,
    last_powered: Cell<bool>,
    last_ready: Cell<bool>,
    stopped: Cell<bool>,
}
impl BluetoothControls {
    pub fn new(
        service: BluetoothService,
        settings: gio::Settings,
        window: &Rc<QuickSettingsWindow>,
    ) -> Rc<Self> {
        let menu = Menu::new("Bluetooth", "bluetooth-active-symbolic");
        let placeholder = gtk::Label::builder()
            .wrap(true)
            .max_width_chars(20)
            .justify(gtk::Justification::Center)
            .css_classes(["bluetooth-placeholder"])
            .build();
        menu.options().append(&placeholder);
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.add_css_class("bluetooth-separator");
        menu.options().append(&separator);
        let settings_button = gtk::Button::with_label("Bluetooth Settings");
        settings_button.add_css_class("bluetooth-settings");
        settings_button
            .child()
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap()
            .set_xalign(0.0);
        menu.options().append(&settings_button);
        let error = gtk::Label::builder()
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(24)
            .build();
        let dismiss = gtk::Button::builder()
            .child(&error)
            .css_classes(["failure-banner"])
            .build();
        dismiss.update_property(&[Property::Label("Dismiss Bluetooth error")]);
        menu.banner().set_child(Some(&dismiss));
        let tile = GridButton::new(
            "Bluetooth",
            None,
            "bluetooth-disabled-symbolic",
            Some(menu.widget().upcast_ref()),
        );
        tile.reveal_button()
            .unwrap()
            .update_property(&[Property::Label("Open Bluetooth menu")]);
        let this = Rc::new(Self {
            service,
            settings,
            window: Rc::downgrade(window),
            tile,
            menu,
            placeholder,
            error,
            rows: RefCell::new(HashMap::new()),
            handlers: RefCell::new(Vec::new()),
            observers: RefCell::new(Vec::new()),
            available: Cell::new(false),
            last_powered: Cell::new(false),
            last_ready: Cell::new(false),
            stopped: Cell::new(false),
        });
        for signal in ["changed", "operation-error", "operation-succeeded"] {
            let weak = Rc::downgrade(&this);
            let handler = this.service.connect_local(signal, false, move |values| {
                if let Some(this) = weak.upgrade().filter(|s| !s.stopped.get()) {
                    match signal {
                        "changed" => this.refresh(!this.tile.revealer().reveals_child()),
                        "operation-error" => this.show_error(&values[1].get::<String>().unwrap()),
                        _ => this.clear_error(),
                    }
                }
                None
            });
            this.handlers.borrow_mut().push(handler);
        }
        let weak = Rc::downgrade(&this);
        this.tile.toggle().connect_clicked(move |_| {
            if let Some(this) = weak.upgrade().filter(|s| !s.stopped.get()) {
                this.clear_error();
                this.service
                    .set_powered(!this.service.state().target_powered);
            }
        });
        let weak = Rc::downgrade(&this);
        this.tile
            .revealer()
            .connect_reveal_child_notify(move |revealer| {
                if revealer.reveals_child()
                    && let Some(this) = weak.upgrade().filter(|s| !s.stopped.get())
                {
                    this.refresh(true);
                }
            });
        let weak = Rc::downgrade(&this);
        dismiss.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.clear_error();
            }
        });
        let weak = Rc::downgrade(&this);
        settings_button.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade().filter(|s| !s.stopped.get()) {
                match settings::launch(&this.settings.string("bluetooth-settings-command")) {
                    Ok(()) => {
                        if let Some(window) = this.window.upgrade() {
                            window.hide();
                        }
                    }
                    Err(error) => this.show_error(error.message()),
                }
            }
        });
        this.refresh(true);
        this
    }
    pub fn button(&self) -> Option<Rc<GridButton>> {
        self.available.get().then(|| self.tile.clone())
    }
    pub fn tile(&self) -> &Rc<GridButton> {
        &self.tile
    }
    pub fn menu(&self) -> &Menu {
        &self.menu
    }
    pub fn on_changed(&self, callback: impl Fn() + 'static) {
        self.observers.borrow_mut().push(Rc::new(callback));
    }
    pub fn stop(&self) {
        if self.stopped.replace(true) {
            return;
        }
        for handler in self.handlers.take() {
            self.service.disconnect(handler);
        }
        self.observers.borrow_mut().clear();
        self.tile.set_sensitive(false);
        for row in self.rows.borrow().values() {
            row.spinner.stop();
            row.button.set_sensitive(false);
        }
    }
    fn row(self: &Rc<Self>, device: &BluetoothDevice) -> Row {
        let button = gtk::Button::new();
        button.add_css_class("quick-settings-menu-option-bluetooth");
        let contents = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let icon = gtk::Image::new();
        icon.set_pixel_size(20);
        let name = gtk::Label::builder()
            .hexpand(true)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(24)
            .build();
        let action = gtk::Label::new(None);
        action.add_css_class("bluetooth-action");
        let spinner = gtk::Spinner::new();
        spinner.set_size_request(16, 16);
        contents.append(&icon);
        contents.append(&name);
        contents.append(&action);
        contents.append(&spinner);
        button.set_child(Some(&contents));
        self.menu.list().append(&button);
        let weak = Rc::downgrade(self);
        let path = device.path.clone();
        button.connect_clicked(move |button| {
            if let Some(this) = weak.upgrade().filter(|s| !s.stopped.get()) {
                let current = this
                    .rows
                    .borrow()
                    .get(&path)
                    .is_some_and(|row| row.button == *button);
                if current {
                    this.service.toggle_device(&path);
                }
            }
        });
        Row {
            button,
            icon,
            name,
            action,
            spinner,
        }
    }
    fn refresh(self: &Rc<Self>, reorder: bool) {
        if self.stopped.get() {
            return;
        }
        let state = self.service.state();
        if self.last_powered.replace(state.powered) != state.powered
            || (state.ready && !self.last_ready.get())
        {
            self.clear_error();
        }
        self.last_ready.set(state.ready);
        let present: HashSet<_> = state.devices.iter().map(|d| d.path.as_str()).collect();
        let removed: Vec<_> = self
            .rows
            .borrow()
            .keys()
            .filter(|p| !present.contains(p.as_str()))
            .cloned()
            .collect();
        for path in removed {
            if let Some(row) = self.rows.borrow_mut().remove(&path) {
                row.spinner.stop();
                row.button.set_sensitive(false);
                self.menu.list().remove(&row.button);
            }
        }
        let mut previous: Option<gtk::Button> = None;
        for device in &state.devices {
            if !self.rows.borrow().contains_key(&device.path) {
                let row = self.row(device);
                self.rows.borrow_mut().insert(device.path.clone(), row);
            }
            let rows = self.rows.borrow();
            let row = &rows[&device.path];
            let symbolic = if device.icon.ends_with("-symbolic") {
                device.icon.clone()
            } else {
                format!("{}-symbolic", device.icon)
            };
            row.icon.set_from_gicon(&gio::ThemedIcon::from_names(&[
                &symbolic,
                &device.icon,
                "bluetooth-symbolic",
            ]));
            row.name.set_label(&device.alias);
            row.action.set_label(if device.connected {
                "Disconnect"
            } else {
                "Connect"
            });
            row.action.set_visible(!device.busy);
            row.spinner.set_visible(device.busy);
            row.spinner.set_spinning(device.busy);
            row.button.set_sensitive(!device.busy && !state.busy);
            row.button.update_property(&[Property::Label(&format!(
                "{} {}",
                if device.connected {
                    "Disconnect"
                } else {
                    "Connect to"
                },
                device.alias
            ))]);
            if reorder {
                self.menu
                    .list()
                    .reorder_child_after(&row.button, previous.as_ref());
            }
            previous = Some(row.button.clone());
        }
        self.placeholder.set_visible(state.devices.is_empty());
        self.menu.update_viewport(MAX_VISIBLE_DEVICES);
        self.placeholder.set_label(if !state.ready {
            "Bluetooth service unavailable"
        } else if state.hardware_blocked {
            "Bluetooth is disabled by a hardware switch"
        } else if state.powered {
            "No available or connected devices"
        } else {
            "Turn on Bluetooth to connect to devices"
        });
        let connected: Vec<_> = state.devices.iter().filter(|d| d.connected).collect();
        let subtitle = match connected.len() {
            0 => None,
            1 => Some(connected[0].alias.clone()),
            n => Some(format!("{n} Connected")),
        };
        self.tile.set_optional_subtitle(subtitle.as_deref());
        self.tile.set_toggled(state.powered);
        let icon = if state.busy {
            "bluetooth-acquiring-symbolic"
        } else if state.powered {
            "bluetooth-active-symbolic"
        } else {
            "bluetooth-disabled-symbolic"
        };
        self.tile.set_icon(icon);
        self.menu.set_icon(icon);
        self.tile
            .toggle()
            .set_tooltip_text(state.busy.then_some(if state.target_powered {
                "Turning Bluetooth on…"
            } else {
                "Turning Bluetooth off…"
            }));
        self.tile
            .toggle()
            .set_sensitive(state.ready && !state.hardware_blocked);
        self.tile
            .toggle()
            .update_property(&[Property::Label(if state.target_powered {
                "Turn Bluetooth off"
            } else {
                "Turn Bluetooth on"
            })]);
        if self.available.replace(state.available) != state.available {
            if !state.available {
                self.tile.revealer().set_reveal_child(false);
            }
            let observers = self.observers.borrow().clone();
            for callback in observers {
                callback();
            }
        }
        if let Some(window) = self.window.upgrade() {
            window.shrink();
        }
    }
    fn clear_error(&self) {
        self.menu.banner().set_reveal_child(false);
    }
    fn show_error(&self, message: &str) {
        self.error.set_label(message);
        self.menu.banner().set_reveal_child(true);
        if !self.tile.revealer().reveals_child()
            && self.window.upgrade().is_some_and(|w| w.is_visible())
        {
            self.tile.toggle_menu();
        }
    }
}
impl Drop for BluetoothControls {
    fn drop(&mut self) {
        self.stop();
    }
}
