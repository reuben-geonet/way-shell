//! Status notifier watcher, registration ownership, and item inventory.
//!
//! Item identity is the resolved unique bus owner plus object path. Requested
//! well-known aliases remain private so replacing an owner cannot route actions
//! to a different process through an old item handle.
use gio::prelude::*;
use glib::subclass::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    sync::OnceLock,
    time::Duration,
};
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const HOST: &str = "org.kde.StatusNotifierHost";
const ITEM: &str = "org.kde.StatusNotifierItem";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";
const DEADLINE_MS: i32 = 2_000;
const WATCHER_XML: &str =
    include_str!("../../../../data/dbus-interfaces/org.kde.StatusNotifierWatcher.xml");
const HOST_XML: &str =
    include_str!("../../../../data/dbus-interfaces/org.kde.StatusNotifierHost.xml");

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ItemKey {
    pub owner: String,
    pub path: String,
}
impl ItemKey {
    /// Unambiguous identifier used by the watcher's public item inventory.
    pub fn registration(&self) -> String {
        format!("{}{}", self.owner, self.path)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Icon {
    pub name: String,
    pub pixmap: Option<RgbaImage>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tooltip {
    pub icon: Icon,
    pub title: String,
    pub description: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayItem {
    pub key: ItemKey,
    pub category: String,
    pub id: String,
    pub title: String,
    pub status: String,
    pub window_id: u32,
    pub icon_theme_path: String,
    pub icon: Icon,
    pub overlay: Icon,
    pub attention: Icon,
    pub attention_movie_name: String,
    pub tooltip: Tooltip,
    pub item_is_menu: bool,
    /// A D-Bus menu endpoint on `key.owner`; the root path means no menu.
    pub menu_path: Option<String>,
    pub label: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayState {
    pub available: bool,
    pub items: Vec<TrayItem>,
}
#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "WayShellTrayEvent")]
pub enum TrayEvent {
    Added(TrayItem),
    Changed(TrayItem),
    Removed(TrayItem),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemCommand {
    Activate {
        x: i32,
        y: i32,
    },
    SecondaryActivate {
        x: i32,
        y: i32,
    },
    ContextMenu {
        x: i32,
        y: i32,
    },
    Scroll {
        delta: i32,
        orientation: Orientation,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Alias {
    name: String,
    path: String,
}
struct Registration {
    owner: Option<String>,
    serial: u64,
}
struct Entry {
    key: ItemKey,
    item: Option<TrayItem>,
    revision: u64,
    refresh: Option<gio::Cancellable>,
    retry: Option<glib::JoinHandle<()>>,
    lifetime: gio::Cancellable,
}
impl Drop for Entry {
    fn drop(&mut self) {
        self.lifetime.cancel();
        if let Some(cancel) = self.refresh.take() {
            cancel.cancel();
        }
        if let Some(task) = self.retry.take() {
            task.abort();
        }
    }
}
struct Connection {
    bus: gio::DBusConnection,
    registrations: Vec<gio::RegistrationId>,
    _subscriptions: Vec<gio::SignalSubscription>,
    closed: Option<glib::SignalHandlerId>,
    cancellable: gio::Cancellable,
    host_name: String,
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.cancellable.cancel();
        if let Some(handler) = self.closed.take() {
            self.bus.disconnect(handler);
        }
        for registration in self.registrations.drain(..) {
            let _ = self.bus.unregister_object(registration);
        }
        if !self.bus.is_closed() {
            for name in [WATCHER, &self.host_name] {
                self.bus.call(
                    Some(BUS),
                    BUS_PATH,
                    BUS,
                    "ReleaseName",
                    Some(&(name,).to_variant()),
                    Some(glib::VariantTy::new("(u)").unwrap()),
                    gio::DBusCallFlags::NONE,
                    DEADLINE_MS,
                    gio::Cancellable::NONE,
                    |_| {},
                );
            }
        }
    }
}
mod imp {
    use super::*;
    #[derive(Default)]
    pub struct TrayService {
        pub state: RefCell<TrayState>,
        pub running: Cell<bool>,
        pub owned: Cell<bool>,
        pub generation: Cell<u64>,
        pub serial: Cell<u64>,
        pub source: RefCell<Option<gio::DBusConnection>>,
        pub(super) connection: RefCell<Option<Connection>>,
        pub(super) aliases: RefCell<HashMap<Alias, Registration>>,
        pub(super) entries: RefCell<Vec<Entry>>,
        pub hosts: RefCell<HashSet<String>>,
        pub connecting: RefCell<Option<gio::Cancellable>>,
        pub connect_deadline: RefCell<Option<glib::JoinHandle<()>>>,
        pub retry: RefCell<Option<glib::JoinHandle<()>>>,
        pub events: RefCell<VecDeque<TrayEvent>>,
        pub publishing: Cell<bool>,
        pub changed_pending: Cell<bool>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for TrayService {
        const NAME: &'static str = "WayShellTrayService";
        type Type = super::TrayService;
    }
    impl ObjectImpl for TrayService {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("changed").build(),
                    glib::subclass::Signal::builder("event")
                        .param_types([TrayEvent::static_type()])
                        .build(),
                ]
            })
        }
        fn dispose(&self) {
            self.obj().stop();
        }
    }
}
glib::wrapper! { pub struct TrayService(ObjectSubclass<imp::TrayService>); }
impl Default for TrayService {
    fn default() -> Self {
        Self::new()
    }
}
impl TrayService {
    pub fn new() -> Self {
        let service: Self = glib::Object::new();
        service.start();
        service
    }
    /// The supplied connection must belong to this GLib main context.
    pub fn on_connection(connection: &gio::DBusConnection) -> Self {
        let service: Self = glib::Object::new();
        service.imp().source.replace(Some(connection.clone()));
        service.start();
        service
    }
    pub fn state(&self) -> TrayState {
        self.imp().state.borrow().clone()
    }
    pub fn start(&self) {
        if !self.imp().running.replace(true) {
            self.connect();
        }
    }
    pub fn stop(&self) {
        self.imp().running.set(false);
        self.detach();
    }
    pub fn command(
        &self,
        key: &ItemKey,
        command: ItemCommand,
        callback: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) {
        let target = self
            .imp()
            .entries
            .borrow()
            .iter()
            .find(|entry| &entry.key == key && entry.item.is_some())
            .map(|entry| entry.lifetime.clone());
        let connection = self.connection();
        let (Some(cancellable), Some(connection)) = (target, connection) else {
            callback(Err(glib::Error::new(
                gio::IOErrorEnum::NotConnected,
                "Tray item is no longer available",
            )));
            return;
        };
        let (method, parameters) = match command {
            ItemCommand::Activate { x, y } => ("Activate", (x, y).to_variant()),
            ItemCommand::SecondaryActivate { x, y } => ("SecondaryActivate", (x, y).to_variant()),
            ItemCommand::ContextMenu { x, y } => ("ContextMenu", (x, y).to_variant()),
            ItemCommand::Scroll { delta, orientation } => (
                "Scroll",
                (
                    delta,
                    match orientation {
                        Orientation::Horizontal => "horizontal",
                        Orientation::Vertical => "vertical",
                    },
                )
                    .to_variant(),
            ),
        };
        connection.call(
            Some(&key.owner),
            &key.path,
            ITEM,
            method,
            Some(&parameters),
            Some(glib::VariantTy::new("()").unwrap()),
            gio::DBusCallFlags::NO_AUTO_START,
            DEADLINE_MS,
            Some(&cancellable),
            move |result| callback(result.map(|_| ())),
        );
    }
    fn connection(&self) -> Option<gio::DBusConnection> {
        self.imp()
            .connection
            .borrow()
            .as_ref()
            .map(|connection| connection.bus.clone())
    }
    fn current(&self, generation: u64) -> bool {
        self.imp().running.get() && self.imp().generation.get() == generation
    }
    fn connect(&self) {
        if let Some(connection) = self.imp().source.borrow().clone() {
            if !connection.is_closed() {
                self.attach(&connection);
            }
            return;
        }
        let cancellable = gio::Cancellable::new();
        self.imp().connecting.replace(Some(cancellable.clone()));
        let cancel = cancellable.clone();
        self.imp().connect_deadline.replace(Some(
            glib::MainContext::ref_thread_default().spawn_local(async move {
                glib::timeout_future(Duration::from_millis(DEADLINE_MS as u64)).await;
                cancel.cancel();
            }),
        ));
        let generation = self.imp().generation.get();
        let weak = self.downgrade();
        gio::bus_get(gio::BusType::Session, Some(&cancellable), move |result| {
            let Some(service) = weak.upgrade().filter(|service| service.current(generation)) else {
                return;
            };
            service.imp().connecting.borrow_mut().take();
            if let Some(task) = service.imp().connect_deadline.borrow_mut().take() {
                task.abort();
            }
            match result {
                Ok(connection) => service.attach(&connection),
                Err(error) => service.failed(&error),
            }
        });
    }
    fn attach(&self, connection: &gio::DBusConnection) {
        self.detach();
        connection.set_exit_on_close(false);
        let info =
            gio::DBusNodeInfo::for_xml(WATCHER_XML).expect("Tracked watcher interface is valid");
        let weak = self.downgrade();
        let properties = self.downgrade();
        let registration = connection
            .register_object(WATCHER_PATH, &info.lookup_interface(WATCHER).unwrap())
            .method_call(move |_, sender, _, _, method, parameters, invocation| {
                let Some(service) = weak.upgrade().filter(|service| service.imp().owned.get())
                else {
                    invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.Disconnected",
                        "Tray watcher is unavailable",
                    );
                    return;
                };
                service.method(sender, method, &parameters, invocation);
            })
            .property(move |_, _, _, _, property| {
                let service = properties.upgrade();
                match property {
                    "RegisteredStatusNotifierItems" => service
                        .map(|service| service.registered())
                        .unwrap_or_default()
                        .to_variant(),
                    "IsStatusNotifierHostRegistered" => service
                        .is_some_and(|service| service.imp().owned.get())
                        .to_variant(),
                    "ProtocolVersion" => 0_i32.to_variant(),
                    _ => unreachable!("Tracked watcher property"),
                }
            })
            .build();
        let registration = match registration {
            Ok(value) => value,
            Err(error) => {
                self.failed(&error);
                return;
            }
        };
        let host_name = format!("org.kde.StatusNotifierHost-{}", std::process::id());
        let host_path = format!("/StatusNotifierHost/{}", std::process::id());
        let host_info =
            gio::DBusNodeInfo::for_xml(HOST_XML).expect("Tracked host interface is valid");
        let host_registration = connection
            .register_object(&host_path, &host_info.lookup_interface(HOST).unwrap())
            .build();
        let host_registration = match host_registration {
            Ok(value) => value,
            Err(error) => {
                let _ = connection.unregister_object(registration);
                self.failed(&error);
                return;
            }
        };
        let generation = self.imp().generation.get();
        let weak = self.downgrade();
        let names = connection.subscribe_to_signal(
            Some(BUS),
            Some(BUS),
            Some("NameOwnerChanged"),
            Some(BUS_PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let Some(service) = weak.upgrade().filter(|service| service.current(generation))
                else {
                    return;
                };
                if let Some((name, old, owner)) =
                    signal.parameters.get::<(String, String, String)>()
                {
                    if name == WATCHER {
                        service.set_owned(
                            signal.connection.unique_name().as_deref() == Some(owner.as_str()),
                        );
                    } else {
                        service.name_changed(&name, &old, &owner);
                    }
                }
            },
        );
        let mut subscriptions = vec![names];
        for interface in [ITEM, PROPERTIES] {
            let weak = self.downgrade();
            subscriptions.push(connection.subscribe_to_signal(
                None,
                Some(interface),
                None,
                None,
                None,
                gio::DBusSignalFlags::NONE,
                move |signal| {
                    let Some(service) =
                        weak.upgrade().filter(|service| service.current(generation))
                    else {
                        return;
                    };
                    if signal.interface_name == PROPERTIES
                        && (signal.signal_name != "PropertiesChanged"
                            || signal
                                .parameters
                                .try_child_get::<String>(0)
                                .ok()
                                .flatten()
                                .as_deref()
                                != Some(ITEM))
                    {
                        return;
                    }
                    if signal.interface_name == ITEM
                        && !matches!(
                            signal.signal_name,
                            "NewTitle"
                                | "NewIcon"
                                | "NewAttentionIcon"
                                | "NewOverlayIcon"
                                | "NewStatus"
                                | "NewToolTip"
                                | "NewIconThemePath"
                                | "XAyatanaNewLabel"
                        )
                    {
                        return;
                    }
                    let key = ItemKey {
                        owner: signal.sender_name.to_owned(),
                        path: signal.object_path.to_owned(),
                    };
                    service.refresh(&key);
                },
            ));
        }
        let weak = self.downgrade();
        let closed = connection.connect_local("closed", false, move |_| {
            if let Some(service) = weak.upgrade().filter(|service| service.current(generation)) {
                service.detach();
                glib::g_message!(
                    "way-shell",
                    "Tray session bus disconnected; waiting for recovery"
                );
                if service.imp().source.borrow().is_none() {
                    service.retry();
                }
            }
            None
        });
        let cancellable = gio::Cancellable::new();
        self.imp().connection.replace(Some(Connection {
            bus: connection.clone(),
            registrations: vec![registration, host_registration],
            _subscriptions: subscriptions,
            closed: Some(closed),
            cancellable: cancellable.clone(),
            host_name: host_name.clone(),
        }));
        for name in [host_name.as_str(), WATCHER] {
            let watcher = name == WATCHER;
            let weak = self.downgrade();
            connection.call(Some(BUS), BUS_PATH, BUS, "RequestName", Some(&(name, 0_u32).to_variant()),
                Some(glib::VariantTy::new("(u)").unwrap()), gio::DBusCallFlags::NONE, DEADLINE_MS, Some(&cancellable), move |result| {
                    let Some(service) = weak.upgrade().filter(|service| service.current(generation)) else { return; };
                    match result {
                        Ok(reply) if matches!(reply.get::<(u32,)>(), Some((1 | 4,))) => { if watcher { service.set_owned(true); } }
                        Ok(reply) if matches!(reply.get::<(u32,)>(), Some((2 | 3,))) => {
                            if watcher && !service.imp().owned.get() { glib::g_message!("way-shell", "Another status notifier watcher is active; waiting for its name"); }
                        }
                        Ok(_) => service.failed(&glib::Error::new(gio::IOErrorEnum::InvalidData, "Invalid tray name ownership reply")),
                        Err(error) => service.failed(&error),
                    }
                });
        }
    }
    fn set_owned(&self, owned: bool) {
        if self.imp().owned.replace(owned) == owned {
            return;
        }
        if owned {
            self.emit_bus("StatusNotifierHostRegistered", &().to_variant());
            self.registry_changed();
        } else {
            self.clear_items();
        }
        self.publish(None);
    }
    fn failed(&self, error: &glib::Error) {
        glib::g_message!("way-shell", "Tray unavailable: {error}; retrying");
        self.detach();
        if self.imp().running.get() {
            self.retry();
        }
    }
    fn retry(&self) {
        if self.imp().retry.borrow().is_some() {
            return;
        }
        let weak = self.downgrade();
        self.imp()
            .retry
            .replace(Some(glib::MainContext::ref_thread_default().spawn_local(
                async move {
                    glib::timeout_future(Duration::from_secs(1)).await;
                    if let Some(service) = weak.upgrade() {
                        service.imp().retry.borrow_mut().take();
                        if service.imp().running.get() {
                            service.connect();
                        }
                    }
                },
            )));
    }
    fn detach(&self) {
        self.imp()
            .generation
            .set(self.imp().generation.get().wrapping_add(1));
        if let Some(cancel) = self.imp().connecting.borrow_mut().take() {
            cancel.cancel();
        }
        if let Some(task) = self.imp().connect_deadline.borrow_mut().take() {
            task.abort();
        }
        if let Some(task) = self.imp().retry.borrow_mut().take() {
            task.abort();
        }
        self.imp().connection.borrow_mut().take();
        self.imp().owned.set(false);
        self.clear_items();
        self.publish(None);
    }
    fn clear_items(&self) {
        self.imp().aliases.borrow_mut().clear();
        self.imp().hosts.borrow_mut().clear();
        let entries = self.imp().entries.replace(Vec::new());
        for entry in entries {
            let item = entry.item.clone();
            drop(entry);
            if let Some(item) = item {
                self.publish(Some(TrayEvent::Removed(item)));
            }
        }
    }
    fn method(
        &self,
        sender: Option<&str>,
        method: &str,
        parameters: &glib::Variant,
        invocation: gio::DBusMethodInvocation,
    ) {
        let Some((name,)) = parameters.get::<(String,)>() else {
            invocation.return_dbus_error(
                "org.freedesktop.DBus.Error.InvalidArgs",
                "Expected a service name or object path",
            );
            return;
        };
        if method == "RegisterStatusNotifierHost" {
            if !gio::dbus_is_name(&name) {
                invocation.return_dbus_error(
                    "org.freedesktop.DBus.Error.InvalidArgs",
                    "Invalid host bus name",
                );
                return;
            }
            self.register_host(name, invocation);
            return;
        }
        if method != "RegisterStatusNotifierItem" {
            invocation.return_dbus_error(
                "org.freedesktop.DBus.Error.UnknownMethod",
                "Unknown watcher method",
            );
            return;
        }
        let alias = if name.starts_with('/') && glib::Variant::is_object_path(&name) {
            let Some(sender) = sender else {
                invocation.return_dbus_error(
                    "org.freedesktop.DBus.Error.InvalidArgs",
                    "Object registration requires a sender",
                );
                return;
            };
            Alias {
                name: sender.into(),
                path: name,
            }
        } else if gio::dbus_is_name(&name) {
            Alias {
                name,
                path: "/StatusNotifierItem".into(),
            }
        } else {
            invocation.return_dbus_error(
                "org.freedesktop.DBus.Error.InvalidArgs",
                "Invalid tray service name or object path",
            );
            return;
        };
        if self
            .imp()
            .aliases
            .borrow()
            .get(&alias)
            .is_some_and(|registration| registration.owner.is_some())
        {
            invocation.return_value(Some(&().to_variant()));
            return;
        }
        // Concurrent requests for one unresolved alias share its generation.
        // Resolving the first request must not invalidate the other replies.
        let pending = self
            .imp()
            .aliases
            .borrow()
            .get(&alias)
            .map(|entry| entry.serial);
        let serial = pending.unwrap_or_else(|| {
            let serial = self.next_serial();
            self.imp().aliases.borrow_mut().insert(
                alias.clone(),
                Registration {
                    owner: None,
                    serial,
                },
            );
            serial
        });
        let generation = self.imp().generation.get();
        let weak = self.downgrade();
        self.resolve(&alias.name.clone(), move |result| {
            let Some(service) = weak
                .upgrade()
                .filter(|service| service.current(generation) && service.imp().owned.get())
            else {
                invocation.return_dbus_error(
                    "org.freedesktop.DBus.Error.Disconnected",
                    "Tray watcher stopped",
                );
                return;
            };
            let current = service
                .imp()
                .aliases
                .borrow()
                .get(&alias)
                .map(|registration| (registration.serial, registration.owner.clone()));
            let Some((current_serial, owner)) = current else {
                invocation.return_dbus_error(
                    "org.freedesktop.DBus.Error.NameHasNoOwner",
                    "Tray owner vanished during registration",
                );
                return;
            };
            if current_serial != serial {
                if owner.is_some() {
                    invocation.return_value(Some(&().to_variant()));
                } else {
                    invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.NameHasNoOwner",
                        "Tray owner vanished during registration",
                    );
                }
                return;
            }
            match result {
                Ok(owner) => {
                    service.assign_alias(&alias, Some(owner));
                    invocation.return_value(Some(&().to_variant()));
                }
                Err(error) => {
                    service.imp().aliases.borrow_mut().remove(&alias);
                    invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.NameHasNoOwner",
                        error.message(),
                    );
                }
            }
        });
    }
    fn register_host(&self, name: String, invocation: gio::DBusMethodInvocation) {
        let weak = self.downgrade();
        let generation = self.imp().generation.get();
        self.resolve(&name, move |result| {
            let Some(service) = weak
                .upgrade()
                .filter(|service| service.current(generation) && service.imp().owned.get())
            else {
                invocation.return_dbus_error(
                    "org.freedesktop.DBus.Error.Disconnected",
                    "Tray watcher stopped",
                );
                return;
            };
            match result {
                Ok(owner) => {
                    let added = service.imp().hosts.borrow_mut().insert(owner);
                    invocation.return_value(Some(&().to_variant()));
                    if added {
                        service.emit_bus("StatusNotifierHostRegistered", &().to_variant());
                    }
                }
                Err(error) => invocation.return_dbus_error(
                    "org.freedesktop.DBus.Error.NameHasNoOwner",
                    error.message(),
                ),
            }
        });
    }
    fn resolve(&self, name: &str, callback: impl FnOnce(Result<String, glib::Error>) + 'static) {
        let source = self.imp().connection.borrow();
        let Some(source) = source.as_ref() else {
            callback(Err(glib::Error::new(
                gio::IOErrorEnum::NotConnected,
                "Session bus unavailable",
            )));
            return;
        };
        source.bus.call(
            Some(BUS),
            BUS_PATH,
            BUS,
            "GetNameOwner",
            Some(&(name,).to_variant()),
            Some(glib::VariantTy::new("(s)").unwrap()),
            gio::DBusCallFlags::NONE,
            DEADLINE_MS,
            Some(&source.cancellable),
            move |result| {
                callback(result.and_then(|reply| {
                    reply
                        .get::<(String,)>()
                        .map(|(owner,)| owner)
                        .filter(|owner| gio::dbus_is_unique_name(owner))
                        .ok_or_else(|| {
                            glib::Error::new(
                                gio::IOErrorEnum::InvalidData,
                                "Invalid tray owner reply",
                            )
                        })
                }));
            },
        );
    }
    fn next_serial(&self) -> u64 {
        let serial = self.imp().serial.get().wrapping_add(1);
        self.imp().serial.set(serial);
        serial
    }
    fn name_changed(&self, name: &str, _old: &str, owner: &str) {
        if !self.imp().owned.get() {
            return;
        }
        if owner.is_empty() {
            self.imp().hosts.borrow_mut().remove(name);
        }
        let aliases: Vec<_> = self
            .imp()
            .aliases
            .borrow()
            .iter()
            .filter(|(alias, registration)| {
                alias.name == name
                    || (owner.is_empty() && registration.owner.as_deref() == Some(name))
            })
            .map(|(alias, _)| alias.clone())
            .collect();
        for alias in aliases {
            let next = if alias.name == name && !owner.is_empty() {
                Some(owner.into())
            } else {
                None
            };
            self.assign_alias(&alias, next);
            if owner.is_empty() && gio::dbus_is_unique_name(&alias.name) {
                self.imp().aliases.borrow_mut().remove(&alias);
            }
        }
    }
    fn assign_alias(&self, alias: &Alias, owner: Option<String>) {
        if !self.imp().running.get() || !self.imp().owned.get() {
            return;
        }
        let generation = self.imp().generation.get();
        let serial = self.next_serial();
        self.imp().aliases.borrow_mut().insert(
            alias.clone(),
            Registration {
                owner: owner.clone(),
                serial,
            },
        );
        self.prune_entries();
        // A removed-item observer may stop or restart the service.
        if !self.current(generation) || !self.imp().owned.get() {
            return;
        }
        let Some(owner) = owner else {
            return;
        };
        let key = ItemKey {
            owner,
            path: alias.path.clone(),
        };
        if self
            .imp()
            .entries
            .borrow()
            .iter()
            .any(|entry| entry.key == key)
        {
            return;
        }
        self.imp().entries.borrow_mut().push(Entry {
            key: key.clone(),
            item: None,
            revision: 0,
            refresh: None,
            retry: None,
            lifetime: gio::Cancellable::new(),
        });
        self.emit_bus(
            "StatusNotifierItemRegistered",
            &(key.registration(),).to_variant(),
        );
        self.registry_changed();
        self.refresh(&key);
    }
    fn prune_entries(&self) {
        let live: HashSet<_> = self
            .imp()
            .aliases
            .borrow()
            .iter()
            .filter_map(|(alias, registration)| {
                registration.owner.as_ref().map(|owner| ItemKey {
                    owner: owner.clone(),
                    path: alias.path.clone(),
                })
            })
            .collect();
        let mut removed = Vec::new();
        {
            let mut entries = self.imp().entries.borrow_mut();
            let mut index = 0;
            while index < entries.len() {
                if live.contains(&entries[index].key) {
                    index += 1;
                } else {
                    removed.push(entries.remove(index));
                }
            }
        }
        for entry in removed {
            let key = entry.key.clone();
            let item = entry.item.clone();
            drop(entry);
            self.emit_bus(
                "StatusNotifierItemUnregistered",
                &(key.registration(),).to_variant(),
            );
            self.registry_changed();
            if let Some(item) = item {
                self.publish(Some(TrayEvent::Removed(item)));
            }
        }
    }
    fn registered(&self) -> Vec<String> {
        self.imp()
            .entries
            .borrow()
            .iter()
            .map(|entry| entry.key.registration())
            .collect()
    }
    fn emit_bus(&self, name: &str, parameters: &glib::Variant) {
        if !self.imp().owned.get() {
            return;
        }
        if let Some(connection) = self.connection()
            && let Err(error) =
                connection.emit_signal(None, WATCHER_PATH, WATCHER, name, Some(parameters))
        {
            glib::g_message!("way-shell", "Cannot emit tray {name}: {error}");
        }
    }
    fn registry_changed(&self) {
        if let Some(connection) = self.connection() {
            let values = HashMap::from([
                (
                    "RegisteredStatusNotifierItems",
                    self.registered().to_variant(),
                ),
                (
                    "IsStatusNotifierHostRegistered",
                    self.imp().owned.get().to_variant(),
                ),
            ]);
            if let Err(error) = connection.emit_signal(
                None,
                WATCHER_PATH,
                PROPERTIES,
                "PropertiesChanged",
                Some(&(WATCHER, values, Vec::<String>::new()).to_variant()),
            ) {
                glib::g_message!("way-shell", "Cannot announce tray inventory: {error}");
            }
        }
    }
    fn refresh(&self, key: &ItemKey) {
        let Some(connection) = self.connection() else {
            return;
        };
        let (revision, cancellable) = {
            let mut entries = self.imp().entries.borrow_mut();
            let Some(entry) = entries.iter_mut().find(|entry| &entry.key == key) else {
                return;
            };
            if let Some(cancel) = entry.refresh.take() {
                cancel.cancel();
            }
            if let Some(task) = entry.retry.take() {
                task.abort();
            }
            entry.revision = entry.revision.wrapping_add(1);
            let cancel = gio::Cancellable::new();
            entry.refresh = Some(cancel.clone());
            (entry.revision, cancel)
        };
        let generation = self.imp().generation.get();
        let weak = self.downgrade();
        let item_key = key.clone();
        connection.call(
            Some(&key.owner),
            &key.path,
            PROPERTIES,
            "GetAll",
            Some(&(ITEM,).to_variant()),
            Some(glib::VariantTy::new("(a{sv})").unwrap()),
            gio::DBusCallFlags::NO_AUTO_START,
            DEADLINE_MS,
            Some(&cancellable),
            move |result| {
                let Some(service) = weak.upgrade().filter(|service| service.current(generation))
                else {
                    return;
                };
                if !service
                    .imp()
                    .entries
                    .borrow()
                    .iter()
                    .any(|entry| entry.key == item_key && entry.revision == revision)
                {
                    return;
                }
                let parsed = result.and_then(|reply| {
                    reply
                        .get::<(HashMap<String, glib::Variant>,)>()
                        .and_then(|(properties,)| parse_item(item_key.clone(), &properties))
                        .ok_or_else(|| {
                            glib::Error::new(
                                gio::IOErrorEnum::InvalidData,
                                "Tray item has not exported valid Id and Status properties",
                            )
                        })
                });
                match parsed {
                    Ok(item) => {
                        let event = {
                            let mut entries = service.imp().entries.borrow_mut();
                            let entry = entries
                                .iter_mut()
                                .find(|entry| entry.key == item_key)
                                .unwrap();
                            entry.refresh.take();
                            let event = match entry.item.as_ref() {
                                None => Some(TrayEvent::Added(item.clone())),
                                Some(old) if old != &item => Some(TrayEvent::Changed(item.clone())),
                                Some(_) => None,
                            };
                            entry.item = Some(item);
                            event
                        };
                        if event.is_some() {
                            service.publish(event);
                        }
                    }
                    Err(error) => {
                        glib::g_message!(
                            "way-shell",
                            "Cannot read tray item {}: {error}; retrying",
                            item_key.registration()
                        );
                        service.retry_item(&item_key);
                    }
                }
            },
        );
    }
    fn retry_item(&self, key: &ItemKey) {
        let weak = self.downgrade();
        let next = key.clone();
        let task = glib::MainContext::ref_thread_default().spawn_local(async move {
            glib::timeout_future(Duration::from_secs(1)).await;
            if let Some(service) = weak.upgrade() {
                if let Some(entry) = service
                    .imp()
                    .entries
                    .borrow_mut()
                    .iter_mut()
                    .find(|entry| entry.key == next)
                {
                    entry.retry.take();
                }
                service.refresh(&next);
            }
        });
        if let Some(entry) = self
            .imp()
            .entries
            .borrow_mut()
            .iter_mut()
            .find(|entry| &entry.key == key)
        {
            entry.refresh.take();
            if let Some(old) = entry.retry.replace(task) {
                old.abort();
            }
        } else {
            task.abort();
        }
    }
    fn publish(&self, event: Option<TrayEvent>) {
        let state = TrayState {
            available: self.imp().owned.get(),
            items: self
                .imp()
                .entries
                .borrow()
                .iter()
                .filter_map(|entry| entry.item.clone())
                .collect(),
        };
        if *self.imp().state.borrow() != state {
            self.imp().state.replace(state);
            self.imp().changed_pending.set(true);
        }
        if let Some(event) = event {
            self.imp().events.borrow_mut().push_back(event);
        }
        if self.imp().publishing.replace(true) {
            return;
        }
        loop {
            loop {
                let event = self.imp().events.borrow_mut().pop_front();
                let Some(event) = event else {
                    break;
                };
                self.emit_by_name::<()>("event", &[&event]);
            }
            if self.imp().changed_pending.replace(false) {
                self.emit_by_name::<()>("changed", &[]);
            }
            if self.imp().events.borrow().is_empty() && !self.imp().changed_pending.get() {
                break;
            }
        }
        self.imp().publishing.set(false);
    }
}

fn parse_item(key: ItemKey, properties: &HashMap<String, glib::Variant>) -> Option<TrayItem> {
    let string = |name| {
        properties
            .get(name)
            .and_then(glib::Variant::str)
            .unwrap_or_default()
            .to_owned()
    };
    let icon = |name, pixmap| Icon {
        name: string(name),
        pixmap: properties.get(pixmap).and_then(parse_pixmaps),
    };
    let tooltip = properties
        .get("ToolTip")
        .and_then(|value| value.get::<(String, Vec<(i32, i32, Vec<u8>)>, String, String)>())
        .map(|(name, pixels, title, description)| Tooltip {
            icon: Icon {
                name,
                pixmap: parse_pixmap_values(pixels),
            },
            title,
            description,
        })
        .unwrap_or_default();
    let menu_path = properties
        .get("Menu")
        .and_then(glib::Variant::str)
        .filter(|path| *path != "/" && glib::Variant::is_object_path(path))
        .map(str::to_owned);
    Some(TrayItem {
        key,
        id: properties.get("Id")?.get::<String>()?,
        status: properties.get("Status")?.get::<String>()?,
        category: string("Category"),
        title: string("Title"),
        window_id: properties
            .get("WindowId")
            .and_then(|value| {
                value
                    .get::<u32>()
                    .or_else(|| value.get::<i32>().map(|id| id as u32))
            })
            .unwrap_or_default(),
        icon_theme_path: string("IconThemePath"),
        icon: icon("IconName", "IconPixmap"),
        overlay: icon("OverlayIconName", "OverlayIconPixmap"),
        attention: icon("AttentionIconName", "AttentionIconPixmap"),
        attention_movie_name: string("AttentionMovieName"),
        tooltip,
        item_is_menu: properties
            .get("ItemIsMenu")
            .and_then(glib::Variant::get)
            .unwrap_or_default(),
        menu_path,
        label: string("XAyatanaLabel"),
    })
}
fn parse_pixmaps(value: &glib::Variant) -> Option<RgbaImage> {
    parse_pixmap_values(value.get::<Vec<(i32, i32, Vec<u8>)>>()?)
}
fn parse_pixmap_values(values: Vec<(i32, i32, Vec<u8>)>) -> Option<RgbaImage> {
    values
        .into_iter()
        .filter_map(|(width, height, mut pixels)| {
            let width = u32::try_from(width).ok().filter(|value| *value > 0)?;
            let height = u32::try_from(height).ok().filter(|value| *value > 0)?;
            let size = (width as usize)
                .checked_mul(height as usize)?
                .checked_mul(4)?;
            if pixels.len() != size {
                return None;
            }
            for pixel in pixels.as_chunks_mut::<4>().0 {
                pixel.rotate_left(1);
            }
            Some(RgbaImage {
                width,
                height,
                pixels,
            })
        })
        .reduce(|largest, image| {
            if image.pixels.len() > largest.pixels.len() {
                image
            } else {
                largest
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn images_validate_dimensions_storage_and_keep_largest_argb_variant() {
        let values: Vec<(i32, i32, Vec<u8>)> = vec![
            (-1, 2, vec![0; 8]),
            (i32::MAX, i32::MAX, vec![0; 4]),
            (0, 1, vec![]),
            (1, 0, vec![]),
            (2, 2, vec![0; 15]),
            (1, 1, vec![255, 1, 2, 3]),
            (2, 1, vec![0x80, 0x11, 0x22, 0x33, 0x40, 0x44, 0x55, 0x66]),
        ];
        let image = parse_pixmaps(&values.to_variant()).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(
            image.pixels,
            [0x11, 0x22, 0x33, 0x80, 0x44, 0x55, 0x66, 0x40]
        );
        assert!(parse_pixmaps(&"malformed".to_variant()).is_none());
        assert!(parse_pixmaps(&vec![(1, 1, vec![0_u8; 5])].to_variant()).is_none());
    }
    #[test]
    fn metadata_keeps_independent_icons_and_clears_invalid_optional_properties() {
        let mut properties = HashMap::from([
            ("Id".into(), "".to_variant()),
            ("Status".into(), "Active".to_variant()),
            ("IconName".into(), "primary".to_variant()),
            ("OverlayIconName".into(), "overlay".to_variant()),
            ("AttentionIconName".into(), "attention".to_variant()),
            ("IconThemePath".into(), "/icons".to_variant()),
            ("WindowId".into(), (-1_i32).to_variant()),
            ("ItemIsMenu".into(), true.to_variant()),
            (
                "ToolTip".into(),
                (
                    "tooltip",
                    Vec::<(i32, i32, Vec<u8>)>::new(),
                    "Title",
                    "Description",
                )
                    .to_variant(),
            ),
        ]);
        let key = ItemKey {
            owner: ":1.42".into(),
            path: "/Item".into(),
        };
        let item = parse_item(key.clone(), &properties).unwrap();
        assert_eq!(
            (
                item.icon.name.as_str(),
                item.overlay.name.as_str(),
                item.attention.name.as_str()
            ),
            ("primary", "overlay", "attention")
        );
        assert_eq!(item.window_id, u32::MAX);
        assert_eq!(item.tooltip.title, "Title");
        assert!(item.item_is_menu);
        properties.insert("ToolTip".into(), 42_i32.to_variant());
        properties.insert("ItemIsMenu".into(), "invalid".to_variant());
        let item = parse_item(key, &properties).unwrap();
        assert_eq!(item.tooltip, Tooltip::default());
        assert!(!item.item_is_menu);
    }
}
