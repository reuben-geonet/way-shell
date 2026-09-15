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
use way_shell::services::power::{DeviceKind, DeviceState, PowerService, preferred_icon_name};

const NAME: &str = "org.freedesktop.UPower";
const ROOT: &str = "/org/freedesktop/UPower";
const DEVICE: &str = "org.freedesktop.UPower.Device";
const BATTERY: &str = "/org/freedesktop/UPower/devices/battery_BAT0";
const AC: &str = "/org/freedesktop/UPower/devices/line_power_ADP1";
const MOUSE: &str = "/org/freedesktop/UPower/devices/battery_mouse";
const XML: &str = r#"<node>
<interface name="org.freedesktop.UPower">
 <method name="EnumerateDevices"><arg type="ao" direction="out"/></method>
 <property name="DaemonVersion" type="s" access="read"/>
 <property name="OnBattery" type="b" access="read"/>
 <property name="LidIsPresent" type="b" access="read"/>
 <property name="LidIsClosed" type="b" access="read"/>
 <signal name="DeviceAdded"><arg type="o"/></signal>
 <signal name="DeviceRemoved"><arg type="o"/></signal>
</interface>
<interface name="org.freedesktop.UPower.Device">
 <property name="Type" type="u" access="read"/>
 <property name="NativePath" type="s" access="read"/>
 <property name="Vendor" type="s" access="read"/>
 <property name="Model" type="s" access="read"/>
 <property name="Serial" type="s" access="read"/>
 <property name="State" type="u" access="read"/>
 <property name="PowerSupply" type="b" access="read"/>
 <property name="IsPresent" type="b" access="read"/>
 <property name="IsRechargeable" type="b" access="read"/>
 <property name="Online" type="b" access="read"/>
 <property name="Percentage" type="d" access="read"/>
 <property name="TimeToEmpty" type="x" access="read"/>
 <property name="TimeToFull" type="x" access="read"/>
 <property name="IconName" type="s" access="read"/>
</interface>
<interface name="org.freedesktop.DBus.Properties">
 <method name="GetAll"><arg type="s" direction="in"/><arg type="a{sv}" direction="out"/></method>
</interface>
</node>"#;
type Properties = HashMap<String, glib::Variant>;

struct Bus {
    process: Child,
    address: String,
}
impl Bus {
    fn new() -> Self {
        let mut process = Command::new("dbus-daemon")
            .args([
                "--nofork",
                "--print-address=1",
                "--config-file=../../tests/fixtures/session-bus.conf",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(process.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        assert!(!address.is_empty());
        Self {
            process,
            address: address.trim().into(),
        }
    }
    fn connect(&self) -> gio::DBusConnection {
        gio::DBusConnection::for_address_sync(
            &self.address,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            gio::Cancellable::NONE,
        )
        .unwrap()
    }
}
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

struct Daemon {
    connection: gio::DBusConnection,
    registrations: Vec<gio::RegistrationId>,
    properties: Rc<RefCell<HashMap<String, Properties>>>,
    inventory: Rc<RefCell<Vec<ObjectPath>>>,
    hold: Rc<Cell<bool>>,
    pending: Rc<RefCell<Vec<gio::DBusMethodInvocation>>>,
    fail_inventory: Rc<Cell<bool>>,
}
impl Daemon {
    fn new(bus: &Bus) -> Self {
        let connection = bus.connect();
        let info = gio::DBusNodeInfo::for_xml(XML).unwrap();
        let inventory = Rc::new(RefCell::new(Vec::<ObjectPath>::new()));
        let properties = Rc::new(RefCell::new(HashMap::<String, Properties>::new()));
        let fail_inventory = Rc::new(Cell::new(false));
        let paths = inventory.clone();
        let fail = fail_inventory.clone();
        let root = connection
            .register_object(ROOT, &info.lookup_interface(NAME).unwrap())
            .method_call(move |_, _, _, _, method, _, invocation| {
                assert_eq!(method, "EnumerateDevices");
                if fail.get() {
                    invocation
                        .return_dbus_error("org.freedesktop.UPower.Error", "fixture unavailable");
                } else {
                    invocation.return_value(Some(&(paths.borrow().clone(),).to_variant()));
                }
            })
            .property(|_, _, _, _, name| match name {
                "DaemonVersion" => "".to_variant(),
                "OnBattery" | "LidIsPresent" | "LidIsClosed" => false.to_variant(),
                name => panic!("unexpected property {name}"),
            })
            .build()
            .unwrap();
        Self {
            connection,
            registrations: vec![root],
            properties,
            inventory,
            hold: Rc::new(Cell::new(false)),
            pending: Rc::new(RefCell::new(Vec::new())),
            fail_inventory,
        }
    }
    fn own(&self) {
        self.connection
            .call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "RequestName",
                Some(&(NAME, 0u32).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                2000,
                gio::Cancellable::NONE,
            )
            .unwrap();
    }
    fn release(&self) {
        self.connection
            .call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "ReleaseName",
                Some(&(NAME,).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                2000,
                gio::Cancellable::NONE,
            )
            .unwrap();
    }
    fn add(&mut self, path: &str, kind: u32, supply: bool) {
        let properties = HashMap::from([
            ("Type".into(), kind.to_variant()),
            ("NativePath".into(), path.to_variant()),
            ("Vendor".into(), "Fixture".to_variant()),
            ("Model".into(), "日本語".to_variant()),
            ("Serial".into(), "123".to_variant()),
            ("State".into(), 2u32.to_variant()),
            ("PowerSupply".into(), supply.to_variant()),
            ("IsPresent".into(), true.to_variant()),
            ("IsRechargeable".into(), (kind == 2).to_variant()),
            ("Online".into(), true.to_variant()),
            ("Percentage".into(), 90f64.to_variant()),
            ("TimeToEmpty".into(), 3600i64.to_variant()),
            ("TimeToFull".into(), 1200i64.to_variant()),
            ("IconName".into(), "upower-icon".to_variant()),
        ]);
        self.properties.borrow_mut().insert(path.into(), properties);
        self.inventory
            .borrow_mut()
            .push(ObjectPath::try_from(path).unwrap());
        let info = gio::DBusNodeInfo::for_xml(XML).unwrap();
        let all = self.properties.clone();
        let registration = self
            .connection
            .register_object(path, &info.lookup_interface(DEVICE).unwrap())
            .property(move |_, _, path, _, name| all.borrow()[path][name].clone())
            .build()
            .unwrap();
        self.registrations.push(registration);
        let all = self.properties.clone();
        let hold = self.hold.clone();
        let pending = self.pending.clone();
        let registration = self
            .connection
            .register_object(
                path,
                &info
                    .lookup_interface("org.freedesktop.DBus.Properties")
                    .unwrap(),
            )
            .method_call(move |_, _, path, _, method, _, invocation| {
                assert_eq!(method, "GetAll");
                if hold.get() {
                    pending.borrow_mut().push(invocation);
                } else {
                    invocation.return_value(Some(&(all.borrow()[path].clone(),).to_variant()));
                }
            })
            .build()
            .unwrap();
        self.registrations.push(registration);
    }
    fn signal(&self, name: &str, path: &str) {
        self.connection
            .emit_signal(
                None,
                ROOT,
                NAME,
                name,
                Some(&(ObjectPath::try_from(path).unwrap(),).to_variant()),
            )
            .unwrap();
    }
    fn change(&self, path: &str, name: &str, value: glib::Variant) {
        self.properties
            .borrow_mut()
            .get_mut(path)
            .unwrap()
            .insert(name.into(), value.clone());
        let changed = HashMap::from([(name.to_owned(), value)]);
        self.connection
            .emit_signal(
                None,
                path,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                Some(&(DEVICE, changed, Vec::<String>::new()).to_variant()),
            )
            .unwrap();
    }
    fn answer_pending(&self) {
        for invocation in self.pending.take() {
            let properties = self.properties.borrow()[invocation.object_path().as_str()].clone();
            invocation.return_value(Some(&(properties,).to_variant()));
        }
    }
}
impl Drop for Daemon {
    fn drop(&mut self) {
        for registration in self.registrations.drain(..) {
            self.connection.unregister_object(registration).unwrap();
        }
        let _ = self.connection.close_sync(gio::Cancellable::NONE);
    }
}

fn wait(context: &glib::MainContext, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "UPower fixture did not reach expected state"
        );
        context.block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}

#[test]
fn inventory_changes_removal_restart_pending_requests_and_cleanup() {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let bus = Bus::new();
            let connection = bus.connect();
            let service = PowerService::on_connection(&connection);
            let changes = Rc::new(Cell::new(0));
            let count = changes.clone();
            let weak = service.downgrade();
            service.connect_local("changed", false, move |_| {
                if weak
                    .upgrade()
                    .is_some_and(|service| service.state().available)
                {
                    count.set(count.get() + 1);
                }
                None
            });
            context.block_on(glib::timeout_future(Duration::from_millis(20)));
            assert!(!service.state().available);
            assert!(service.primary_device().is_none());
            let mut daemon = Daemon::new(&bus);
            daemon.own();
            wait(&context, || service.state().available);
            assert!(
                changes.get() > 0,
                "availability must notify with an empty inventory"
            );
            daemon.add(MOUSE, 2, false);
            daemon.add(AC, 1, true);
            daemon.add(BATTERY, 2, true);
            for path in [MOUSE, AC, BATTERY] {
                daemon.signal("DeviceAdded", path);
            }
            wait(&context, || service.devices().len() == 3);
            let primary = service.primary_device().unwrap();
            assert_eq!(primary.path, BATTERY);
            assert_eq!(primary.model, "日本語");
            assert_eq!(primary.percentage, Some(90.0));
            assert_eq!(primary.preferred_icon_name(), "battery-level-90-symbolic");
            daemon
                .connection
                .emit_signal(
                    None,
                    ROOT,
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    Some(
                        &(
                            NAME,
                            HashMap::from([
                                ("OnBattery", true.to_variant()),
                                ("LidIsClosed", true.to_variant()),
                            ]),
                            Vec::<String>::new(),
                        )
                            .to_variant(),
                    ),
                )
                .unwrap();
            wait(&context, || {
                service.state().on_battery && service.state().lid_is_closed
            });
            daemon.change(BATTERY, "IsPresent", false.to_variant());
            wait(&context, || {
                service.primary_device().unwrap().kind == DeviceKind::LinePower
            });
            daemon.change(BATTERY, "IsPresent", true.to_variant());
            wait(&context, || {
                service.primary_device().unwrap().kind == DeviceKind::Battery
            });
            daemon.change(BATTERY, "Percentage", 52.5f64.to_variant());
            daemon.change(BATTERY, "State", 1u32.to_variant());
            wait(&context, || {
                service.primary_device().unwrap().state == DeviceState::Charging
            });
            assert_eq!(service.primary_device().unwrap().percentage, Some(52.5));
            assert_eq!(service.primary_device().unwrap().time_to_full, 1200);
            daemon.change(BATTERY, "Percentage", f64::NAN.to_variant());
            daemon.change(BATTERY, "TimeToEmpty", (-12i64).to_variant());
            wait(&context, || {
                let device = service.primary_device().unwrap();
                device.percentage.is_none() && device.time_to_empty == 0
            });
            assert_eq!(service.primary_device().unwrap().time_to_empty, 0);
            daemon.signal("DeviceRemoved", BATTERY);
            wait(&context, || service.devices().len() == 2);
            assert_eq!(
                service.primary_device().unwrap().kind,
                DeviceKind::LinePower
            );
            // Hold proxy construction while removing and immediately re-adding the same path.
            daemon.hold.set(true);
            daemon.signal("DeviceAdded", BATTERY);
            wait(&context, || !daemon.pending.borrow().is_empty());
            daemon.signal("DeviceRemoved", BATTERY);
            daemon.signal("DeviceAdded", BATTERY);
            wait(&context, || daemon.pending.borrow().len() == 2);
            daemon.hold.set(false);
            daemon.answer_pending();
            wait(&context, || service.devices().len() == 3);
            // A transient per-device GetAll failure retries without losing other devices.
            daemon.signal("DeviceRemoved", BATTERY);
            wait(&context, || service.devices().len() == 2);
            daemon.hold.set(true);
            daemon.signal("DeviceAdded", BATTERY);
            wait(&context, || !daemon.pending.borrow().is_empty());
            daemon.hold.set(false);
            for invocation in daemon.pending.take() {
                invocation.return_dbus_error(
                    "org.freedesktop.UPower.Error",
                    "device temporarily unavailable",
                );
            }
            wait(&context, || service.devices().len() == 3);
            assert!(service.state().available);
            // Losing the daemon clears every snapshot and reconnects when it returns.
            daemon.release();
            wait(&context, || {
                !service.state().available && service.devices().is_empty()
            });
            daemon.fail_inventory.set(true);
            daemon.own();
            context.block_on(glib::timeout_future(Duration::from_millis(50)));
            assert!(!service.state().available);
            daemon.fail_inventory.set(false);
            wait(&context, || {
                service.state().available && service.devices().len() == 3
            });
            // Drop while device construction has an outstanding remote response.
            daemon.signal("DeviceRemoved", BATTERY);
            wait(&context, || service.devices().len() == 2);
            daemon.hold.set(true);
            daemon.signal("DeviceAdded", BATTERY);
            wait(&context, || !daemon.pending.borrow().is_empty());
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
            daemon.hold.set(false);
            daemon.answer_pending();
            for _ in 0..20 {
                let service = PowerService::on_connection(&connection);
                let weak = service.downgrade();
                drop(service);
                assert!(weak.upgrade().is_none());
            }
            context.block_on(glib::timeout_future(Duration::from_millis(50)));
            connection.close_sync(gio::Cancellable::NONE).unwrap();
        })
        .unwrap();
}

#[test]
fn icon_thresholds_and_invalid_values() {
    for percentage in [90., 90.1, 100.] {
        assert_eq!(
            preferred_icon_name(true, DeviceState::Discharging, Some(percentage)),
            "battery-level-90-symbolic"
        );
        assert_eq!(
            preferred_icon_name(true, DeviceState::Charging, Some(percentage)),
            "battery-level-90-charging-symbolic"
        );
        assert_eq!(
            preferred_icon_name(true, DeviceState::FullyCharged, Some(percentage)),
            "battery-full-charging-symbolic"
        );
    }
    assert_eq!(
        preferred_icon_name(true, DeviceState::Charging, Some(10.)),
        "battery-caution-charging-symbolic"
    );
    assert_eq!(
        preferred_icon_name(true, DeviceState::Discharging, Some(10.1)),
        "battery-level-10-symbolic"
    );
    assert_eq!(
        preferred_icon_name(true, DeviceState::Unknown, Some(f64::NAN)),
        "battery-missing-symbolic"
    );
    for percentage in [-1., 101., f64::INFINITY] {
        assert_eq!(
            preferred_icon_name(true, DeviceState::Unknown, Some(percentage)),
            "battery-missing-symbolic"
        );
    }
    assert_eq!(
        preferred_icon_name(false, DeviceState::Unknown, None),
        "ac-adapter-symbolic"
    );
}
