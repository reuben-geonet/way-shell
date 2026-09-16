#![allow(dead_code)]
use gio::prelude::*;
use glib::variant::ObjectPath;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    io::{BufRead, BufReader},
    os::unix::net::UnixDatagram,
    process::{Child, Command, Stdio},
    rc::Rc,
    time::{Duration, Instant},
};
use way_shell::services::bluetooth::BluetoothService;
pub const ADAPTER: &str = "org.bluez.Adapter1";
pub const DEVICE: &str = "org.bluez.Device1";
pub const HCI: &str = "/org/bluez/hci0";
pub const MOUSE: &str = "/org/bluez/hci0/dev_01";
pub const HEADPHONES: &str = "/org/bluez/hci0/dev_02";
pub type Properties = HashMap<String, glib::Variant>;
pub type Interfaces = HashMap<String, Properties>;
pub type Managed = HashMap<ObjectPath, Interfaces>;
pub fn path(value: &str) -> ObjectPath {
    ObjectPath::try_from(value).unwrap()
}
pub struct Bus(Child);
impl Bus {
    pub fn start() -> (Self, String) {
        let mut child = Command::new("dbus-daemon")
            .args([
                "--nofork",
                "--print-address=1",
                &format!(
                    "--config-file={}/../../tests/fixtures/session-bus.conf",
                    env!("CARGO_MANIFEST_DIR")
                ),
            ])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        (Self(child), address.trim().into())
    }
}
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
pub fn connect(address: &str) -> gio::DBusConnection {
    let c = gio::DBusConnection::for_address_sync(
        address,
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        gio::Cancellable::NONE,
    )
    .unwrap();
    c.set_exit_on_close(false);
    c
}
pub fn pump(context: &glib::MainContext) {
    context.block_on(glib::timeout_future(Duration::from_millis(30)));
}
#[track_caller]
pub fn wait(context: &glib::MainContext, condition: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(7);
    while !condition() {
        assert!(Instant::now() < end, "Bluetooth fixture timed out");
        context.block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}
pub fn device(
    alias: &str,
    adapter: &str,
    paired: bool,
    trusted: bool,
    connected: bool,
    uuid: &str,
) -> Interfaces {
    HashMap::from([(
        DEVICE.into(),
        HashMap::from([
            ("Alias".into(), alias.to_variant()),
            ("Icon".into(), "input-mouse".to_variant()),
            ("Adapter".into(), path(adapter).to_variant()),
            ("Paired".into(), paired.to_variant()),
            ("Trusted".into(), trusted.to_variant()),
            ("Connected".into(), connected.to_variant()),
            ("UUIDs".into(), vec![uuid].to_variant()),
        ]),
    )])
}
pub struct Held {
    pub path: String,
    pub target: bool,
    pub power: bool,
    pub invocation: gio::DBusMethodInvocation,
}
#[derive(Default)]
pub struct State {
    pub objects: RefCell<Managed>,
    pub calls: RefCell<Vec<(String, String, bool)>>,
    pub power_errors: RefCell<VecDeque<String>>,
    pub device_error: RefCell<Option<String>>,
    pub hold_power: Cell<bool>,
    pub hold_device: Cell<bool>,
    pub held: RefCell<Vec<Held>>,
}
pub struct Fake {
    pub connection: gio::DBusConnection,
    pub state: Rc<State>,
    registrations: RefCell<HashMap<String, Vec<gio::RegistrationId>>>,
}
const XML: &str = "<node>
<interface name='org.freedesktop.DBus.ObjectManager'><method name='GetManagedObjects'><arg type='a{oa{sa{sv}}}' direction='out'/></method><signal name='InterfacesAdded'><arg type='o'/><arg type='a{sa{sv}}'/></signal><signal name='InterfacesRemoved'><arg type='o'/><arg type='as'/></signal></interface>
<interface name='org.bluez.Adapter1'><property name='Powered' type='b' access='readwrite'/></interface>
<interface name='org.bluez.Device1'><method name='Connect'/><method name='Disconnect'/><property name='Alias' type='s' access='read'/><property name='Icon' type='s' access='read'/><property name='Adapter' type='o' access='read'/><property name='UUIDs' type='as' access='read'/><property name='Paired' type='b' access='read'/><property name='Trusted' type='b' access='read'/><property name='Connected' type='b' access='read'/></interface>
<interface name='org.freedesktop.DBus.Properties'><method name='Get'><arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='v' direction='out'/></method><method name='GetAll'><arg type='s' direction='in'/><arg type='a{sv}' direction='out'/></method><method name='Set'><arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='v' direction='in'/></method></interface></node>";
impl Fake {
    pub fn new(address: &str) -> Self {
        let this = Self {
            connection: connect(address),
            state: Rc::new(State::default()),
            registrations: RefCell::new(HashMap::new()),
        };
        let info = gio::DBusNodeInfo::for_xml(XML).unwrap();
        let state = this.state.clone();
        let root = this
            .connection
            .register_object(
                "/",
                &info
                    .lookup_interface("org.freedesktop.DBus.ObjectManager")
                    .unwrap(),
            )
            .method_call(move |_, _, _, _, _, _, invocation| {
                invocation.return_value(Some(&(state.objects.borrow().clone(),).to_variant()));
            })
            .build()
            .unwrap();
        this.registrations
            .borrow_mut()
            .insert("/".into(), vec![root]);
        this.add(
            HCI,
            HashMap::from([(
                ADAPTER.into(),
                HashMap::from([("Powered".into(), true.to_variant())]),
            )]),
        );
        this.add(
            MOUSE,
            device(
                "Zebra mouse",
                HCI,
                true,
                false,
                true,
                "00001812-0000-1000-8000-00805f9b34fb",
            ),
        );
        this.add(
            HEADPHONES,
            device(
                "Alpha headphones",
                HCI,
                false,
                true,
                false,
                "0000110b-0000-1000-8000-00805f9b34fb",
            ),
        );
        this.add(
            "/org/bluez/hci0/dev_03",
            device(
                "Unpaired mouse",
                HCI,
                false,
                false,
                false,
                "00001812-0000-1000-8000-00805f9b34fb",
            ),
        );
        this.add(
            "/org/bluez/hci0/dev_04",
            device(
                "File transfer phone",
                HCI,
                true,
                true,
                false,
                "00001105-0000-1000-8000-00805f9b34fb",
            ),
        );
        this.ownership(true);
        this
    }
    pub fn ownership(&self, acquire: bool) {
        self.connection
            .call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                if acquire {
                    "RequestName"
                } else {
                    "ReleaseName"
                },
                Some(&if acquire {
                    ("org.bluez", 0_u32).to_variant()
                } else {
                    ("org.bluez",).to_variant()
                }),
                None,
                gio::DBusCallFlags::NONE,
                2000,
                gio::Cancellable::NONE,
            )
            .unwrap();
    }
    pub fn add(&self, id: &str, interfaces: Interfaces) {
        let info = gio::DBusNodeInfo::for_xml(XML).unwrap();
        self.state
            .objects
            .borrow_mut()
            .insert(path(id), interfaces.clone());
        let mut registrations = Vec::new();
        for interface in interfaces
            .keys()
            .map(String::as_str)
            .chain(["org.freedesktop.DBus.Properties"])
        {
            let state = self.state.clone();
            let props = self.state.clone();
            let registration = self
                .connection
                .register_object(id, &info.lookup_interface(interface).unwrap())
                .property(move |_, _, object, interface, name| {
                    props.objects.borrow()[&path(object)][interface][name].clone()
                })
                .method_call(
                    move |connection, _, object, interface, method, parameters, invocation| {
                        if interface == Some("org.freedesktop.DBus.Properties") && method != "Set" {
                            let iface = parameters.child_get::<String>(0);
                            let properties = state.objects.borrow()[&path(object)][&iface].clone();
                            invocation.return_value(Some(&if method == "GetAll" {
                                (properties,).to_variant()
                            } else {
                                (properties[&parameters.child_get::<String>(1)].clone(),)
                                    .to_variant()
                            }));
                            return;
                        }
                        let power = method == "Set";
                        let target = if power {
                            parameters
                                .child_get::<glib::Variant>(2)
                                .get::<bool>()
                                .unwrap()
                        } else {
                            method == "Connect"
                        };
                        state
                            .calls
                            .borrow_mut()
                            .push((object.into(), method.into(), target));
                        let error = if power {
                            state.power_errors.borrow_mut().pop_front()
                        } else {
                            state.device_error.borrow().clone()
                        };
                        if let Some(error) = error {
                            invocation.return_dbus_error(
                                &error,
                                if power {
                                    "Blocked during radio startup"
                                } else {
                                    "Device is out of range"
                                },
                            );
                            return;
                        }
                        if if power {
                            state.hold_power.get()
                        } else {
                            state.hold_device.get()
                        } {
                            state.held.borrow_mut().push(Held {
                                path: object.into(),
                                target,
                                power,
                                invocation,
                            });
                        } else {
                            set(
                                &connection,
                                &state,
                                object,
                                if power { ADAPTER } else { DEVICE },
                                if power { "Powered" } else { "Connected" },
                                target.to_variant(),
                            );
                            invocation.return_value(None);
                        }
                    },
                )
                .build()
                .unwrap();
            registrations.push(registration);
        }
        self.registrations
            .borrow_mut()
            .insert(id.into(), registrations);
        self.connection
            .emit_signal(
                None,
                "/",
                "org.freedesktop.DBus.ObjectManager",
                "InterfacesAdded",
                Some(&(path(id), interfaces).to_variant()),
            )
            .unwrap();
    }
    pub fn remove(&self, id: &str) -> Interfaces {
        let interfaces = self.state.objects.borrow_mut().remove(&path(id)).unwrap();
        for registration in self.registrations.borrow_mut().remove(id).unwrap() {
            self.connection.unregister_object(registration).unwrap();
        }
        self.connection
            .emit_signal(
                None,
                "/",
                "org.freedesktop.DBus.ObjectManager",
                "InterfacesRemoved",
                Some(&(path(id), interfaces.keys().cloned().collect::<Vec<_>>()).to_variant()),
            )
            .unwrap();
        interfaces
    }
    pub fn set(&self, id: &str, interface: &str, property: &str, value: impl ToVariant) {
        set(
            &self.connection,
            &self.state,
            id,
            interface,
            property,
            value.to_variant(),
        );
    }
    pub fn value(&self, id: &str, property: &str) -> bool {
        self.state.objects.borrow()[&path(id)][if property == "Powered" {
            ADAPTER
        } else {
            DEVICE
        }][property]
            .get()
            .unwrap()
    }
    pub fn finish(&self, error: Option<&str>, update: bool) {
        let held = self.state.held.borrow_mut().remove(0);
        if update {
            self.set(
                &held.path,
                if held.power { ADAPTER } else { DEVICE },
                if held.power { "Powered" } else { "Connected" },
                held.target,
            );
        }
        if let Some(error) = error {
            held.invocation
                .return_dbus_error(error, "Timeout was reached");
        } else {
            held.invocation.return_value(None);
        }
    }
    pub fn power_calls(&self) -> usize {
        self.state
            .calls
            .borrow()
            .iter()
            .filter(|(_, m, _)| m == "Set")
            .count()
    }
}
fn set(
    connection: &gio::DBusConnection,
    state: &State,
    id: &str,
    interface: &str,
    name: &str,
    value: glib::Variant,
) {
    state
        .objects
        .borrow_mut()
        .get_mut(&path(id))
        .unwrap()
        .get_mut(interface)
        .unwrap()
        .insert(name.into(), value.clone());
    connection
        .emit_signal(
            None,
            id,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            Some(
                &(
                    interface,
                    HashMap::from([(name, value)]),
                    Vec::<String>::new(),
                )
                    .to_variant(),
            ),
        )
        .unwrap();
}
impl Drop for Fake {
    fn drop(&mut self) {
        for held in self.state.held.take() {
            held.invocation
                .return_dbus_error("org.bluez.Error.Failed", "Fixture stopped");
        }
        for registrations in self.registrations.take().into_values() {
            for r in registrations {
                self.connection.unregister_object(r).unwrap();
            }
        }
        let _ = self.connection.close_sync(gio::Cancellable::NONE);
    }
}
pub fn radio(peer: &UnixDatagram, id: u32, operation: u8, soft: bool, hard: bool) {
    let mut bytes = [0u8; 8];
    bytes[..4].copy_from_slice(&id.to_ne_bytes());
    bytes[4..].copy_from_slice(&[2, operation, u8::from(soft), u8::from(hard)]);
    peer.send(&bytes).unwrap();
}
pub struct Fixture {
    pub service: BluetoothService,
    pub fake: Fake,
    pub radio: UnixDatagram,
    pub errors: Rc<RefCell<Vec<String>>>,
    pub successes: Rc<Cell<u32>>,
    _client: gio::DBusConnection,
    _bus: Bus,
}
impl Fixture {
    pub fn new(context: &glib::MainContext) -> Self {
        let (bus, address) = Bus::start();
        let fake = Fake::new(&address);
        let client = connect(&address);
        let (fd, peer) = UnixDatagram::pair().unwrap();
        fd.set_nonblocking(true).unwrap();
        peer.set_nonblocking(true).unwrap();
        radio(&peer, 4, 0, false, false);
        let service = BluetoothService::on_connection(&client, Some(fd.into()));
        let errors = Rc::new(RefCell::new(Vec::new()));
        let seen = errors.clone();
        service.connect_local("operation-error", false, move |v| {
            seen.borrow_mut().push(v[1].get::<String>().unwrap());
            None
        });
        let successes = Rc::new(Cell::new(0));
        let seen = successes.clone();
        service.connect_local("operation-succeeded", false, move |_| {
            seen.set(seen.get() + 1);
            None
        });
        wait(context, || service.state().ready);
        Self {
            service,
            fake,
            radio: peer,
            errors,
            successes,
            _client: client,
            _bus: bus,
        }
    }
    pub fn settle(&self, context: &glib::MainContext) {
        pump(context);
        wait(context, || !self.service.state().busy);
    }
    pub fn read_radio(&self) -> Vec<[u8; 8]> {
        let mut result = Vec::new();
        loop {
            let mut b = [0; 8];
            if self.radio.recv(&mut b).is_err() {
                break;
            }
            result.push(b);
        }
        result
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.service.stop();
    }
}
