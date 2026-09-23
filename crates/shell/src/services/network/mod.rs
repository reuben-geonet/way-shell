//! NetworkManager inventory owns libnm state; callers receive owned domain values.
mod actions;
mod connections;
mod native;
use gio::prelude::*;
use glib::subclass::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    sync::OnceLock,
    time::Duration,
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub interface: String,
    pub description: String,
    pub kind: u32,
    pub state: u32,
    pub managed: bool,
    pub carrier: bool,
    pub available_connections: Vec<String>,
    pub active_connection: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessPoint {
    pub id: String,
    pub device: String,
    pub ssid: Vec<u8>,
    pub name: String,
    pub strength: u8,
    pub flags: u32,
    pub wpa_flags: u32,
    pub rsn_flags: u32,
    pub active: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedConnection {
    pub id: String,
    pub uuid: String,
    pub name: String,
    pub kind: String,
    pub ssid: Option<Vec<u8>>,
    pub last_used: u64,
}
impl SavedConnection {
    pub fn is_vpn(&self) -> bool {
        matches!(self.kind.as_str(), "vpn" | "wireguard")
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveConnection {
    pub id: String,
    pub connection: Option<String>,
    pub name: String,
    pub kind: String,
    pub state: u32,
    pub devices: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkState {
    pub available: bool,
    pub state: u32,
    pub networking_enabled: bool,
    pub wireless_enabled: bool,
    pub wireless_hardware_enabled: bool,
    pub primary: Option<String>,
    pub devices: Vec<Device>,
    pub access_points: Vec<AccessPoint>,
    pub connections: Vec<SavedConnection>,
    pub active_connections: Vec<ActiveConnection>,
}
impl NetworkState {
    pub fn has_wifi(&self) -> bool {
        self.devices.iter().any(|device| device.kind == 2)
    }
    pub fn wifi_state(&self) -> u32 {
        if !self.has_wifi() {
            return 0;
        }
        if !self.wireless_enabled {
            return 30;
        }
        let states = self
            .devices
            .iter()
            .filter(|device| device.kind == 2)
            .map(|device| device.state);
        if states.clone().any(|state| state == 100) {
            100
        } else if states.clone().any(|state| (40..=90).contains(&state)) {
            40
        } else {
            30
        }
    }
}
struct Subscription {
    object: glib::Object,
    handler: Option<glib::SignalHandlerId>,
}
impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(handler) = self.handler.take() {
            self.object.disconnect(handler);
        }
    }
}
mod imp {
    use super::*;
    #[derive(Default)]
    pub struct NetworkService {
        pub state: RefCell<NetworkState>,
        pub(super) client: RefCell<Option<native::Client>>,
        pub(super) watchers: RefCell<Vec<Subscription>>,
        pub connection: RefCell<Option<gio::DBusConnection>>,
        pub initializing: RefCell<Option<gio::Cancellable>>,
        pub retry: RefCell<Option<glib::JoinHandle<()>>>,
        pub generation: Cell<u64>,
        pub operations: RefCell<HashMap<u64, gio::Cancellable>>,
        pub next_operation: Cell<u64>,
        pub owner: RefCell<Option<String>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for NetworkService {
        const NAME: &'static str = "WayShellNetworkService";
        type Type = super::NetworkService;
    }
    impl ObjectImpl for NetworkService {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("changed").build()])
        }
        fn dispose(&self) {
            self.obj().stop();
        }
    }
}
glib::wrapper! {pub struct NetworkService(ObjectSubclass<imp::NetworkService>);}
impl Default for NetworkService {
    fn default() -> Self {
        Self::new()
    }
}
impl NetworkService {
    pub fn new() -> Self {
        let service: Self = glib::Object::new();
        service.initialize();
        service
    }
    pub fn on_connection(connection: &gio::DBusConnection) -> Self {
        let service: Self = glib::Object::new();
        service.imp().connection.replace(Some(connection.clone()));
        service.initialize();
        service
    }
    pub fn state(&self) -> NetworkState {
        self.imp().state.borrow().clone()
    }
    pub fn stop(&self) {
        self.imp()
            .generation
            .set(self.imp().generation.get().wrapping_add(1));
        if let Some(cancel) = self.imp().initializing.borrow_mut().take() {
            cancel.cancel();
        }
        if let Some(retry) = self.imp().retry.borrow_mut().take() {
            retry.abort();
        }
        let operations = std::mem::take(&mut *self.imp().operations.borrow_mut());
        for (_, cancel) in operations {
            cancel.cancel();
        }
        self.imp().watchers.borrow_mut().clear();
        self.imp().client.borrow_mut().take();
        self.imp().owner.borrow_mut().take();
        self.update(NetworkState::default());
    }
    fn initialize(&self) {
        self.stop();
        if self
            .imp()
            .connection
            .borrow()
            .as_ref()
            .is_some_and(|connection| connection.is_closed())
        {
            return;
        }
        let cancel = gio::Cancellable::new();
        self.imp().initializing.replace(Some(cancel.clone()));
        let generation = self.imp().generation.get();
        let weak = self.downgrade();
        let connection = self.imp().connection.borrow().clone();
        native::create(connection.as_ref(), &cancel, move |result| {
            let Some(service) = weak
                .upgrade()
                .filter(|service| service.imp().generation.get() == generation)
            else {
                return;
            };
            service.imp().initializing.borrow_mut().take();
            match result {
                Ok(client) => {
                    service.imp().client.replace(Some(client));
                    service.refresh();
                }
                Err(error) => {
                    glib::g_message!("way-shell", "NetworkManager unavailable: {error}; retrying");
                    let weak = service.downgrade();
                    let retry = glib::MainContext::ref_thread_default().spawn_local(async move {
                        glib::timeout_future(Duration::from_secs(1)).await;
                        if let Some(service) = weak.upgrade() {
                            service.imp().retry.borrow_mut().take();
                            service.initialize();
                        }
                    });
                    service.imp().retry.replace(Some(retry));
                }
            }
        });
    }
    fn refresh(&self) {
        let Some(client) = self.imp().client.borrow().clone() else {
            return;
        };
        let snapshot = client.snapshot();
        let owner = client.owner();
        if *self.imp().owner.borrow() != owner {
            self.imp().owner.replace(owner);
            self.imp()
                .generation
                .set(self.imp().generation.get().wrapping_add(1));
            let operations = std::mem::take(&mut *self.imp().operations.borrow_mut());
            for (_, cancel) in operations {
                cancel.cancel();
            }
        }
        let mut seen = HashSet::new();
        let mut watchers = Vec::new();
        for object in std::iter::once(client.object())
            .chain(client.devices())
            .chain(client.active())
            .chain(client.all_active())
            .chain(client.connections())
            .chain(client.access_points())
        {
            if !seen.insert(object.as_ptr() as usize) {
                continue;
            }
            let weak = self.downgrade();
            let handler = object.connect_notify_local(None, move |_, _| {
                if let Some(service) = weak.upgrade() {
                    service.refresh();
                }
            });
            watchers.push(Subscription {
                object,
                handler: Some(handler),
            });
        }
        for object in client.connections() {
            let weak = self.downgrade();
            let handler = object.connect_local("changed", false, move |_| {
                if let Some(service) = weak.upgrade() {
                    service.refresh();
                }
                None
            });
            watchers.push(Subscription {
                object,
                handler: Some(handler),
            });
        }
        for signal in [
            "device-added",
            "device-removed",
            "connection-added",
            "connection-removed",
            "active-connection-added",
            "active-connection-removed",
        ] {
            let object = client.object();
            let weak = self.downgrade();
            let handler = object.connect_local(signal, false, move |_| {
                if let Some(service) = weak.upgrade() {
                    service.refresh();
                }
                None
            });
            watchers.push(Subscription {
                object,
                handler: Some(handler),
            });
        }
        if let Some(connection) = client.connection() {
            let weak = self.downgrade();
            let handler = connection.connect_local("closed", false, move |_| {
                if let Some(service) = weak.upgrade() {
                    service.stop();
                    if service.imp().connection.borrow().is_none() {
                        let weak = service.downgrade();
                        let retry =
                            glib::MainContext::ref_thread_default().spawn_local(async move {
                                glib::timeout_future(Duration::from_secs(1)).await;
                                if let Some(service) = weak.upgrade() {
                                    service.imp().retry.borrow_mut().take();
                                    service.initialize();
                                }
                            });
                        service.imp().retry.replace(Some(retry));
                    }
                }
                None
            });
            watchers.push(Subscription {
                object: connection.upcast(),
                handler: Some(handler),
            });
        }
        self.imp().watchers.replace(watchers);
        self.update(snapshot);
    }
    fn update(&self, state: NetworkState) {
        if *self.imp().state.borrow() != state {
            self.imp().state.replace(state);
            self.emit_by_name::<()>("changed", &[]);
        }
    }
    pub fn set_wireless(
        &self,
        enabled: bool,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) -> gio::Cancellable {
        self.operation(
            "org.freedesktop.DBus.Properties",
            "Set",
            &(
                "org.freedesktop.NetworkManager",
                "WirelessEnabled",
                enabled.to_variant(),
            )
                .to_variant(),
            done,
        )
    }
    pub fn set_networking(
        &self,
        enabled: bool,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) -> gio::Cancellable {
        self.operation(
            "org.freedesktop.NetworkManager",
            "Enable",
            &(enabled,).to_variant(),
            done,
        )
    }
    fn operation(
        &self,
        interface: &'static str,
        method: &'static str,
        parameters: &glib::Variant,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) -> gio::Cancellable {
        self.execute(
            Ok(vec![actions::Call {
                path: "/org/freedesktop/NetworkManager".into(),
                interface,
                method,
                parameters: parameters.clone(),
                reply: "()",
            }]),
            done,
        )
    }
}
