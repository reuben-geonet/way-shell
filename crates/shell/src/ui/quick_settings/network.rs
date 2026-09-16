//! Network tiles own only service snapshots, GTK widgets and cancellable requests.
use super::{grid::GridButton, menu::Menu};
use crate::services::network::{
    AccessPoint, Device, NetworkService, NetworkState, SavedConnection,
};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct AccessPointKey(Vec<u8>, bool, u32, u32);
fn key(point: &AccessPoint) -> AccessPointKey {
    AccessPointKey(
        point.ssid.clone(),
        point.flags & 1 != 0,
        point.wpa_flags,
        point.rsn_flags,
    )
}
fn strongest(state: &NetworkState, device: &str) -> Vec<AccessPoint> {
    let mut groups = HashMap::<AccessPointKey, AccessPoint>::new();
    for point in state.access_points.iter().filter(|point| {
        point.device == device && !point.ssid.is_empty() && point.ssid.iter().any(|byte| *byte != 0)
    }) {
        groups
            .entry(key(point))
            .and_modify(|old| {
                let active = old.active || point.active;
                if point.strength > old.strength {
                    *old = point.clone();
                }
                old.active = active;
            })
            .or_insert_with(|| point.clone());
    }
    let mut points: Vec<_> = groups.into_values().collect();
    points.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(a.name.cmp(&b.name))
            .then(a.id.cmp(&b.id))
    });
    points
}
fn signal_icon(strength: u8) -> &'static str {
    match strength {
        0..=25 => "network-wireless-signal-weak-symbolic",
        26..=50 => "network-wireless-signal-ok-symbolic",
        51..=75 => "network-wireless-signal-good-symbolic",
        _ => "network-wireless-signal-excellent-symbolic",
    }
}
fn ethernet_state(state: u32) -> (&'static str, &'static str) {
    match state {
        40..=90 => ("no internet", "network-wired-acquiring-symbolic"),
        100 => ("connected", "network-wired-symbolic"),
        _ => ("no network", "network-wired-disconnected-symbolic"),
    }
}

/// A native operation may fail synchronously, or a GTK notification may stop its
/// owner before the native cancellable is returned. Both orders are supported.
#[derive(Default)]
struct Request {
    cancel: RefCell<Option<gio::Cancellable>>,
    finished: Cell<bool>,
    cancelled: Cell<bool>,
}
impl Request {
    fn attach(&self, cancel: gio::Cancellable) {
        if self.cancelled.get() {
            cancel.cancel();
        } else if !self.finished.get() {
            self.cancel.replace(Some(cancel));
        }
    }
    fn finish(&self) {
        self.finished.set(true);
        self.cancel.borrow_mut().take();
    }
    fn cancel(&self) {
        self.cancelled.set(true);
        let cancel = self.cancel.borrow_mut().take();
        if let Some(cancel) = cancel {
            cancel.cancel();
        }
    }
    fn active(&self) -> bool {
        !self.finished.get() && !self.cancelled.get()
    }
}
impl Drop for Request {
    fn drop(&mut self) {
        self.cancel();
    }
}
struct DeviceTile {
    kind: u32,
    tile: Rc<GridButton>,
    wifi: Option<Rc<WifiMenu>>,
}
impl DeviceTile {
    fn stop(&self) {
        if let Some(menu) = &self.wifi {
            menu.stop();
        }
        self.tile.set_sensitive(false);
    }
}
type Changed = Rc<dyn Fn()>;
pub struct NetworkControls {
    service: NetworkService,
    handler: RefCell<Option<glib::SignalHandlerId>>,
    devices: RefCell<HashMap<String, Rc<DeviceTile>>>,
    order: RefCell<Vec<String>>,
    vpn: Rc<VpnMenu>,
    vpn_tile: Rc<GridButton>,
    has_vpn: Cell<bool>,
    airplane: Rc<GridButton>,
    networking_request: RefCell<Option<Rc<Request>>>,
    wireless_request: RefCell<Option<Rc<Request>>>,
    updating: Cell<bool>,
    pending: Cell<bool>,
    stopped: Cell<bool>,
    observers: RefCell<Vec<Changed>>,
}
impl NetworkControls {
    pub fn new(service: NetworkService) -> Rc<Self> {
        let vpn = VpnMenu::new(service.clone());
        let vpn_tile = GridButton::new(
            "VPN",
            Some(""),
            "network-vpn-disconnected-symbolic",
            Some(vpn.menu.widget().upcast_ref()),
        );
        let airplane = GridButton::new("Airplane Mode", None, "airplane-mode-symbolic", None);
        let this = Rc::new(Self {
            service,
            handler: RefCell::new(None),
            devices: RefCell::new(HashMap::new()),
            order: RefCell::new(Vec::new()),
            vpn,
            vpn_tile,
            has_vpn: Cell::new(false),
            airplane,
            networking_request: RefCell::new(None),
            wireless_request: RefCell::new(None),
            updating: Cell::new(false),
            pending: Cell::new(false),
            stopped: Cell::new(false),
            observers: RefCell::new(Vec::new()),
        });
        let weak = Rc::downgrade(&this);
        let handler = this.service.connect_local("changed", false, move |_| {
            if let Some(this) = weak.upgrade() {
                this.refresh();
            }
            None
        });
        this.handler.replace(Some(handler));
        let weak = Rc::downgrade(&this);
        this.airplane.toggle().connect_clicked(move |_| {
            if let Some(this) = weak.upgrade().filter(|this| !this.stopped.get()) {
                this.set_networking();
            }
        });
        this.refresh();
        this
    }
    pub fn buttons(&self) -> Vec<Rc<GridButton>> {
        self.device_buttons()
            .into_iter()
            .chain(self.vpn_button())
            .collect()
    }
    pub fn device_buttons(&self) -> Vec<Rc<GridButton>> {
        let devices = self.devices.borrow();
        self.order
            .borrow()
            .iter()
            .filter_map(|id| devices.get(id).map(|device| device.tile.clone()))
            .collect()
    }
    pub fn vpn_button(&self) -> Option<Rc<GridButton>> {
        self.has_vpn.get().then(|| self.vpn_tile.clone())
    }
    pub fn airplane(&self) -> Rc<GridButton> {
        self.airplane.clone()
    }
    /// Capture composition owners weakly to avoid retaining the whole popup.
    pub fn on_changed(&self, callback: impl Fn() + 'static) {
        self.observers.borrow_mut().push(Rc::new(callback));
    }
    pub fn hide_passwords(&self) {
        let devices: Vec<_> = self.devices.borrow().values().cloned().collect();
        for device in devices {
            if let Some(menu) = &device.wifi {
                menu.hide_passwords();
            }
        }
    }
    pub fn stop(&self) {
        if self.stopped.replace(true) {
            return;
        }
        if let Some(handler) = self.handler.borrow_mut().take() {
            self.service.disconnect(handler);
        }
        for request in [
            self.networking_request.borrow_mut().take(),
            self.wireless_request.borrow_mut().take(),
        ]
        .into_iter()
        .flatten()
        {
            request.cancel();
        }
        let devices: Vec<_> = self.devices.borrow().values().cloned().collect();
        for device in devices {
            device.stop();
        }
        self.vpn.stop();
        self.vpn_tile.set_sensitive(false);
        self.airplane.set_sensitive(false);
    }
    fn refresh(self: &Rc<Self>) {
        if self.stopped.get() {
            return;
        }
        self.pending.set(true);
        if self.updating.replace(true) {
            return;
        }
        while self.pending.replace(false) && !self.stopped.get() {
            let state = self.service.state();
            let order: Vec<_> = state
                .devices
                .iter()
                .filter(|device| matches!(device.kind, 1 | 2))
                .map(|device| device.id.clone())
                .collect();
            let mut old = self.devices.take();
            let mut next = HashMap::new();
            for device in &state.devices {
                if self.stopped.get() {
                    break;
                }
                if !matches!(device.kind, 1 | 2) {
                    continue;
                }
                let retained = old
                    .remove(&device.id)
                    .filter(|tile| tile.kind == device.kind);
                let tile = retained.unwrap_or_else(|| self.make_device(device));
                next.insert(device.id.clone(), tile);
            }
            let structure = *self.order.borrow() != order
                || self.has_vpn.get() != state.connections.iter().any(SavedConnection::is_vpn);
            self.devices.replace(next);
            for (_, removed) in old {
                removed.stop();
            }
            self.order.replace(order);
            self.has_vpn
                .set(state.connections.iter().any(SavedConnection::is_vpn));
            if self.stopped.get() {
                let tiles = self.devices.borrow().values().cloned().collect::<Vec<_>>();
                for tile in tiles {
                    tile.stop();
                }
                break;
            }
            let radio_busy = self
                .wireless_request
                .borrow()
                .as_ref()
                .is_some_and(|request| request.active());
            for device in &state.devices {
                let tile = self.devices.borrow().get(&device.id).cloned();
                let Some(tile) = tile else {
                    continue;
                };
                if let Some(menu) = &tile.wifi {
                    let active = state
                        .access_points
                        .iter()
                        .find(|point| point.device == device.id && point.active);
                    tile.tile
                        .set_subtitle(active.map_or("", |point| point.name.as_str()));
                    tile.tile.set_icon(if (40..=90).contains(&device.state) {
                        "network-wireless-acquiring-symbolic"
                    } else if !state.wireless_enabled || device.state != 100 {
                        "network-wireless-offline-symbolic"
                    } else {
                        active.map_or("network-wireless-signal-none-symbolic", |point| {
                            signal_icon(point.strength)
                        })
                    });
                    tile.tile
                        .set_toggled(state.wireless_enabled && device.state == 100);
                    tile.tile.set_sensitive(
                        state.available
                            && state.networking_enabled
                            && state.wireless_hardware_enabled
                            && !radio_busy,
                    );
                    menu.refresh(&state, device);
                } else {
                    let (subtitle, icon) = ethernet_state(device.state);
                    tile.tile.set_subtitle(subtitle);
                    tile.tile.set_icon(icon);
                    tile.tile.set_sensitive(state.available);
                }
                if self.stopped.get() {
                    tile.stop();
                    break;
                }
            }
            if self.stopped.get() {
                break;
            }
            self.vpn.refresh(&state);
            let active = state.active_connections.iter().find(|active| {
                matches!(active.kind.as_str(), "vpn" | "wireguard") && active.state == 2
            });
            self.vpn_tile
                .set_subtitle(active.map_or("", |active| active.name.as_str()));
            self.vpn_tile.set_icon(if active.is_some() {
                "network-vpn-symbolic"
            } else {
                "network-vpn-disconnected-symbolic"
            });
            self.vpn_tile.set_toggled(active.is_some());
            self.vpn_tile
                .set_sensitive(state.available && state.networking_enabled);
            self.airplane
                .set_toggled(state.available && !state.networking_enabled);
            let busy = self
                .networking_request
                .borrow()
                .as_ref()
                .is_some_and(|request| request.active());
            self.airplane.set_sensitive(state.available && !busy);
            if self.stopped.get() {
                self.vpn_tile.set_sensitive(false);
                self.airplane.set_sensitive(false);
                break;
            }
            if structure {
                let callbacks = self.observers.borrow().clone();
                for callback in callbacks {
                    callback();
                }
            }
        }
        self.updating.set(false);
    }
    fn make_device(self: &Rc<Self>, device: &Device) -> Rc<DeviceTile> {
        let wifi =
            (device.kind == 2).then(|| WifiMenu::new(self.service.clone(), device.id.clone()));
        let tile = GridButton::new(
            if wifi.is_some() { "Wi-Fi" } else { "Wired" },
            Some(""),
            "network-wired-symbolic",
            wifi.as_ref().map(|menu| menu.menu.widget().upcast_ref()),
        );
        if let Some(menu) = &wifi {
            let weak = Rc::downgrade(menu);
            tile.revealer()
                .connect_reveal_child_notify(move |revealer| {
                    if let Some(menu) = weak.upgrade() {
                        if revealer.reveals_child() {
                            menu.scan();
                        } else {
                            menu.hide_passwords();
                            menu.cancel_scan();
                        }
                    }
                });
            let weak = Rc::downgrade(self);
            let weak_tile = Rc::downgrade(&tile);
            let id = device.id.clone();
            tile.toggle().connect_clicked(move |_| {
                if let (Some(this), Some(tile)) = (weak.upgrade(), weak_tile.upgrade()) {
                    let current = this
                        .devices
                        .borrow()
                        .get(&id)
                        .is_some_and(|current| Rc::ptr_eq(&current.tile, &tile));
                    if current && !this.stopped.get() {
                        this.set_wireless();
                    }
                }
            });
        }
        Rc::new(DeviceTile {
            kind: device.kind,
            tile,
            wifi,
        })
    }
    fn set_networking(self: &Rc<Self>) {
        if self
            .networking_request
            .borrow()
            .as_ref()
            .is_some_and(|request| request.active())
        {
            return;
        }
        let state = self.service.state();
        if !state.available {
            return;
        }
        let request = Rc::new(Request::default());
        self.networking_request.replace(Some(request.clone()));
        let weak = Rc::downgrade(self);
        let done = request.clone();
        let cancel = self
            .service
            .set_networking(!state.networking_enabled, move |result| {
                done.finish();
                if let Some(this) = weak.upgrade().filter(|this| !this.stopped.get()) {
                    this.airplane
                        .widget()
                        .set_tooltip_text(result.as_ref().err().map(|error| error.message()));
                    this.refresh();
                }
            });
        request.attach(cancel);
        self.refresh();
    }
    fn set_wireless(self: &Rc<Self>) {
        if self
            .wireless_request
            .borrow()
            .as_ref()
            .is_some_and(|request| request.active())
        {
            return;
        }
        let state = self.service.state();
        if !state.available || !state.networking_enabled || !state.wireless_hardware_enabled {
            return;
        }
        let request = Rc::new(Request::default());
        self.wireless_request.replace(Some(request.clone()));
        let weak = Rc::downgrade(self);
        let done = request.clone();
        let cancel = self
            .service
            .set_wireless(!state.wireless_enabled, move |result| {
                done.finish();
                if let Some(this) = weak.upgrade().filter(|this| !this.stopped.get()) {
                    let tiles = this.devices.borrow().values().cloned().collect::<Vec<_>>();
                    for tile in tiles {
                        if tile.wifi.is_some() {
                            tile.tile.widget().set_tooltip_text(
                                result.as_ref().err().map(|error| error.message()),
                            );
                        }
                    }
                    this.refresh();
                }
            });
        request.attach(cancel);
        self.refresh();
    }
}
impl Drop for NetworkControls {
    fn drop(&mut self) {
        self.stop();
    }
}

struct WifiMenu {
    service: NetworkService,
    device: String,
    menu: Menu,
    spinner: gtk::Spinner,
    refresh_button: gtk::Button,
    failure: gtk::Button,
    rows: RefCell<HashMap<AccessPointKey, Rc<WifiRow>>>,
    scan: RefCell<Option<Rc<Request>>>,
    stopped: Cell<bool>,
    enabled: Cell<bool>,
}
impl WifiMenu {
    fn new(service: NetworkService, device: String) -> Rc<Self> {
        let menu = Menu::new("Wi-Fi", "network-wireless-signal-excellent-symbolic", true);
        menu.widget().set_size_request(-1, 420);
        let failure_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        failure_box.add_css_class("failure-banner");
        let failure = gtk::Button::with_label("Failed to connect to network");
        failure.set_hexpand(true);
        failure_box.append(&failure);
        menu.banner().set_child(Some(&failure_box));
        let spinner = gtk::Spinner::new();
        spinner.set_visible(false);
        menu.heading().append(&spinner);
        let refresh_button = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh_button.set_hexpand(true);
        refresh_button.set_halign(gtk::Align::End);
        menu.heading().append(&refresh_button);
        let this = Rc::new(Self {
            service,
            device,
            menu,
            spinner,
            refresh_button,
            failure,
            rows: RefCell::new(HashMap::new()),
            scan: RefCell::new(None),
            stopped: Cell::new(false),
            enabled: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        this.refresh_button.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.scan();
            }
        });
        let weak = Rc::downgrade(&this);
        this.failure.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.menu.banner().set_reveal_child(false);
            }
        });
        this
    }
    fn refresh(self: &Rc<Self>, state: &NetworkState, device: &Device) {
        if self.stopped.get() {
            return;
        }
        let enabled = state.available
            && state.networking_enabled
            && state.wireless_enabled
            && state.wireless_hardware_enabled
            && !matches!(device.state, 0 | 10 | 20);
        self.enabled.set(enabled);
        if !enabled {
            self.cancel_scan();
            self.hide_passwords();
        }
        let scanning = self
            .scan
            .borrow()
            .as_ref()
            .is_some_and(|request| request.active());
        self.refresh_button.set_sensitive(enabled && !scanning);
        if device.state == 120 {
            self.error("Failed to connect to network", None);
        } else if device.state == 100 {
            self.menu.banner().set_reveal_child(false);
        }
        let points = if enabled {
            strongest(state, &self.device)
        } else {
            Vec::new()
        };
        let mut old = self.rows.take();
        let mut next = HashMap::new();
        let mut ordered = Vec::new();
        for point in points {
            if self.stopped.get() {
                break;
            }
            let id = key(&point);
            let row = old
                .remove(&id)
                .unwrap_or_else(|| WifiRow::new(self, point.clone()));
            row.point.replace(point.clone());
            row.update(&point, device.state);
            ordered.push(row.clone());
            next.insert(id, row);
        }
        self.rows.replace(next);
        for (_, row) in old {
            row.stop();
            if row.root.parent().is_some() {
                self.menu.options().remove(&row.root);
            }
        }
        if self.stopped.get() {
            for row in ordered {
                row.stop();
            }
            return;
        }
        let mut previous: Option<gtk::Widget> = None;
        for row in ordered {
            if row.root.parent().is_none() {
                self.menu.options().append(&row.root);
            }
            self.menu
                .options()
                .reorder_child_after(&row.root, previous.as_ref());
            previous = Some(row.root.clone().upcast());
            if self.stopped.get() {
                row.stop();
                break;
            }
        }
    }
    fn scan(self: &Rc<Self>) {
        if self.stopped.get()
            || !self.enabled.get()
            || self
                .scan
                .borrow()
                .as_ref()
                .is_some_and(|request| request.active())
        {
            return;
        }
        let request = Rc::new(Request::default());
        self.scan.replace(Some(request.clone()));
        self.spinner.set_visible(true);
        self.spinner.start();
        self.refresh_button.set_sensitive(false);
        if self.stopped.get() {
            request.cancel();
            return;
        }
        let weak = Rc::downgrade(self);
        let done = request.clone();
        let cancel = self.service.request_scan(&self.device, move |result| {
            done.finish();
            if done.cancelled.get() {
                return;
            }
            if let Some(this) = weak.upgrade().filter(|this| !this.stopped.get()) {
                this.spinner.stop();
                this.spinner.set_visible(false);
                this.refresh_button.set_sensitive(this.enabled.get());
                if let Err(error) = result
                    && !error.matches(gio::IOErrorEnum::Cancelled)
                {
                    this.error("Failed to scan for networks", Some(error.message()));
                }
            }
        });
        request.attach(cancel);
    }
    fn cancel_scan(&self) {
        let request = self.scan.borrow_mut().take();
        if let Some(request) = request {
            request.cancel();
        }
        self.spinner.stop();
        self.spinner.set_visible(false);
    }
    fn hide_passwords(&self) {
        let rows = self.rows.borrow().values().cloned().collect::<Vec<_>>();
        for row in rows {
            row.password.set_text("");
            row.revealer.set_reveal_child(false);
        }
    }
    fn reveal_password(&self, row: &WifiRow) {
        let reveal = !row.revealer.reveals_child();
        self.hide_passwords();
        if reveal && !self.stopped.get() && !row.stopped.get() {
            row.revealer.set_reveal_child(true);
            row.password.grab_focus();
        }
    }
    fn error(&self, title: &str, diagnostic: Option<&str>) {
        if self.stopped.get() {
            return;
        }
        self.failure.set_label(title);
        self.failure.set_tooltip_text(diagnostic);
        self.menu.banner().set_reveal_child(true);
    }
    fn stop(&self) {
        if self.stopped.replace(true) {
            return;
        }
        self.enabled.set(false);
        self.cancel_scan();
        let rows = self.rows.borrow().values().cloned().collect::<Vec<_>>();
        for row in rows {
            row.stop();
        }
        self.menu.widget().set_sensitive(false);
    }
}
impl Drop for WifiMenu {
    fn drop(&mut self) {
        self.stop();
    }
}
struct WifiRow {
    root: gtk::Box,
    button: gtk::Button,
    label: gtk::Label,
    icon: gtk::Image,
    security: gtk::Image,
    active: gtk::Image,
    spinner: gtk::Spinner,
    revealer: gtk::Revealer,
    password: gtk::PasswordEntry,
    point: RefCell<AccessPoint>,
    request: RefCell<Option<Rc<Request>>>,
    stopped: Cell<bool>,
}
impl WifiRow {
    fn new(menu: &Rc<WifiMenu>, point: AccessPoint) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("quick-settings-menu-option-wifi");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let icon = gtk::Image::new();
        let security = gtk::Image::from_icon_name("network-wireless-encrypted-symbolic");
        security.set_pixel_size(10);
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let active = gtk::Image::from_icon_name("object-select-symbolic");
        let spinner = gtk::Spinner::new();
        row.append(&icon);
        row.append(&security);
        row.append(&label);
        row.append(&active);
        row.append(&spinner);
        let button = gtk::Button::new();
        button.set_child(Some(&row));
        root.append(&button);
        let entry_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        entry_box.add_css_class("quick-settings-menu-option-wifi-password-entry-container");
        entry_box.append(&gtk::Image::from_icon_name("dialog-password-symbolic"));
        let password = gtk::PasswordEntry::new();
        password.set_hexpand(true);
        password.set_show_peek_icon(true);
        entry_box.append(&password);
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SwingDown)
            .transition_duration(350)
            .child(&entry_box)
            .build();
        root.append(&revealer);
        let this = Rc::new(Self {
            root,
            button,
            label,
            icon,
            security,
            active,
            spinner,
            revealer,
            password,
            point: RefCell::new(point),
            request: RefCell::new(None),
            stopped: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        let weak_menu = Rc::downgrade(menu);
        this.button.connect_clicked(move |_| {
            if let (Some(this), Some(menu)) = (weak.upgrade(), weak_menu.upgrade())
                && !this.stopped.get()
                && !menu.stopped.get()
                && menu.enabled.get()
            {
                let secured = {
                    let point = this.point.borrow();
                    point.wpa_flags != 0 || point.rsn_flags != 0
                };
                if secured {
                    menu.reveal_password(&this);
                } else {
                    this.join(&menu, None);
                }
            }
        });
        let weak = Rc::downgrade(&this);
        let weak_menu = Rc::downgrade(menu);
        this.password.connect_activate(move |entry| {
            if let (Some(this), Some(menu)) = (weak.upgrade(), weak_menu.upgrade())
                && !this.stopped.get()
                && !menu.stopped.get()
                && menu.enabled.get()
            {
                let password = entry.text().to_string();
                entry.set_text("");
                this.revealer.set_reveal_child(false);
                if !this.stopped.get() && !menu.stopped.get() {
                    this.join(&menu, Some(&password));
                }
            }
        });
        this
    }
    fn update(&self, point: &AccessPoint, state: u32) {
        if self.stopped.get() {
            return;
        }
        self.label.set_label(&point.name);
        self.icon.set_icon_name(Some(signal_icon(point.strength)));
        self.security
            .set_visible(point.wpa_flags != 0 || point.rsn_flags != 0);
        self.active.set_visible(point.active && state == 100);
        let pending = self
            .request
            .borrow()
            .as_ref()
            .is_some_and(|request| request.active());
        let busy = pending || point.active && (40..=90).contains(&state);
        self.spinner.set_spinning(busy);
        self.spinner.set_visible(busy);
        self.button.set_sensitive(!pending);
        if self.stopped.get() {
            self.button.set_sensitive(false);
            self.password.set_sensitive(false);
        }
    }
    fn join(self: &Rc<Self>, menu: &Rc<WifiMenu>, password: Option<&str>) {
        if self
            .request
            .borrow()
            .as_ref()
            .is_some_and(|request| request.active())
        {
            return;
        }
        let point = self.point.borrow().clone();
        let request = Rc::new(Request::default());
        self.request.replace(Some(request.clone()));
        self.button.set_sensitive(false);
        self.spinner.set_visible(true);
        self.spinner.start();
        if self.stopped.get() || menu.stopped.get() {
            request.cancel();
            return;
        }
        let weak = Rc::downgrade(self);
        let weak_menu = Rc::downgrade(menu);
        let done = request.clone();
        let cancel =
            menu.service
                .join_access_point(&menu.device, &point.id, password, move |result| {
                    done.finish();
                    if let (Some(this), Some(menu)) = (weak.upgrade(), weak_menu.upgrade())
                        && !this.stopped.get()
                        && !menu.stopped.get()
                    {
                        this.spinner.stop();
                        this.spinner.set_visible(false);
                        this.button.set_sensitive(menu.enabled.get());
                        if let Err(error) = result
                            && !error.matches(gio::IOErrorEnum::Cancelled)
                        {
                            menu.error("Failed to connect to network", Some(error.message()));
                        }
                    }
                });
        request.attach(cancel);
    }
    fn stop(&self) {
        if self.stopped.replace(true) {
            return;
        }
        let request = self.request.borrow_mut().take();
        if let Some(request) = request {
            request.cancel();
        }
        self.password.set_text("");
        self.revealer.set_reveal_child(false);
        self.spinner.stop();
        self.spinner.set_visible(false);
        self.root.set_sensitive(false);
    }
}
impl Drop for WifiRow {
    fn drop(&mut self) {
        self.stop();
    }
}

struct VpnRow {
    widget: adw::SwitchRow,
    updating: Cell<bool>,
    stopped: Cell<bool>,
    request: RefCell<Option<Rc<Request>>>,
}
impl VpnRow {
    fn stop(&self) {
        self.stopped.set(true);
        let request = self.request.borrow_mut().take();
        if let Some(request) = request {
            request.cancel();
        }
        self.widget.set_sensitive(false);
    }
}
impl Drop for VpnRow {
    fn drop(&mut self) {
        self.stop();
    }
}
struct VpnMenu {
    service: NetworkService,
    menu: Menu,
    rows: RefCell<HashMap<String, Rc<VpnRow>>>,
    stopped: Cell<bool>,
    updating: Cell<bool>,
    pending: RefCell<Option<NetworkState>>,
}
impl VpnMenu {
    fn new(service: NetworkService) -> Rc<Self> {
        Rc::new(Self {
            service,
            menu: Menu::new("VPN", "network-vpn-symbolic", false),
            rows: RefCell::new(HashMap::new()),
            stopped: Cell::new(false),
            updating: Cell::new(false),
            pending: RefCell::new(None),
        })
    }
    fn refresh(self: &Rc<Self>, state: &NetworkState) {
        if self.stopped.get() {
            return;
        }
        self.pending.replace(Some(state.clone()));
        if self.updating.replace(true) {
            return;
        }
        loop {
            let pending = self.pending.borrow_mut().take();
            let Some(state) = pending else {
                break;
            };
            if self.stopped.get() {
                break;
            }
            self.apply(&state);
        }
        self.updating.set(false);
    }
    fn apply(self: &Rc<Self>, state: &NetworkState) {
        let mut old = self.rows.take();
        let mut next = HashMap::new();
        let mut ordered = Vec::new();
        for saved in state.connections.iter().filter(|saved| saved.is_vpn()) {
            if self.stopped.get() {
                break;
            }
            let row = old.remove(&saved.id).unwrap_or_else(|| self.row(saved));
            row.updating.set(true);
            row.widget.set_title(&saved.name);
            row.widget
                .set_active(state.active_connections.iter().any(|active| {
                    active.connection.as_deref() == Some(&saved.id) && active.state == 2
                }));
            row.updating.set(false);
            row.widget.set_sensitive(
                state.available
                    && state.networking_enabled
                    && !row
                        .request
                        .borrow()
                        .as_ref()
                        .is_some_and(|request| request.active()),
            );
            row.widget.set_focusable(state.networking_enabled);
            ordered.push(row.clone());
            next.insert(saved.id.clone(), row);
        }
        self.rows.replace(next);
        for (_, row) in old {
            row.stop();
            if row.widget.parent().is_some() {
                self.menu.options().remove(&row.widget);
            }
        }
        let mut previous: Option<gtk::Widget> = None;
        for row in ordered {
            if self.stopped.get() {
                row.stop();
                continue;
            }
            if row.widget.parent().is_none() {
                self.menu.options().append(&row.widget);
            }
            self.menu
                .options()
                .reorder_child_after(&row.widget, previous.as_ref());
            previous = Some(row.widget.clone().upcast());
        }
    }
    fn row(self: &Rc<Self>, saved: &SavedConnection) -> Rc<VpnRow> {
        let widget = adw::SwitchRow::new();
        widget.add_css_class("switch-row-vpn");
        let row = Rc::new(VpnRow {
            widget,
            updating: Cell::new(false),
            stopped: Cell::new(false),
            request: RefCell::new(None),
        });
        let weak = Rc::downgrade(self);
        let weak_row = Rc::downgrade(&row);
        let id = saved.id.clone();
        row.widget.connect_active_notify(move |widget| {
            let (Some(menu), Some(row)) = (weak.upgrade(), weak_row.upgrade()) else {
                return;
            };
            if menu.stopped.get()
                || row.stopped.get()
                || row.updating.get()
                || row
                    .request
                    .borrow()
                    .as_ref()
                    .is_some_and(|request| request.active())
            {
                return;
            }
            let state = menu.service.state();
            if !state.available || !state.networking_enabled {
                return;
            }
            let enabled = widget.is_active();
            let request = Rc::new(Request::default());
            row.request.replace(Some(request.clone()));
            widget.set_sensitive(false);
            if menu.stopped.get() || row.stopped.get() {
                request.cancel();
                return;
            }
            let weak = Rc::downgrade(&menu);
            let weak_row = Rc::downgrade(&row);
            let done = request.clone();
            let cancel = menu.service.set_vpn(&id, enabled, move |result| {
                done.finish();
                if let (Some(menu), Some(row)) = (weak.upgrade(), weak_row.upgrade())
                    && !menu.stopped.get()
                    && !row.stopped.get()
                {
                    row.widget
                        .set_tooltip_text(result.as_ref().err().map(|error| error.message()));
                    menu.refresh(&menu.service.state());
                }
            });
            request.attach(cancel);
        });
        row
    }
    fn stop(&self) {
        if self.stopped.replace(true) {
            return;
        }
        let rows = self.rows.borrow().values().cloned().collect::<Vec<_>>();
        for row in rows {
            row.stop();
        }
        self.menu.widget().set_sensitive(false);
    }
}
impl Drop for VpnMenu {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ap(id: &str, bytes: &[u8], strength: u8, active: bool) -> AccessPoint {
        AccessPoint {
            id: id.into(),
            device: "wifi".into(),
            ssid: bytes.into(),
            name: String::from_utf8_lossy(bytes).into(),
            strength,
            flags: 0,
            wpa_flags: 0,
            rsn_flags: 0,
            active,
        }
    }
    #[test]
    fn strongest_bssid_keeps_connected_ssid_first_without_merging_security_or_bytes() {
        let mut secured = ap("secure", b"same", 99, false);
        secured.rsn_flags = 256;
        let state = NetworkState {
            access_points: vec![
                ap("weak", b"same", 20, true),
                ap("strong", b"same", 80, false),
                secured,
                ap("raw1", &[0xff], 20, false),
                ap("raw2", &[0xfe], 40, false),
                ap("hidden", &[], 90, false),
                ap("nul", &[0], 90, false),
            ],
            ..Default::default()
        };
        let rows = strongest(&state, "wifi");
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].id, "strong");
        assert!(rows[0].active);
        assert!(rows.iter().any(|point| point.id == "secure"));
        assert_eq!(rows.iter().filter(|point| point.name == "�").count(), 2);
    }
    #[test]
    fn display_states_match_existing_wired_and_signal_boundaries() {
        assert_eq!(ethernet_state(30).0, "no network");
        assert_eq!(ethernet_state(50).0, "no internet");
        assert_eq!(ethernet_state(100).0, "connected");
        for (strength, level) in [
            (0, "weak"),
            (25, "weak"),
            (26, "ok"),
            (50, "ok"),
            (51, "good"),
            (75, "good"),
            (76, "excellent"),
            (100, "excellent"),
        ] {
            assert!(signal_icon(strength).contains(level));
        }
    }
    #[test]
    fn cancellation_before_request_attachment_cannot_restart_it() {
        let request = Request::default();
        request.cancel();
        let cancel = gio::Cancellable::new();
        request.attach(cancel.clone());
        assert!(cancel.is_cancelled());
        assert!(!request.active());
        let request = Request::default();
        request.finish();
        request.attach(gio::Cancellable::new());
        assert!(request.cancel.borrow().is_none());
    }
}
