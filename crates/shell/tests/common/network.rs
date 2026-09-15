use gio::prelude::*;
use glib::variant::ObjectPath;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
pub const NAME: &str = "org.freedesktop.NetworkManager";
pub const ROOT: &str = "/org/freedesktop/NetworkManager";
pub const DEVICE: &str = "/org/freedesktop/NetworkManager/Devices/1";
pub const ACTIVE: &str = "/org/freedesktop/NetworkManager/ActiveConnection/1";
pub const DEVICE_IFACE: &str = "org.freedesktop.NetworkManager.Device";
pub fn path(value: &str) -> ObjectPath {
    ObjectPath::try_from(value).unwrap()
}
pub type Properties = HashMap<String, glib::Variant>;
pub type Managed = HashMap<ObjectPath, HashMap<String, Properties>>;
pub struct Bus(Child);
impl Bus {
    pub fn start() -> (Self, String) {
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
        (Self(child), address.trim().to_owned())
    }
}
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
pub fn connect(address: &str) -> gio::DBusConnection {
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
pub fn wait(context: &glib::MainContext, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "NetworkManager fixture timed out"
        );
        context.block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}
pub fn ownership(connection: &gio::DBusConnection, acquire: bool) {
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
pub fn properties(
    connection: &gio::DBusConnection,
    object: &str,
    interface: &str,
    values: Properties,
) {
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
pub fn inventory() -> Managed {
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
