//! UPower inventory and state. D-Bus proxies stay private to the service.
use gio::prelude::*;
use glib::{subclass::prelude::*, variant::ObjectPath};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    sync::OnceLock,
    time::Duration,
};

const NAME: &str = "org.freedesktop.UPower";
const PATH: &str = "/org/freedesktop/UPower";
const DEVICE: &str = "org.freedesktop.UPower.Device";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    LinePower,
    Battery,
    Ups,
    Other(u32),
}

impl DeviceKind {
    pub fn from_code(code: u32) -> Self {
        match code {
            1 => Self::LinePower,
            2 => Self::Battery,
            3 => Self::Ups,
            code => Self::Other(code),
        }
    }

    pub fn code(self) -> u32 {
        match self {
            Self::LinePower => 1,
            Self::Battery => 2,
            Self::Ups => 3,
            Self::Other(code) => code,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeviceState {
    #[default]
    Unknown,
    Charging,
    Discharging,
    Empty,
    FullyCharged,
    PendingCharge,
    PendingDischarge,
    Other(u32),
}

impl DeviceState {
    pub fn from_code(code: u32) -> Self {
        match code {
            0 => Self::Unknown,
            1 => Self::Charging,
            2 => Self::Discharging,
            3 => Self::Empty,
            4 => Self::FullyCharged,
            5 => Self::PendingCharge,
            6 => Self::PendingDischarge,
            code => Self::Other(code),
        }
    }

    pub fn code(self) -> u32 {
        match self {
            Self::Unknown => 0,
            Self::Charging => 1,
            Self::Discharging => 2,
            Self::Empty => 3,
            Self::FullyCharged => 4,
            Self::PendingCharge => 5,
            Self::PendingDischarge => 6,
            Self::Other(code) => code,
        }
    }

    pub fn is_charging(self) -> bool {
        matches!(
            self,
            Self::Charging | Self::FullyCharged | Self::PendingCharge
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerDevice {
    pub path: String,
    pub native_path: String,
    pub vendor: String,
    pub model: String,
    pub serial: String,
    pub kind: DeviceKind,
    pub state: DeviceState,
    pub power_supply: bool,
    pub present: bool,
    pub rechargeable: bool,
    pub online: bool,
    pub percentage: Option<f64>,
    pub time_to_empty: i64,
    pub time_to_full: i64,
    pub icon_name: String,
}

impl PowerDevice {
    pub fn preferred_icon_name(&self) -> &'static str {
        if !self.present {
            "battery-missing-symbolic"
        } else {
            preferred_icon_name(self.rechargeable, self.state, self.percentage)
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PowerState {
    pub available: bool,
    pub daemon_version: String,
    pub on_battery: bool,
    pub lid_is_closed: bool,
    pub lid_is_present: bool,
}

/// Keep Way Shell's icon choices, including its charging threshold at 10%.
pub fn preferred_icon_name(
    rechargeable: bool,
    state: DeviceState,
    percentage: Option<f64>,
) -> &'static str {
    if !rechargeable {
        return "ac-adapter-symbolic";
    }
    let Some(percentage) =
        percentage.filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
    else {
        return "battery-missing-symbolic";
    };
    let charging = state.is_charging();
    if percentage <= 10.0 {
        return if charging {
            "battery-caution-charging-symbolic"
        } else {
            "battery-level-0-symbolic"
        };
    }
    if percentage >= 90.0 && state == DeviceState::FullyCharged {
        return "battery-full-charging-symbolic";
    }
    const NORMAL: [&str; 10] = [
        "battery-level-0-symbolic",
        "battery-level-10-symbolic",
        "battery-level-20-symbolic",
        "battery-level-30-symbolic",
        "battery-level-40-symbolic",
        "battery-level-50-symbolic",
        "battery-level-60-symbolic",
        "battery-level-70-symbolic",
        "battery-level-80-symbolic",
        "battery-level-90-symbolic",
    ];
    const CHARGING: [&str; 10] = [
        "battery-level-0-charging-symbolic",
        "battery-level-10-charging-symbolic",
        "battery-level-20-charging-symbolic",
        "battery-level-30-charging-symbolic",
        "battery-level-40-charging-symbolic",
        "battery-level-50-charging-symbolic",
        "battery-level-60-charging-symbolic",
        "battery-level-70-charging-symbolic",
        "battery-level-80-charging-symbolic",
        "battery-level-90-charging-symbolic",
    ];
    let index = (percentage / 10.0).clamp(1.0, 9.0) as usize;
    if charging {
        CHARGING[index]
    } else {
        NORMAL[index]
    }
}

pub(super) struct Proxy {
    proxy: gio::DBusProxy,
    handlers: Vec<glib::SignalHandlerId>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        for handler in self.handlers.drain(..) {
            self.proxy.disconnect(handler);
        }
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct PowerService {
        pub watcher: RefCell<Option<Box<dyn FnOnce()>>>,
        pub owner: RefCell<Option<(gio::DBusConnection, String)>>,
        pub generation: Cell<u64>,
        pub disposed: Cell<bool>,
        pub inventory_ready: Cell<bool>,
        pub device_retries: RefCell<HashMap<String, glib::JoinHandle<()>>>,
        pub cancellable: RefCell<Option<gio::Cancellable>>,
        pub retry: RefCell<Option<glib::JoinHandle<()>>>,
        pub(super) root: RefCell<Option<Proxy>>,
        pub(super) proxies: RefCell<HashMap<String, Proxy>>,
        pub pending: RefCell<HashMap<String, gio::Cancellable>>,
        pub order: RefCell<Vec<String>>,
        pub removed: RefCell<HashSet<String>>,
        pub devices: RefCell<Vec<PowerDevice>>,
        pub state: RefCell<PowerState>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PowerService {
        const NAME: &'static str = "WayShellPowerService";
        type Type = super::PowerService;
    }

    impl ObjectImpl for PowerService {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("changed").build()])
        }

        fn dispose(&self) {
            self.disposed.set(true);
            if let Some(watcher) = self.watcher.borrow_mut().take() {
                watcher();
            }
            self.obj().reset();
            self.owner.borrow_mut().take();
        }
    }
}

glib::wrapper! { pub struct PowerService(ObjectSubclass<imp::PowerService>); }

impl Default for PowerService {
    fn default() -> Self {
        Self::new()
    }
}

impl PowerService {
    pub fn new() -> Self {
        let service: Self = glib::Object::new();
        let appeared = service.downgrade();
        let vanished = service.downgrade();
        let watcher = gio::bus_watch_name(
            gio::BusType::System,
            NAME,
            gio::BusNameWatcherFlags::NONE,
            move |connection, _, owner| {
                if let Some(service) = appeared.upgrade() {
                    service.appeared(connection, owner);
                }
            },
            move |_, _| {
                if let Some(service) = vanished.upgrade() {
                    service.vanished();
                }
            },
        );
        // gio 0.21 exposes two distinct WatcherId types; retain the inferred
        // bus watcher in its teardown closure instead of naming the hidden type.
        service
            .imp()
            .watcher
            .replace(Some(Box::new(move || gio::bus_unwatch_name(watcher))));
        service
    }

    /// Use a supplied bus for private fixtures or an already connected session.
    pub fn on_connection(connection: &gio::DBusConnection) -> Self {
        let service: Self = glib::Object::new();
        let appeared = service.downgrade();
        let vanished = service.downgrade();
        let watcher = gio::bus_watch_name_on_connection(
            connection,
            NAME,
            gio::BusNameWatcherFlags::NONE,
            move |connection, _, owner| {
                if let Some(service) = appeared.upgrade() {
                    service.appeared(connection, owner);
                }
            },
            move |_, _| {
                if let Some(service) = vanished.upgrade() {
                    service.vanished();
                }
            },
        );
        // gio 0.21 exposes two distinct WatcherId types; retain the inferred
        // bus watcher in its teardown closure instead of naming the hidden type.
        service
            .imp()
            .watcher
            .replace(Some(Box::new(move || gio::bus_unwatch_name(watcher))));
        service
    }

    pub fn state(&self) -> PowerState {
        self.imp().state.borrow().clone()
    }

    pub fn devices(&self) -> Vec<PowerDevice> {
        self.imp().devices.borrow().clone()
    }

    pub fn primary_device(&self) -> Option<PowerDevice> {
        let devices = self.imp().devices.borrow();
        // Keep the first system battery; a peripheral reporting Type=Battery
        // must not replace the laptop battery. AC names need not be "AC".
        devices
            .iter()
            .find(|device| {
                device.present && device.power_supply && device.kind == DeviceKind::Battery
            })
            .or_else(|| {
                devices
                    .iter()
                    .find(|device| device.kind == DeviceKind::LinePower)
            })
            .cloned()
    }

    fn appeared(&self, connection: gio::DBusConnection, owner: &str) {
        self.reset();
        self.imp()
            .owner
            .replace(Some((connection.clone(), owner.to_owned())));
        let generation = self.imp().generation.get();
        let cancellable = gio::Cancellable::new();
        self.imp().cancellable.replace(Some(cancellable.clone()));
        let weak = self.downgrade();
        gio::DBusProxy::new(
            &connection,
            gio::DBusProxyFlags::DO_NOT_AUTO_START
                | gio::DBusProxyFlags::GET_INVALIDATED_PROPERTIES,
            None,
            Some(owner),
            PATH,
            NAME,
            Some(&cancellable),
            move |result| {
                let Some(service) = weak.upgrade().filter(|s| s.current(generation)) else {
                    return;
                };
                match result {
                    Ok(proxy) => service.root_ready(proxy, generation),
                    Err(error) => service.failed(&error),
                }
            },
        );
    }

    fn current(&self, generation: u64) -> bool {
        self.imp().generation.get() == generation
    }

    fn root_ready(&self, proxy: gio::DBusProxy, generation: u64) {
        let weak = self.downgrade();
        let properties = proxy.connect_local("g-properties-changed", false, move |_| {
            if let Some(service) = weak.upgrade() {
                service.refresh();
            }
            None
        });
        let weak = self.downgrade();
        let signals = proxy.connect_local("g-signal", false, move |values| {
            if let Some(service) = weak.upgrade() {
                let name = values[2].get::<String>().unwrap();
                let parameters = values[3].get::<glib::Variant>().unwrap();
                if let Some((path,)) = parameters.get::<(ObjectPath,)>() {
                    match name.as_str() {
                        "DeviceAdded" => service.add_device(path.as_str(), generation),
                        "DeviceRemoved" => service.remove_device(path.as_str()),
                        _ => (),
                    }
                }
            }
            None
        });
        self.imp().root.replace(Some(Proxy {
            proxy: proxy.clone(),
            handlers: vec![properties, signals],
        }));
        let weak = self.downgrade();
        let cancellable = self.imp().cancellable.borrow().clone();
        proxy.call(
            "EnumerateDevices",
            None,
            gio::DBusCallFlags::NONE,
            2_000,
            cancellable.as_ref(),
            move |result| {
                let Some(service) = weak.upgrade().filter(|s| s.current(generation)) else {
                    return;
                };
                match result {
                    Ok(reply) => {
                        if let Some((paths,)) = reply.get::<(Vec<ObjectPath>,)>() {
                            for path in paths {
                                if !service.imp().removed.borrow().contains(path.as_str()) {
                                    service.add_device(path.as_str(), generation);
                                }
                            }
                            service.imp().inventory_ready.set(true);
                            service.refresh();
                        } else {
                            service.failed(&glib::Error::new(
                                gio::IOErrorEnum::InvalidData,
                                "UPower returned an invalid device inventory",
                            ));
                        }
                    }
                    Err(error) => service.failed(&error),
                }
            },
        );
    }

    fn add_device(&self, path: &str, generation: u64) {
        self.imp().removed.borrow_mut().remove(path);
        if self.imp().pending.borrow().contains_key(path)
            || self.imp().proxies.borrow().contains_key(path)
        {
            return;
        }
        let Some((connection, owner)) = self.imp().owner.borrow().clone() else {
            return;
        };
        if !self.imp().order.borrow().iter().any(|item| item == path) {
            self.imp().order.borrow_mut().push(path.to_owned());
        }
        let cancellable = gio::Cancellable::new();
        self.imp()
            .pending
            .borrow_mut()
            .insert(path.to_owned(), cancellable.clone());
        let weak = self.downgrade();
        let device_path = path.to_owned();
        let request = cancellable.clone();
        gio::DBusProxy::new(
            &connection,
            gio::DBusProxyFlags::DO_NOT_AUTO_START
                | gio::DBusProxyFlags::GET_INVALIDATED_PROPERTIES,
            None,
            Some(&owner),
            path,
            DEVICE,
            Some(&cancellable),
            move |result| {
                let Some(service) = weak.upgrade().filter(|s| s.current(generation)) else {
                    return;
                };
                // A removed path can already have a new request when the old
                // cancellation callback runs. Only its own request may complete it.
                if service.imp().pending.borrow().get(&device_path) != Some(&request) {
                    return;
                }
                service.imp().pending.borrow_mut().remove(&device_path);
                match result {
                    Ok(proxy) if snapshot(&device_path, &proxy).is_some() => {
                        let weak = service.downgrade();
                        let handler =
                            proxy.connect_local("g-properties-changed", false, move |_| {
                                if let Some(service) = weak.upgrade() {
                                    service.refresh();
                                }
                                None
                            });
                        service.imp().proxies.borrow_mut().insert(
                            device_path,
                            Proxy {
                                proxy,
                                handlers: vec![handler],
                            },
                        );
                        service.refresh();
                    }
                    Ok(_) => service.device_failed(
                        &device_path,
                        generation,
                        "device properties are incomplete",
                    ),
                    Err(error) => {
                        service.device_failed(&device_path, generation, &error.to_string())
                    }
                }
            },
        );
    }

    fn device_failed(&self, path: &str, generation: u64, error: &str) {
        glib::g_warning!(
            "way-shell",
            "Could not read UPower device {path}: {error}; retrying"
        );
        let weak = self.downgrade();
        let path_owned = path.to_owned();
        let retry = glib::MainContext::ref_thread_default().spawn_local(async move {
            glib::timeout_future(Duration::from_secs(1)).await;
            if let Some(service) = weak.upgrade().filter(|service| service.current(generation)) {
                service
                    .imp()
                    .device_retries
                    .borrow_mut()
                    .remove(&path_owned);
                service.add_device(&path_owned, generation);
            }
        });
        if let Some(previous) = self
            .imp()
            .device_retries
            .borrow_mut()
            .insert(path.to_owned(), retry)
        {
            previous.abort();
        }
    }

    fn remove_device(&self, path: &str) {
        self.imp().removed.borrow_mut().insert(path.to_owned());
        if let Some(retry) = self.imp().device_retries.borrow_mut().remove(path) {
            retry.abort();
        }
        if let Some(cancellable) = self.imp().pending.borrow_mut().remove(path) {
            cancellable.cancel();
        }
        self.imp().proxies.borrow_mut().remove(path);
        self.imp().order.borrow_mut().retain(|item| item != path);
        self.refresh();
    }

    fn refresh(&self) {
        let previous_state = self.state();
        if let Some(root) = self.imp().root.borrow().as_ref() {
            let mut state = self.imp().state.borrow_mut();
            state.available = self.imp().inventory_ready.get();
            state.daemon_version = property(&root.proxy, "DaemonVersion").unwrap_or_default();
            state.on_battery = property(&root.proxy, "OnBattery").unwrap_or(false);
            state.lid_is_present = property(&root.proxy, "LidIsPresent").unwrap_or(false);
            state.lid_is_closed = property(&root.proxy, "LidIsClosed").unwrap_or(false);
        }
        let devices: Vec<_> = {
            let proxies = self.imp().proxies.borrow();
            self.imp()
                .order
                .borrow()
                .iter()
                .filter_map(|path| {
                    proxies
                        .get(path)
                        .and_then(|proxy| snapshot(path, &proxy.proxy))
                })
                .collect()
        };
        let changed = *self.imp().devices.borrow() != devices || previous_state != self.state();
        self.imp().devices.replace(devices);
        if changed {
            self.emit_by_name::<()>("changed", &[]);
        }
    }

    fn failed(&self, error: &glib::Error) {
        glib::g_warning!("way-shell", "UPower is unavailable: {error}; retrying");
        self.reset();
        let weak = self.downgrade();
        let retry = glib::MainContext::ref_thread_default().spawn_local(async move {
            glib::timeout_future(Duration::from_secs(1)).await;
            if let Some(service) = weak.upgrade() {
                service.imp().retry.borrow_mut().take();
                let owner = service.imp().owner.borrow().clone();
                if let Some((connection, owner)) = owner {
                    service.appeared(connection, &owner);
                }
            }
        });
        self.imp().retry.replace(Some(retry));
    }

    fn vanished(&self) {
        self.imp().owner.borrow_mut().take();
        self.reset();
        glib::g_message!(
            "way-shell",
            "UPower is unavailable; battery controls wait for the service"
        );
    }

    fn reset(&self) {
        self.imp()
            .generation
            .set(self.imp().generation.get().wrapping_add(1));
        if let Some(retry) = self.imp().retry.borrow_mut().take() {
            retry.abort();
        }
        if let Some(cancellable) = self.imp().cancellable.borrow_mut().take() {
            cancellable.cancel();
        }
        for (_, cancellable) in self.imp().pending.borrow_mut().drain() {
            cancellable.cancel();
        }
        for (_, retry) in self.imp().device_retries.borrow_mut().drain() {
            retry.abort();
        }
        self.imp().inventory_ready.set(false);
        self.imp().root.borrow_mut().take();
        self.imp().proxies.borrow_mut().clear();
        self.imp().order.borrow_mut().clear();
        self.imp().removed.borrow_mut().clear();
        let changed =
            self.state() != PowerState::default() || !self.imp().devices.borrow().is_empty();
        self.imp().state.replace(PowerState::default());
        self.imp().devices.borrow_mut().clear();
        if changed && !self.imp().disposed.get() {
            self.emit_by_name::<()>("changed", &[]);
        }
    }
}

fn property<T: glib::variant::FromVariant>(proxy: &gio::DBusProxy, name: &str) -> Option<T> {
    proxy.cached_property(name)?.get()
}

fn snapshot(path: &str, proxy: &gio::DBusProxy) -> Option<PowerDevice> {
    let kind = DeviceKind::from_code(property(proxy, "Type")?);
    Some(PowerDevice {
        path: path.to_owned(),
        native_path: property(proxy, "NativePath").unwrap_or_default(),
        vendor: property(proxy, "Vendor").unwrap_or_default(),
        model: property(proxy, "Model").unwrap_or_default(),
        serial: property(proxy, "Serial").unwrap_or_default(),
        kind,
        state: DeviceState::from_code(property(proxy, "State").unwrap_or(0)),
        power_supply: property(proxy, "PowerSupply").unwrap_or(false),
        present: kind == DeviceKind::LinePower || property(proxy, "IsPresent").unwrap_or(false),
        rechargeable: property(proxy, "IsRechargeable").unwrap_or(false),
        online: property(proxy, "Online").unwrap_or(false),
        percentage: property::<f64>(proxy, "Percentage")
            .filter(|value| value.is_finite() && (0.0..=100.0).contains(value)),
        time_to_empty: property::<i64>(proxy, "TimeToEmpty").unwrap_or(0).max(0),
        time_to_full: property::<i64>(proxy, "TimeToFull").unwrap_or(0).max(0),
        icon_name: property(proxy, "IconName").unwrap_or_default(),
    })
}
