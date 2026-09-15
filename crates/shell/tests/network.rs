use gio::prelude::*;
use glib::variant::ObjectPath;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    rc::Rc,
    time::{Duration, Instant},
};
use way_shell::services::network::NetworkService;
const NAME: &str = "org.freedesktop.NetworkManager";
const ROOT: &str = "/org/freedesktop/NetworkManager";
const DEVICE: &str = "/org/freedesktop/NetworkManager/Devices/1";
const ACTIVE: &str = "/org/freedesktop/NetworkManager/ActiveConnection/1";
const DEVICE_IFACE: &str = "org.freedesktop.NetworkManager.Device";
fn path(value: &str) -> ObjectPath {
    ObjectPath::try_from(value).unwrap()
}
type Properties = HashMap<String, glib::Variant>;
type Managed = HashMap<ObjectPath, HashMap<String, Properties>>;
struct Bus(Child);
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn connect(address: &str) -> gio::DBusConnection {
    let connection = gio::DBusConnection::for_address_sync(
        address,
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        gio::Cancellable::NONE,
    )
    .unwrap();
    connection.set_exit_on_close(false);
    connection
}
#[track_caller]
fn wait(context: &glib::MainContext, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "NetworkManager fixture timed out"
        );
        context.block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}
fn ownership(connection: &gio::DBusConnection, acquire: bool) {
    let parameters = if acquire {
        (NAME, 0_u32).to_variant()
    } else {
        (NAME,).to_variant()
    };
    connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            if acquire {
                "RequestName"
            } else {
                "ReleaseName"
            },
            Some(&parameters),
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        )
        .unwrap();
}
fn properties(connection: &gio::DBusConnection, object: &str, interface: &str, values: Properties) {
    connection
        .emit_signal(
            None,
            object,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            Some(&(interface, values, Vec::<String>::new()).to_variant()),
        )
        .unwrap();
}
fn inventory() -> Managed {
    HashMap::from([
        (
            path(ROOT),
            HashMap::from([(
                NAME.into(),
                HashMap::from([
                    ("Version".into(), "1.58.0".to_variant()),
                    ("State".into(), 70_u32.to_variant()),
                    ("NetworkingEnabled".into(), true.to_variant()),
                    ("WirelessEnabled".into(), true.to_variant()),
                    ("WirelessHardwareEnabled".into(), true.to_variant()),
                    ("Devices".into(), vec![path(DEVICE)].to_variant()),
                    ("AllDevices".into(), vec![path(DEVICE)].to_variant()),
                    ("ActiveConnections".into(), vec![path(ACTIVE)].to_variant()),
                    ("PrimaryConnection".into(), path(ACTIVE).to_variant()),
                    ("ActivatingConnection".into(), path("/").to_variant()),
                ]),
            )]),
        ),
        (
            path(DEVICE),
            HashMap::from([
                (
                    DEVICE_IFACE.into(),
                    HashMap::from([
                        ("Interface".into(), "wlan0".to_variant()),
                        ("IpInterface".into(), "wlan0".to_variant()),
                        ("DeviceType".into(), 2_u32.to_variant()),
                        ("State".into(), 100_u32.to_variant()),
                        ("StateReason".into(), (100_u32, 0_u32).to_variant()),
                        ("Managed".into(), true.to_variant()),
                        ("Real".into(), true.to_variant()),
                        ("ActiveConnection".into(), path(ACTIVE).to_variant()),
                    ]),
                ),
                (
                    format!("{DEVICE_IFACE}.Wireless"),
                    HashMap::from([
                        ("AccessPoints".into(), Vec::<ObjectPath>::new().to_variant()),
                        ("ActiveAccessPoint".into(), path("/").to_variant()),
                    ]),
                ),
            ]),
        ),
        (
            path(ACTIVE),
            HashMap::from([(
                format!("{NAME}.Connection.Active"),
                HashMap::from([
                    ("Id".into(), "fixture".to_variant()),
                    (
                        "Uuid".into(),
                        "b701fe6c-0bc1-4e40-9faa-b6738b841f56".to_variant(),
                    ),
                    ("Type".into(), "802-11-wireless".to_variant()),
                    ("Devices".into(), vec![path(DEVICE)].to_variant()),
                    ("State".into(), 2_u32.to_variant()),
                    ("Connection".into(), path("/").to_variant()),
                    ("SpecificObject".into(), path("/").to_variant()),
                ]),
            )]),
        ),
    ])
}
#[test]
fn native_inventory_radio_state_removal_restart_and_cleanup() {
    let config = format!(
        "--config-file={}/../../tests/fixtures/session-bus.conf",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut child = Command::new("dbus-daemon")
        .args(["--nofork", "--print-address=1", &config])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut address = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    let _bus = Bus(child);
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| exercise(&context, address.trim()))
        .unwrap();
}

fn exercise(context: &glib::MainContext, address: &str) {
    let daemon = connect(address);
    let client = connect(address);
    let objects = Rc::new(RefCell::new(inventory()));
    let mut registrations = Vec::new();
    let manager=gio::DBusNodeInfo::for_xml("<node><interface name='org.freedesktop.DBus.ObjectManager'><method name='GetManagedObjects'><arg type='a{oa{sa{sv}}}' direction='out'/></method></interface></node>").unwrap();
    for root in ["/org/freedesktop", ROOT] {
        let objects = objects.clone();
        registrations.push(
            daemon
                .register_object(root, &manager.interfaces()[0])
                .method_call(move |_, _, _, _, method, _, invocation| {
                    assert_eq!(method, "GetManagedObjects");
                    invocation.return_value(Some(&(objects.borrow().clone(),).to_variant()));
                })
                .build()
                .unwrap(),
        );
    }
    let deny = Rc::new(Cell::new(false));
    let calls = Rc::new(RefCell::new(Vec::new()));
    let delay = Rc::new(Cell::new(false));
    let pending = Rc::new(RefCell::new(None));
    let node=gio::DBusNodeInfo::for_xml("<node><interface name='org.freedesktop.NetworkManager'><method name='Enable'><arg type='b' direction='in'/></method><method name='GetPermissions'><arg type='a{ss}' direction='out'/></method><property name='WirelessEnabled' type='b' access='readwrite'/></interface></node>").unwrap();
    let delayed = delay.clone();
    let held = pending.clone();
    let rejected = deny.clone();
    let recorded = calls.clone();
    let updated = objects.clone();
    let get = objects.clone();
    let set = objects.clone();
    let set_denied = deny.clone();
    registrations.push(
        daemon
            .register_object(ROOT, &node.interfaces()[0])
            .method_call(move |connection, _, _, _, method, args, invocation| {
                if method == "GetPermissions" {
                    invocation
                        .return_value(Some(&(HashMap::<String, String>::new(),).to_variant()));
                    return;
                }
                assert_eq!(method, "Enable");
                let (enabled,) = args.get::<(bool,)>().unwrap();
                recorded.borrow_mut().push(enabled);
                if rejected.get() {
                    invocation.return_dbus_error(
                        "org.freedesktop.NetworkManager.PermissionDenied",
                        "fixture denied",
                    );
                    return;
                }
                if delayed.get() {
                    held.replace(Some(invocation));
                    return;
                }
                updated
                    .borrow_mut()
                    .get_mut(&path(ROOT))
                    .unwrap()
                    .get_mut(NAME)
                    .unwrap()
                    .insert("NetworkingEnabled".into(), enabled.to_variant());
                properties(
                    &connection,
                    ROOT,
                    NAME,
                    HashMap::from([("NetworkingEnabled".into(), enabled.to_variant())]),
                );
                invocation.return_value(None);
            })
            .property(move |_, _, _, _, property| get.borrow()[&path(ROOT)][NAME][property].clone())
            .set_property(move |connection, _, _, _, property, value| {
                if set_denied.get() {
                    return false;
                }
                set.borrow_mut()
                    .get_mut(&path(ROOT))
                    .unwrap()
                    .get_mut(NAME)
                    .unwrap()
                    .insert(property.into(), value.clone());
                properties(
                    &connection,
                    ROOT,
                    NAME,
                    HashMap::from([(property.into(), value)]),
                );
                true
            })
            .build()
            .unwrap(),
    );
    let service = NetworkService::on_connection(&client);
    assert!(!service.state().available);
    service.set_networking(true, |result| {
        assert!(result.unwrap_err().matches(gio::IOErrorEnum::NotConnected));
    });
    ownership(&daemon, true);
    wait(context, || {
        service.state().available && service.state().devices.len() == 1
    });
    assert_eq!(service.state().devices[0].interface, "wlan0");
    assert_eq!(service.state().wifi_state(), 100);
    assert_eq!(service.state().primary.as_deref(), Some(DEVICE));
    let result = Rc::new(RefCell::new(None));
    let out = result.clone();
    service.set_wireless(false, move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || {
        result.borrow().is_some() && !service.state().wireless_enabled
    });
    result.borrow_mut().take().unwrap().unwrap();
    assert_eq!(service.state().wifi_state(), 30);
    let out = result.clone();
    service.set_wireless(true, move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || {
        result.borrow().is_some() && service.state().wireless_enabled
    });
    result.borrow_mut().take().unwrap().unwrap();
    properties(
        &daemon,
        DEVICE,
        DEVICE_IFACE,
        HashMap::from([
            ("State".into(), 50_u32.to_variant()),
            ("StateReason".into(), (50_u32, 0_u32).to_variant()),
        ]),
    );
    wait(context, || service.state().wifi_state() == 40);
    deny.set(true);
    let out = result.clone();
    service.set_networking(false, move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || result.borrow().is_some());
    assert!(result.borrow_mut().take().unwrap().is_err());
    assert!(service.state().networking_enabled);
    deny.set(false);
    let out = result.clone();
    service.set_networking(false, move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || {
        result.borrow().is_some() && !service.state().networking_enabled
    });
    result.borrow_mut().take().unwrap().unwrap();
    properties(
        &daemon,
        ROOT,
        NAME,
        HashMap::from([("PrimaryConnection".into(), path("/").to_variant())]),
    );
    wait(context, || service.state().primary.is_none());
    properties(
        &daemon,
        ROOT,
        NAME,
        HashMap::from([("PrimaryConnection".into(), path(ACTIVE).to_variant())]),
    );
    wait(context, || {
        service.state().primary.as_deref() == Some(DEVICE)
    });
    properties(
        &daemon,
        ACTIVE,
        &format!("{NAME}.Connection.Active"),
        HashMap::from([("Devices".into(), Vec::<ObjectPath>::new().to_variant())]),
    );
    wait(context, || service.state().primary.is_none());
    daemon
        .emit_signal(
            None,
            "/org/freedesktop",
            "org.freedesktop.DBus.ObjectManager",
            "InterfacesRemoved",
            Some(
                &(
                    path(DEVICE),
                    vec![DEVICE_IFACE, &format!("{DEVICE_IFACE}.Wireless")],
                )
                    .to_variant(),
            ),
        )
        .unwrap();
    properties(
        &daemon,
        ROOT,
        NAME,
        HashMap::from([("Devices".into(), Vec::<ObjectPath>::new().to_variant())]),
    );
    wait(context, || service.state().devices.is_empty());
    assert_eq!(service.state().wifi_state(), 0);
    delay.set(true);
    // Cancellation and deadlines leave the last observed state unchanged.
    let out = result.clone();
    let cancel = service.set_networking(true, move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || pending.borrow().is_some());
    assert!(!service.state().networking_enabled);
    cancel.cancel();
    wait(context, || result.borrow().is_some());
    assert!(
        result
            .borrow_mut()
            .take()
            .unwrap()
            .unwrap_err()
            .matches(gio::IOErrorEnum::Cancelled)
    );
    pending.borrow_mut().take().unwrap().return_value(None);
    let out = result.clone();
    service.set_networking(true, move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || result.borrow().is_some());
    assert!(result.borrow_mut().take().unwrap().is_err());
    assert!(!service.state().networking_enabled);
    pending.borrow_mut().take().unwrap().return_value(None);
    let out = result.clone();
    service.set_networking(true, move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || pending.borrow().is_some());
    ownership(&daemon, false);
    wait(context, || {
        !service.state().available && result.borrow().is_some()
    });
    assert!(
        result
            .borrow_mut()
            .take()
            .unwrap()
            .unwrap_err()
            .matches(gio::IOErrorEnum::Cancelled)
    );
    pending.borrow_mut().take().unwrap().return_value(None);
    delay.set(false);
    ownership(&daemon, true);
    wait(context, || {
        service.state().available && service.state().devices.len() == 1
    });
    for _ in 0..20 {
        let initializing = NetworkService::on_connection(&client);
        let weak = initializing.downgrade();
        drop(initializing);
        assert!(weak.upgrade().is_none());
    }
    context.block_on(glib::timeout_future(Duration::from_millis(30)));
    client.close_sync(gio::Cancellable::NONE).unwrap();
    wait(context, || !service.state().available);
    let weak = service.downgrade();
    drop(service);
    assert!(weak.upgrade().is_none());
    for _ in 0..20 {
        let service = NetworkService::on_connection(&client);
        let weak = service.downgrade();
        drop(service);
        assert!(weak.upgrade().is_none());
    }
    context.block_on(glib::timeout_future(Duration::from_millis(30)));
    for registration in registrations {
        daemon.unregister_object(registration).unwrap();
    }
    daemon.close_sync(gio::Cancellable::NONE).unwrap();
}
