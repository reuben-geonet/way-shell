//! BlueZ inventory and serialized Bluetooth operations on the default GLib context.
mod radio;
pub mod settings;

use crate::platform::bus_watch::Watcher;
use gio::prelude::*;
use glib::subclass::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    os::fd::OwnedFd,
    rc::Rc,
    sync::OnceLock,
    time::{Duration, Instant},
};

const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const POWER_TIMEOUT: Duration = Duration::from_secs(5);
const DEVICE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BluetoothDevice {
    pub path: String,
    pub alias: String,
    pub icon: String,
    pub connected: bool,
    pub busy: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BluetoothState {
    pub available: bool,
    pub ready: bool,
    pub powered: bool,
    pub target_powered: bool,
    pub busy: bool,
    pub hardware_blocked: bool,
    pub devices: Vec<BluetoothDevice>,
}
#[derive(Clone, Copy)]
enum Action {
    Power(bool),
    Connection(bool),
}
impl Action {
    fn is_power(self) -> bool {
        matches!(self, Self::Power(_))
    }
    fn target(self) -> bool {
        match self {
            Self::Power(v) | Self::Connection(v) => v,
        }
    }
    fn property(self) -> &'static str {
        if self.is_power() {
            "Powered"
        } else {
            "Connected"
        }
    }
    fn interface(self) -> &'static str {
        if self.is_power() { ADAPTER } else { DEVICE }
    }
}
struct Operation {
    id: u64,
    proxy: gio::DBusProxy,
    action: Action,
    cancel: gio::Cancellable,
    deadline: Instant,
    retries: Cell<u32>,
    retry: RefCell<Option<glib::Source>>,
    description: String,
}
impl Operation {
    fn cancel(&self) {
        self.cancel.cancel();
        destroy(&self.retry);
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        self.cancel();
    }
}
struct Manager {
    manager: gio::DBusObjectManagerClient,
    handlers: Vec<glib::SignalHandlerId>,
}
impl Drop for Manager {
    fn drop(&mut self) {
        for handler in self.handlers.drain(..) {
            self.manager.disconnect(handler);
        }
    }
}
mod imp {
    use super::*;
    #[derive(Default)]
    pub struct BluetoothService {
        pub(super) watcher: RefCell<Option<Watcher>>,
        pub(super) manager: RefCell<Option<Manager>>,
        pub(super) initialization: RefCell<Option<gio::Cancellable>>,
        pub(super) radio: RefCell<Option<radio::Radio>>,
        pub(super) radios: RefCell<HashMap<u32, radio::Event>>,
        pub(super) pending: RefCell<HashMap<String, Rc<Operation>>>,
        pub(super) failed: RefCell<Option<(gio::DBusProxy, bool)>>,
        pub(super) changed: RefCell<Option<glib::Source>>,
        pub(super) power_timeout: RefCell<Option<glib::Source>>,
        pub(super) targets: RefCell<Option<HashMap<String, bool>>>,
        pub(super) requested: Cell<bool>,
        pub(super) block_after_off: Cell<bool>,
        pub(super) airplane: Cell<bool>,
        pub(super) airplane_pending: Cell<bool>,
        pub(super) airplane_override: Cell<bool>,
        pub(super) restore_power: RefCell<HashMap<String, bool>>,
        pub(super) restore_blocks: RefCell<HashMap<u32, bool>>,
        pub(super) generation: Cell<u64>,
        pub(super) next_operation: Cell<u64>,
        pub(super) stopped: Cell<bool>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for BluetoothService {
        const NAME: &'static str = "WayShellBluetoothService";
        type Type = super::BluetoothService;
    }
    impl ObjectImpl for BluetoothService {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("changed").build(),
                    glib::subclass::Signal::builder("operation-error")
                        .param_types([String::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("operation-succeeded").build(),
                ]
            })
        }
        fn dispose(&self) {
            self.obj().stop();
        }
    }
}
glib::wrapper! { pub struct BluetoothService(ObjectSubclass<imp::BluetoothService>); }
impl Default for BluetoothService {
    fn default() -> Self {
        Self::new()
    }
}
impl BluetoothService {
    pub fn new() -> Self {
        let service: Self = glib::Object::new();
        service.attach_radio(radio::open());
        let appeared = service.downgrade();
        let vanished = service.downgrade();
        service.imp().watcher.replace(Some(Watcher::on_bus(
            gio::BusType::System,
            "org.bluez",
            move |connection, owner| {
                if let Some(s) = appeared.upgrade() {
                    s.appeared(connection, owner);
                }
            },
            move || {
                if let Some(s) = vanished.upgrade() {
                    s.reset_manager();
                    s.changed();
                }
            },
        )));
        service
    }
    /// Takes ownership of an optional nonblocking rfkill descriptor.
    /// Like GIO's asynchronous ObjectManager client, use on the default context.
    pub fn on_connection(connection: &gio::DBusConnection, rfkill: Option<OwnedFd>) -> Self {
        let service: Self = glib::Object::new();
        service.attach_radio(rfkill);
        let appeared = service.downgrade();
        let vanished = service.downgrade();
        service.imp().watcher.replace(Some(Watcher::on_connection(
            connection,
            "org.bluez",
            move |connection, owner| {
                if let Some(s) = appeared.upgrade() {
                    s.appeared(connection, owner);
                }
            },
            move || {
                if let Some(s) = vanished.upgrade() {
                    s.reset_manager();
                    s.changed();
                }
            },
        )));
        service
    }
    pub fn stop(&self) {
        if self.imp().stopped.replace(true) {
            return;
        }
        self.imp().watcher.borrow_mut().take();
        self.reset_manager();
        destroy(&self.imp().changed);
        destroy(&self.imp().power_timeout);
        self.imp().radio.borrow_mut().take();
        self.imp().radios.borrow_mut().clear();
        self.imp().targets.borrow_mut().take();
    }
    fn reset_manager(&self) {
        let imp = self.imp();
        imp.generation.set(imp.generation.get().wrapping_add(1));
        if let Some(task) = imp.initialization.borrow_mut().take() {
            task.cancel();
        }
        imp.manager.borrow_mut().take();
        imp.failed.borrow_mut().take();
        let pending = imp.pending.take();
        for op in pending.values() {
            op.cancel();
        }
    }
    fn appeared(&self, connection: gio::DBusConnection, owner: &str) {
        if self.imp().stopped.get() {
            return;
        }
        self.reset_manager();
        let generation = self.imp().generation.get();
        let cancel = gio::Cancellable::new();
        self.imp().initialization.replace(Some(cancel.clone()));
        let weak = self.downgrade();
        gio::AsyncInitable::builder::<gio::DBusObjectManagerClient>()
            .property("connection", &connection)
            .property(
                "flags",
                gio::DBusObjectManagerClientFlags::DO_NOT_AUTO_START,
            )
            .property("name", owner)
            .property("object-path", "/")
            .build(glib::Priority::DEFAULT, Some(&cancel), move |result| {
                let Some(service) = weak
                    .upgrade()
                    .filter(|s| !s.imp().stopped.get() && s.imp().generation.get() == generation)
                else {
                    return;
                };
                service.imp().initialization.borrow_mut().take();
                match result {
                    Ok(manager) => {
                        let mut handlers = Vec::new();
                        for signal in [
                            "object-added",
                            "object-removed",
                            "interface-added",
                            "interface-removed",
                        ] {
                            let weak = service.downgrade();
                            handlers.push(manager.connect_local(signal, false, move |_| {
                                if let Some(s) = weak.upgrade() {
                                    s.prune();
                                    s.changed();
                                }
                                None
                            }));
                        }
                        let weak = service.downgrade();
                        handlers.push(manager.connect_local(
                            "interface-proxy-properties-changed",
                            false,
                            move |args| {
                                if let Some(s) = weak.upgrade() {
                                    let proxy = args[2].get::<gio::DBusProxy>().unwrap();
                                    let properties = args[3].get::<glib::Variant>().unwrap();
                                    s.properties_changed(&proxy, &properties);
                                }
                                None
                            },
                        ));
                        service
                            .imp()
                            .manager
                            .replace(Some(Manager { manager, handlers }));
                    }
                    Err(error) => service.error(error.message()),
                }
                service.changed();
            });
    }
    fn objects(&self) -> Vec<gio::DBusObject> {
        self.imp()
            .manager
            .borrow()
            .as_ref()
            .map(|m| m.manager.objects())
            .unwrap_or_default()
    }
    fn proxy(&self, path: &str, interface: &str) -> Option<gio::DBusProxy> {
        DBusObjectManagerExt::interface(
            &self.imp().manager.borrow().as_ref()?.manager,
            path,
            interface,
        )?
        .downcast()
        .ok()
    }
    fn ready(&self) -> bool {
        self.imp()
            .manager
            .borrow()
            .as_ref()
            .is_some_and(|m| m.manager.name_owner().is_some())
    }
    fn blocked(&self, hard: bool) -> bool {
        self.imp()
            .radios
            .borrow()
            .values()
            .any(|r| if hard { r.hard } else { r.soft })
    }
    fn powered(&self) -> bool {
        !self.blocked(false)
            && !self.blocked(true)
            && self.objects().iter().any(|o| {
                DBusObjectExt::interface(o, ADAPTER)
                    .and_then(|p| p.downcast::<gio::DBusProxy>().ok())
                    .is_some_and(|p| boolean(&p, "Powered"))
            })
    }
    fn busy(&self) -> bool {
        self.imp().power_timeout.borrow().is_some()
            || self
                .imp()
                .pending
                .borrow()
                .values()
                .any(|op| op.action.is_power())
    }
    pub fn state(&self) -> BluetoothState {
        let powered = self.powered();
        let objects = self.objects();
        let mut devices = Vec::new();
        if powered {
            for object in &objects {
                let Some(proxy) = DBusObjectExt::interface(object, DEVICE)
                    .and_then(|p| p.downcast::<gio::DBusProxy>().ok())
                else {
                    continue;
                };
                let connected = boolean(&proxy, "Connected");
                if (!boolean(&proxy, "Paired") && !boolean(&proxy, "Trusted"))
                    || (!connected && !connectable(&proxy))
                {
                    continue;
                }
                if !self
                    .proxy(&string(&proxy, "Adapter", ""), ADAPTER)
                    .is_some_and(|p| boolean(&p, "Powered"))
                {
                    continue;
                }
                let path = proxy.object_path().to_string();
                devices.push(BluetoothDevice {
                    busy: self.imp().pending.borrow().contains_key(&path),
                    path,
                    alias: string(&proxy, "Alias", "Bluetooth device"),
                    icon: string(&proxy, "Icon", "bluetooth-symbolic"),
                    connected,
                });
            }
        }
        devices.sort_by(|a, b| {
            b.connected
                .cmp(&a.connected)
                .then_with(|| {
                    glib::CollationKey::from(a.alias.as_str())
                        .cmp(&glib::CollationKey::from(b.alias.as_str()))
                })
                .then_with(|| a.path.cmp(&b.path))
        });
        BluetoothState {
            available: !self.imp().radios.borrow().is_empty()
                || objects
                    .iter()
                    .any(|o| DBusObjectExt::interface(o, ADAPTER).is_some()),
            ready: self.ready(),
            powered,
            target_powered: if self.imp().power_timeout.borrow().is_some() {
                self.imp().requested.get()
            } else {
                powered
            },
            busy: self.busy(),
            hardware_blocked: self.blocked(true),
            devices,
        }
    }
    fn changed(&self) {
        if self.imp().stopped.get() || self.imp().changed.borrow().is_some() {
            return;
        }
        let weak: glib::SendWeakRef<Self> = self.downgrade().into();
        let source = glib::source::idle_source_new(
            Some("bluetooth-changed"),
            glib::Priority::DEFAULT_IDLE,
            move || {
                if let Some(s) = weak.upgrade() {
                    s.imp().changed.borrow_mut().take();
                    s.enter_airplane_if_ready();
                    s.reconcile();
                    if !s.imp().stopped.get() {
                        s.emit_by_name::<()>("changed", &[]);
                    }
                }
                glib::ControlFlow::Break
            },
        );
        source.attach(Some(&glib::MainContext::ref_thread_default()));
        self.imp().changed.replace(Some(source));
    }
    fn error(&self, message: &str) {
        self.imp().failed.borrow_mut().take();
        if !self.imp().stopped.get() {
            glib::g_message!("way-shell", "Bluetooth: {message}");
            self.emit_by_name::<()>("operation-error", &[&message]);
        }
    }
    fn succeeded(&self) {
        self.imp().failed.borrow_mut().take();
        if !self.imp().stopped.get() {
            self.emit_by_name::<()>("operation-succeeded", &[]);
        }
    }
    pub fn set_powered(&self, powered: bool) {
        if self.imp().airplane.get() {
            self.imp().airplane_pending.set(false);
            self.imp().airplane_override.set(true);
        }
        self.request_power(powered, None);
    }
    fn request_power(&self, powered: bool, targets: Option<HashMap<String, bool>>) {
        if self.imp().stopped.get() {
            return;
        }
        if powered && self.blocked(true) {
            self.error("Bluetooth is disabled by a hardware switch");
            return;
        }
        if !self.ready() {
            self.error("The Bluetooth service is unavailable");
            return;
        }
        destroy(&self.imp().power_timeout);
        self.imp().requested.set(powered);
        self.imp().targets.replace(targets);
        self.imp()
            .block_after_off
            .set(!powered && self.imp().airplane.get() && !self.imp().airplane_override.get());
        let weak: glib::SendWeakRef<Self> = self.downgrade().into();
        let source = glib::source::timeout_source_new(
            POWER_TIMEOUT,
            Some("bluetooth-power"),
            glib::Priority::DEFAULT,
            move || {
                if let Some(s) = weak.upgrade() {
                    s.imp().power_timeout.borrow_mut().take();
                    s.finish_power(false);
                    s.error(if s.ready() {
                        "Bluetooth did not respond. Check that the adapter is available."
                    } else {
                        "The Bluetooth service is unavailable"
                    });
                }
                glib::ControlFlow::Break
            },
        );
        source.attach(Some(&glib::MainContext::ref_thread_default()));
        self.imp().power_timeout.replace(Some(source));
        if !powered {
            self.cancel_where(|op| !op.action.is_power());
        }
        if powered
            && self.imp().targets.borrow().is_none()
            && self.blocked(false)
            && !self.write_radio(3, 0, false)
        {
            self.finish_power(false);
            return;
        }
        self.changed();
    }
    fn target(&self, path: &str) -> bool {
        self.imp()
            .targets
            .borrow()
            .as_ref()
            .map(|m| m.get(path).copied().unwrap_or(false))
            .unwrap_or(self.imp().requested.get())
    }
    fn reconcile(&self) {
        if self.imp().stopped.get() || self.imp().power_timeout.borrow().is_none() || !self.ready()
        {
            return;
        }
        let mut count = 0;
        let mut complete = true;
        for object in self.objects() {
            let Some(adapter) = DBusObjectExt::interface(&object, ADAPTER)
                .and_then(|p| p.downcast::<gio::DBusProxy>().ok())
            else {
                continue;
            };
            let path = adapter.object_path();
            if self
                .imp()
                .targets
                .borrow()
                .as_ref()
                .is_some_and(|m| !m.contains_key(path.as_str()))
            {
                continue;
            }
            count += 1;
            let target = self.target(&path);
            if boolean(&adapter, "Powered") != target {
                complete = false;
                if !self.imp().pending.borrow().contains_key(path.as_str()) {
                    self.call(
                        adapter,
                        Action::Power(target),
                        if target {
                            "Could not turn Bluetooth on"
                        } else {
                            "Could not turn Bluetooth off"
                        }
                        .into(),
                    );
                }
            }
        }
        if self.imp().requested.get()
            && (count == 0 || (self.imp().targets.borrow().is_none() && self.blocked(false)))
        {
            complete = false;
        }
        if self
            .imp()
            .pending
            .borrow()
            .values()
            .any(|op| op.action.is_power())
        {
            complete = false;
        }
        if complete {
            self.finish_power(true);
        }
    }
    fn finish_power(&self, success: bool) {
        destroy(&self.imp().power_timeout);
        self.imp().targets.borrow_mut().take();
        self.cancel_where(|op| op.action.is_power());
        let block = self.imp().block_after_off.replace(false);
        if success
            && (!block || self.imp().radio.borrow().is_none() || self.write_radio(3, 0, true))
        {
            self.succeeded();
        }
        self.changed();
    }
    pub fn toggle_device(&self, path: &str) {
        if self.imp().stopped.get()
            || self.busy()
            || !self.powered()
            || self.imp().pending.borrow().contains_key(path)
        {
            return;
        }
        let Some(proxy) = self.proxy(path, DEVICE) else {
            return;
        };
        let target = !boolean(&proxy, "Connected");
        let description = format!(
            "Could not {} {}",
            if target { "connect to" } else { "disconnect" },
            string(&proxy, "Alias", "device")
        );
        self.call(proxy, Action::Connection(target), description);
    }
    fn call(&self, proxy: gio::DBusProxy, action: Action, description: String) {
        let id = self.imp().next_operation.get().wrapping_add(1);
        self.imp().next_operation.set(id);
        let op = Rc::new(Operation {
            id,
            proxy,
            action,
            description,
            cancel: gio::Cancellable::new(),
            deadline: Instant::now()
                + if action.is_power() {
                    POWER_TIMEOUT
                } else {
                    DEVICE_TIMEOUT
                },
            retries: Cell::new(0),
            retry: RefCell::new(None),
        });
        self.imp()
            .pending
            .borrow_mut()
            .insert(op.proxy.object_path().to_string(), op.clone());
        self.perform(op);
        self.changed();
    }
    fn perform(&self, op: Rc<Operation>) {
        if self.imp().stopped.get() || op.cancel.is_cancelled() {
            return;
        }
        let (method, params) = match op.action {
            Action::Power(value) => (
                "org.freedesktop.DBus.Properties.Set",
                Some((ADAPTER, "Powered", value.to_variant()).to_variant()),
            ),
            Action::Connection(true) => ("Connect", None),
            Action::Connection(false) => ("Disconnect", None),
        };
        let weak = self.downgrade();
        let call = op.clone();
        op.proxy.call(
            method,
            params.as_ref(),
            gio::DBusCallFlags::NONE,
            op.deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .clamp(1, 30_000) as i32,
            Some(&op.cancel),
            move |reply| {
                if let Some(s) = weak.upgrade() {
                    s.finished(call, reply);
                }
            },
        );
    }
    fn finished(&self, op: Rc<Operation>, result: Result<glib::Variant, glib::Error>) {
        let path = op.proxy.object_path();
        let current = self.proxy(&path, op.action.interface());
        let relevant = !self.imp().stopped.get()
            && self.ready()
            && current.as_ref() == Some(&op.proxy)
            && !op.cancel.is_cancelled()
            && self
                .imp()
                .pending
                .borrow()
                .get(path.as_str())
                .is_some_and(|p| p.id == op.id);
        let superseded = op.action.is_power()
            && self.imp().power_timeout.borrow().is_some()
            && self.target(&path) != op.action.target();
        let reached =
            current.is_some_and(|p| boolean(&p, op.action.property()) == op.action.target());
        if op.action.is_power()
            && relevant
            && !superseded
            && !reached
            && op.retries.get() < 40
            && Instant::now() < op.deadline
            && result.as_ref().err().is_some_and(transient)
        {
            op.retries.set(op.retries.get() + 1);
            let weak: glib::SendWeakRef<Self> = self.downgrade().into();
            let path = path.to_string();
            let id = op.id;
            let source = glib::source::timeout_source_new(
                Duration::from_millis(100),
                Some("bluetooth-retry"),
                glib::Priority::DEFAULT,
                move || {
                    if let Some(s) = weak.upgrade() {
                        let op = s
                            .imp()
                            .pending
                            .borrow()
                            .get(&path)
                            .filter(|p| p.id == id)
                            .cloned();
                        if let Some(op) = op {
                            op.retry.borrow_mut().take();
                            if s.target(&path) != op.action.target() {
                                s.imp().pending.borrow_mut().remove(&path);
                                s.changed();
                            } else {
                                s.perform(op);
                            }
                        }
                    }
                    glib::ControlFlow::Break
                },
            );
            source.attach(Some(&glib::MainContext::ref_thread_default()));
            op.retry.replace(Some(source));
            return;
        }
        if self
            .imp()
            .pending
            .borrow()
            .get(path.as_str())
            .is_some_and(|p| p.id == op.id)
        {
            self.imp().pending.borrow_mut().remove(path.as_str());
        }
        if relevant && !superseded {
            match result {
                Err(mut error) if !reached => {
                    gio::DBusError::strip_remote_error(&mut error);
                    self.error(&format!("{}: {}", op.description, error.message()));
                    if op.action.is_power() {
                        self.finish_power(false);
                    } else if !self.imp().stopped.get() {
                        self.imp()
                            .failed
                            .replace(Some((op.proxy.clone(), op.action.target())));
                    }
                }
                _ => self.succeeded(),
            }
        }
        self.changed();
    }
    fn cancel_where(&self, predicate: impl Fn(&Operation) -> bool) {
        let paths: Vec<_> = self
            .imp()
            .pending
            .borrow()
            .iter()
            .filter(|(_, op)| predicate(op))
            .map(|(p, _)| p.clone())
            .collect();
        for path in paths {
            let op = self.imp().pending.borrow_mut().remove(&path);
            if let Some(op) = op {
                op.cancel();
            }
        }
    }
    fn prune(&self) {
        let stale = self
            .imp()
            .failed
            .borrow()
            .as_ref()
            .is_some_and(|(p, _)| self.proxy(&p.object_path(), DEVICE).as_ref() != Some(p));
        if stale {
            self.imp().failed.borrow_mut().take();
        }
        self.cancel_where(|op| {
            let current = self.proxy(&op.proxy.object_path(), op.action.interface());
            current.as_ref() != Some(&op.proxy)
                || (!op.action.is_power()
                    && (self.blocked(false)
                        || self.blocked(true)
                        || !self
                            .proxy(&string(&op.proxy, "Adapter", ""), ADAPTER)
                            .is_some_and(|p| boolean(&p, "Powered"))))
        });
    }
    fn properties_changed(&self, proxy: &gio::DBusProxy, values: &glib::Variant) {
        self.prune();
        let values = glib::VariantDict::new(Some(values));
        let recovered = self
            .imp()
            .failed
            .borrow()
            .as_ref()
            .is_some_and(|(p, target)| {
                p == proxy && values.lookup::<bool>("Connected").ok().flatten() == Some(*target)
            });
        if recovered {
            self.succeeded();
        }
        if self.imp().airplane.get()
            && !self.busy()
            && proxy.interface_name() == ADAPTER
            && values.lookup::<bool>("Powered").ok().flatten() == Some(true)
        {
            self.imp().airplane_override.set(true);
        }
        self.changed();
    }
    pub fn set_airplane_mode(&self, enabled: bool) {
        if self.imp().stopped.get() || self.imp().airplane.replace(enabled) == enabled {
            return;
        }
        if enabled {
            self.imp().airplane_override.set(false);
            self.imp().airplane_pending.set(true);
            self.enter_airplane_if_ready();
        } else {
            self.imp().airplane_pending.set(false);
            let powers = self.imp().restore_power.take();
            let blocks = self.imp().restore_blocks.take();
            if !self.imp().airplane_override.get() {
                self.imp().block_after_off.set(false);
                if !blocks.is_empty() {
                    self.write_radio(3, 0, blocks.values().all(|v| *v));
                }
                for (id, blocked) in blocks {
                    if self.imp().radios.borrow().contains_key(&id) {
                        self.write_radio(2, id, blocked);
                    }
                }
                if !powers.is_empty() {
                    self.request_power(powers.values().any(|v| *v), Some(powers));
                }
            }
        }
    }
    fn enter_airplane_if_ready(&self) {
        if self.imp().stopped.get()
            || !self.imp().airplane_pending.get()
            || !self.ready()
            || !self.state().available
        {
            return;
        }
        self.imp().airplane_pending.set(false);
        let powers = self
            .objects()
            .into_iter()
            .filter_map(|o| {
                let p = DBusObjectExt::interface(&o, ADAPTER)?
                    .downcast::<gio::DBusProxy>()
                    .ok()?;
                Some((p.object_path().to_string(), boolean(&p, "Powered")))
            })
            .collect();
        self.imp().restore_power.replace(powers);
        self.imp().restore_blocks.replace(
            self.imp()
                .radios
                .borrow()
                .iter()
                .map(|(id, r)| (*id, r.soft))
                .collect(),
        );
        self.request_power(false, None);
    }
}
fn destroy(source: &RefCell<Option<glib::Source>>) {
    if let Some(source) = source.borrow_mut().take() {
        source.destroy();
    }
}
fn boolean(proxy: &gio::DBusProxy, name: &str) -> bool {
    proxy
        .cached_property(name)
        .and_then(|v| v.get())
        .unwrap_or(false)
}
fn string(proxy: &gio::DBusProxy, name: &str, fallback: &str) -> String {
    proxy
        .cached_property(name)
        .and_then(|v| v.str().map(str::to_owned))
        .unwrap_or_else(|| fallback.into())
}
fn connectable(proxy: &gio::DBusProxy) -> bool {
    const PROFILES: &[&str] = &[
        "00001108", "0000110a", "0000110b", "0000110c", "0000110e", "00001112", "0000111e",
        "0000111f", "00001124", "00001812",
    ];
    proxy
        .cached_property("UUIDs")
        .and_then(|v| v.get::<Vec<String>>())
        .is_some_and(|uuids| {
            uuids.iter().any(|uuid| {
                uuid.eq_ignore_ascii_case("03b80e5a-ede8-4b33-a751-6ce34ec4c700")
                    || PROFILES.iter().any(|p| {
                        uuid.eq_ignore_ascii_case(&format!("{p}-0000-1000-8000-00805f9b34fb"))
                    })
            })
        })
}
fn transient(error: &glib::Error) -> bool {
    match gio::DBusError::remote_error(error).as_deref() {
        Some(
            "org.bluez.Error.Busy"
            | "org.bluez.Error.Blocked"
            | "org.bluez.Error.InProgress"
            | "org.bluez.Error.NotReady",
        ) => true,
        Some("org.bluez.Error.Failed") => {
            error.message().contains("Blocked") || error.message().contains("blocked")
        }
        _ => false,
    }
}
