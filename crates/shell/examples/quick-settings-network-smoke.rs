//! Real libnm and GTK controls against a private NetworkManager fixture.
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};
use way_shell::{
    services::network::NetworkService,
    ui::{
        quick_settings::{grid::Grid, network::NetworkControls},
        window::{LayerWindow, WindowRole},
    },
};
#[path = "../tests/common/network.rs"]
mod fixture;
use fixture::*;

const WIFI2: &str = "/org/freedesktop/NetworkManager/Devices/2";
const WIRED: &str = "/org/freedesktop/NetworkManager/Devices/3";
const AP: &str = "/org/freedesktop/NetworkManager/AccessPoint/1";
const STRONG: &str = "/org/freedesktop/NetworkManager/AccessPoint/2";
const OPEN: &str = "/org/freedesktop/NetworkManager/AccessPoint/3";
const AP2: &str = "/org/freedesktop/NetworkManager/AccessPoint/4";
const SAVED: &str = "/org/freedesktop/NetworkManager/Settings/1";
const VPN: &str = "/org/freedesktop/NetworkManager/Settings/2";
const WG: &str = "/org/freedesktop/NetworkManager/Settings/3";
const SETTINGS: &str = "/org/freedesktop/NetworkManager/Settings";
const SAVED_IFACE: &str = "org.freedesktop.NetworkManager.Settings.Connection";
const WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const SECRET: &str = "fixture-password-must-never-be-logged";
type Settings = HashMap<String, Properties>;
struct Fake {
    daemon: gio::DBusConnection,
    objects: Rc<RefCell<Managed>>,
    registrations: Vec<gio::RegistrationId>,
    calls: Rc<RefCell<Vec<(String, glib::Variant)>>>,
    deny: Rc<Cell<bool>>,
    delay: Rc<Cell<bool>>,
    pending: Rc<RefCell<Vec<gio::DBusMethodInvocation>>>,
}
impl Fake {
    fn new(address: &str) -> Self {
        let daemon = connect(address);
        let mut objects = inventory();
        let mut wireless_two = objects[&path(DEVICE)].clone();
        wireless_two
            .get_mut(DEVICE_IFACE)
            .unwrap()
            .insert("Interface".into(), "wlan1".to_variant());
        wireless_two
            .get_mut(DEVICE_IFACE)
            .unwrap()
            .insert("State".into(), 30_u32.to_variant());
        wireless_two
            .get_mut(DEVICE_IFACE)
            .unwrap()
            .insert("ActiveConnection".into(), path("/").to_variant());
        objects.insert(path(WIFI2), wireless_two);
        let mut wired = objects[&path(DEVICE)].clone();
        wired.remove(WIRELESS);
        wired.insert(
            format!("{DEVICE_IFACE}.Wired"),
            HashMap::from([("Carrier".into(), true.to_variant())]),
        );
        let values = wired.get_mut(DEVICE_IFACE).unwrap();
        values.insert("Interface".into(), "eth0".to_variant());
        values.insert("DeviceType".into(), 1_u32.to_variant());
        values.insert("State".into(), 30_u32.to_variant());
        values.insert("ActiveConnection".into(), path("/").to_variant());
        objects.insert(path(WIRED), wired);
        for (id, ssid, strength, secure) in [
            (AP, b"Saved Wi-Fi".as_slice(), 20_u8, true),
            (STRONG, b"Saved Wi-Fi", 90, true),
            (OPEN, b"Open Wi-Fi", 50, false),
            (AP2, b"Other Wi-Fi", 60, true),
        ] {
            objects.insert(
                path(id),
                HashMap::from([(
                    format!("{NAME}.AccessPoint"),
                    HashMap::from([
                        ("Ssid".into(), ssid.to_vec().to_variant()),
                        ("Strength".into(), strength.to_variant()),
                        ("Flags".into(), u32::from(secure).to_variant()),
                        ("WpaFlags".into(), 0_u32.to_variant()),
                        (
                            "RsnFlags".into(),
                            if secure { 256_u32 } else { 0_u32 }.to_variant(),
                        ),
                        ("Mode".into(), 2_u32.to_variant()),
                        ("Frequency".into(), 5180_u32.to_variant()),
                        ("HwAddress".into(), "02:00:00:00:00:01".to_variant()),
                    ]),
                )]),
            );
        }
        for (device, points, active) in [
            (DEVICE, vec![path(AP), path(STRONG), path(OPEN)], AP),
            (WIFI2, vec![path(AP2)], "/"),
        ] {
            let wireless = objects
                .get_mut(&path(device))
                .unwrap()
                .get_mut(WIRELESS)
                .unwrap();
            wireless.insert("AccessPoints".into(), points.to_variant());
            wireless.insert("ActiveAccessPoint".into(), path(active).to_variant());
        }
        let values = objects.get_mut(&path(ROOT)).unwrap().get_mut(NAME).unwrap();
        values.insert(
            "Devices".into(),
            vec![path(DEVICE), path(WIFI2), path(WIRED)].to_variant(),
        );
        values.insert(
            "AllDevices".into(),
            vec![path(DEVICE), path(WIFI2), path(WIRED)].to_variant(),
        );
        objects.insert(
            path(SETTINGS),
            HashMap::from([(
                format!("{NAME}.Settings"),
                HashMap::from([
                    (
                        "Connections".into(),
                        vec![path(SAVED), path(VPN), path(WG)].to_variant(),
                    ),
                    ("CanModify".into(), true.to_variant()),
                ]),
            )]),
        );
        for id in [SAVED, VPN, WG] {
            objects.insert(
                path(id),
                HashMap::from([(
                    SAVED_IFACE.into(),
                    HashMap::from([
                        ("Unsaved".into(), false.to_variant()),
                        ("Flags".into(), 0_u32.to_variant()),
                    ]),
                )]),
            );
        }
        let objects = Rc::new(RefCell::new(objects));
        let calls = Rc::new(RefCell::new(Vec::new()));
        let deny = Rc::new(Cell::new(false));
        let delay = Rc::new(Cell::new(false));
        let pending = Rc::new(RefCell::new(Vec::new()));
        let mut registrations = Vec::new();
        let node=gio::DBusNodeInfo::for_xml("<node><interface name='org.freedesktop.DBus.ObjectManager'><method name='GetManagedObjects'><arg type='a{oa{sa{sv}}}' direction='out'/></method></interface></node>").unwrap();
        for root in ["/org/freedesktop", ROOT] {
            let objects = objects.clone();
            registrations.push(
                daemon
                    .register_object(root, &node.interfaces()[0])
                    .method_call(move |_, _, _, _, _, _, call| {
                        call.return_value(Some(&(objects.borrow().clone(),).to_variant()))
                    })
                    .build()
                    .unwrap(),
            );
        }
        let node=gio::DBusNodeInfo::for_xml("<node><interface name='org.freedesktop.NetworkManager'><method name='Enable'><arg type='b' direction='in'/></method><method name='GetPermissions'><arg type='a{ss}' direction='out'/></method><property name='WirelessEnabled' type='b' access='readwrite'/><method name='ActivateConnection'><arg type='o' direction='in'/><arg type='o' direction='in'/><arg type='o' direction='in'/><arg type='o' direction='out'/></method><method name='AddAndActivateConnection'><arg type='a{sa{sv}}' direction='in'/><arg type='o' direction='in'/><arg type='o' direction='in'/><arg type='o' direction='out'/><arg type='o' direction='out'/></method><method name='DeactivateConnection'><arg type='o' direction='in'/></method></interface></node>").unwrap();
        let recorded = calls.clone();
        let rejected = deny.clone();
        let updated = objects.clone();
        let get = objects.clone();
        let set = objects.clone();
        let properties_recorded = calls.clone();
        registrations.push(
            daemon
                .register_object(ROOT, &node.interfaces()[0])
                .method_call(move |connection, _, _, _, method, args, call| {
                    if method == "GetPermissions" {
                        call.return_value(Some(&(HashMap::<String, String>::new(),).to_variant()));
                        return;
                    }
                    recorded.borrow_mut().push((method.into(), args.clone()));
                    if rejected.get() {
                        call.return_dbus_error(
                            "org.freedesktop.NetworkManager.PermissionDenied",
                            "fixture request denied",
                        );
                        return;
                    }
                    match method {
                        "Enable" => {
                            let (enabled,) = args.get::<(bool,)>().unwrap();
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
                            call.return_value(None);
                        }
                        "ActivateConnection" => {
                            call.return_value(Some(&(path(ACTIVE),).to_variant()))
                        }
                        "AddAndActivateConnection" => {
                            call.return_value(Some(&(path(SAVED), path(ACTIVE)).to_variant()))
                        }
                        "DeactivateConnection" => call.return_value(None),
                        _ => panic!("unexpected manager request"),
                    }
                })
                .property(move |_, _, _, _, name| get.borrow()[&path(ROOT)][NAME][name].clone())
                .set_property(move |connection, _, _, _, name, value| {
                    properties_recorded
                        .borrow_mut()
                        .push(("WirelessEnabled".into(), value.clone()));
                    set.borrow_mut()
                        .get_mut(&path(ROOT))
                        .unwrap()
                        .get_mut(NAME)
                        .unwrap()
                        .insert(name.into(), value.clone());
                    properties(
                        &connection,
                        ROOT,
                        NAME,
                        HashMap::from([(name.into(), value)]),
                    );
                    true
                })
                .build()
                .unwrap(),
        );
        let saved_node=gio::DBusNodeInfo::for_xml("<node><interface name='org.freedesktop.NetworkManager.Settings.Connection'><method name='GetSettings'><arg type='a{sa{sv}}' direction='out'/></method><method name='Update'><arg type='a{sa{sv}}' direction='in'/></method></interface></node>").unwrap();
        for (id, kind, uuid) in [
            (
                SAVED,
                "802-11-wireless",
                "89b90c4a-7bc2-4ab4-8d87-46e1b1c965f3",
            ),
            (VPN, "vpn", "9fcad6cc-28a2-43fd-bf8a-aafda519c416"),
            (WG, "wireguard", "9fcad6cc-28a2-43fd-bf8a-aafda519c417"),
        ] {
            let mut settings: Settings = HashMap::from([(
                "connection".into(),
                HashMap::from([
                    (
                        "id".into(),
                        if id == SAVED {
                            "Saved Wi-Fi"
                        } else {
                            "Same tunnel name"
                        }
                        .to_variant(),
                    ),
                    ("type".into(), kind.to_variant()),
                    ("uuid".into(), uuid.to_variant()),
                ]),
            )]);
            settings.insert(
                kind.into(),
                match kind {
                    "802-11-wireless" => {
                        HashMap::from([("ssid".into(), b"Saved Wi-Fi".to_vec().to_variant())])
                    }
                    "vpn" => HashMap::from([(
                        "service-type".into(),
                        "org.freedesktop.NetworkManager.openvpn".to_variant(),
                    )]),
                    _ => HashMap::new(),
                },
            );
            let recorded = calls.clone();
            let rejected = deny.clone();
            registrations.push(
                daemon
                    .register_object(id, &saved_node.interfaces()[0])
                    .method_call(move |_, _, _, _, method, args, call| {
                        if method == "GetSettings" {
                            call.return_value(Some(&(settings.clone(),).to_variant()));
                            return;
                        }
                        recorded.borrow_mut().push((method.into(), args));
                        if rejected.get() {
                            call.return_dbus_error(
                                "org.freedesktop.NetworkManager.PermissionDenied",
                                "fixture request denied",
                            );
                        } else {
                            call.return_value(None);
                        }
                    })
                    .build()
                    .unwrap(),
            );
        }
        let node=gio::DBusNodeInfo::for_xml("<node><interface name='org.freedesktop.NetworkManager.Device.Wireless'><method name='RequestScan'><arg type='a{sv}' direction='in'/></method></interface></node>").unwrap();
        for device in [DEVICE, WIFI2] {
            let recorded = calls.clone();
            let delayed = delay.clone();
            let held = pending.clone();
            registrations.push(
                daemon
                    .register_object(device, &node.interfaces()[0])
                    .method_call(move |_, _, _, _, method, args, call| {
                        recorded
                            .borrow_mut()
                            .push((format!("{method}:{device}"), args));
                        if delayed.get() {
                            held.borrow_mut().push(call);
                        } else {
                            call.return_value(None);
                        }
                    })
                    .build()
                    .unwrap(),
            );
        }
        ownership(&daemon, true);
        Self {
            daemon,
            objects,
            registrations,
            calls,
            deny,
            delay,
            pending,
        }
    }
    fn update(&self, object: &str, interface: &str, values: Properties) {
        self.objects
            .borrow_mut()
            .get_mut(&path(object))
            .unwrap()
            .get_mut(interface)
            .unwrap()
            .extend(values.clone());
        properties(&self.daemon, object, interface, values);
    }
}
impl Drop for Fake {
    fn drop(&mut self) {
        for call in self.pending.take() {
            call.return_dbus_error("org.freedesktop.DBus.Error.Cancelled", "fixture stopped");
        }
        for registration in self.registrations.drain(..) {
            self.daemon.unregister_object(registration).unwrap();
        }
        let _ = self.daemon.close_sync(gio::Cancellable::NONE);
    }
}
fn descendants(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut out = vec![root.as_ref().clone()];
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        out.extend(descendants(&widget));
    }
    out
}
fn find<T: IsA<gtk::Widget> + glib::object::ObjectType>(root: &impl IsA<gtk::Widget>) -> T {
    descendants(root)
        .into_iter()
        .find_map(|widget| widget.downcast::<T>().ok())
        .unwrap()
}
fn wifi_row(root: &impl IsA<gtk::Widget>, name: &str) -> gtk::Widget {
    descendants(root)
        .into_iter()
        .find(|widget| {
            widget.has_css_class("quick-settings-menu-option-wifi")
                && descendants(widget)
                    .into_iter()
                    .filter_map(|child| child.downcast::<gtk::Label>().ok())
                    .any(|label| label.text() == name)
        })
        .unwrap()
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    adw::init()?;
    let context = glib::MainContext::default();
    context.with_thread_default(|| {
        let (_bus, address) = Bus::start();
        let fixture = Fake::new(&address);
        let connection = connect(&address);
        let service = NetworkService::on_connection(&connection);
        wait(&context, || {
            service.state().devices.len() == 3
                && service.state().connections.len() == 3
                && service.state().access_points.len() == 4
        });
        let controls = NetworkControls::new(service.clone());
        let grid = Grid::new(|_| {});
        let buttons = || {
            let mut buttons = controls.buttons();
            buttons.push(controls.airplane());
            buttons
        };
        grid.set_buttons(buttons());
        let weak_grid = Rc::downgrade(&grid);
        let weak_controls = Rc::downgrade(&controls);
        controls.on_changed(move || {
            if let (Some(grid), Some(controls)) = (weak_grid.upgrade(), weak_controls.upgrade()) {
                let mut buttons = controls.buttons();
                buttons.push(controls.airplane());
                grid.set_buttons(buttons);
            }
        });
        let window = LayerWindow::new(WindowRole::QuickSettings, None).unwrap();
        window.window().set_default_size(460, 560);
        window.window().set_content(Some(grid.widget()));
        window.present();
        wait(&context, || window.window().is_mapped());
        let state = service.state();
        let index = state
            .devices
            .iter()
            .filter(|device| matches!(device.kind, 1 | 2))
            .position(|device| device.id == DEVICE)
            .unwrap();
        let wifi = controls.buttons()[index].clone();
        let second_index = state
            .devices
            .iter()
            .filter(|device| matches!(device.kind, 1 | 2))
            .position(|device| device.id == WIFI2)
            .unwrap();
        let second_wifi = controls.buttons()[second_index].clone();
        assert_eq!(controls.buttons().len(), 4);
        wifi.toggle_menu();
        wait(&context, || {
            fixture
                .calls
                .borrow()
                .iter()
                .any(|(name, _)| name == &format!("RequestScan:{DEVICE}"))
        });
        let menu = wifi.revealer().child().unwrap();
        let rows = descendants(&menu)
            .into_iter()
            .filter(|widget| widget.has_css_class("quick-settings-menu-option-wifi"))
            .count();
        assert_eq!(rows, 2);
        let saved = wifi_row(&menu, "Saved Wi-Fi");
        let selected = descendants(&saved)
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Image>().ok())
            .find(|image| image.icon_name().as_deref() == Some("object-select-symbolic"))
            .unwrap();
        assert!(selected.is_visible());
        find::<gtk::Button>(&saved).emit_clicked();
        let password = find::<gtk::PasswordEntry>(&saved);
        let reveal = find::<gtk::Revealer>(&saved);
        assert!(reveal.reveals_child());
        password.set_text(SECRET);
        password.emit_by_name::<()>("activate", &[]);
        wait(&context, || {
            fixture
                .calls
                .borrow()
                .iter()
                .any(|(method, _)| method == "ActivateConnection")
        });
        assert!(password.text().is_empty());
        assert!(!reveal.reveals_child());
        let request = fixture
            .calls
            .borrow()
            .iter()
            .find(|(method, _)| method == "Update")
            .unwrap()
            .1
            .get::<(Settings,)>()
            .unwrap()
            .0;
        assert!(
            request["802-11-wireless-security"]["psk"]
                .get::<String>()
                .is_some_and(|value| value == SECRET)
        );
        fixture.calls.borrow_mut().clear();
        let open = wifi_row(&menu, "Open Wi-Fi");
        find::<gtk::Button>(&open).emit_clicked();
        wait(&context, || {
            fixture
                .calls
                .borrow()
                .iter()
                .any(|(method, _)| method == "AddAndActivateConnection")
        });
        // An external radio change must be the source of the next toggle.
        fixture.update(
            ROOT,
            NAME,
            HashMap::from([("WirelessEnabled".into(), false.to_variant())]),
        );
        wait(&context, || !service.state().wireless_enabled);
        fixture.calls.borrow_mut().clear();
        wifi.toggle().emit_clicked();
        wait(&context, || service.state().wireless_enabled);
        wait(&context, || wifi.toggle().is_sensitive());
        assert!(fixture.calls.borrow().iter().any(
            |(method, value)| method == "WirelessEnabled" && value.get::<bool>() == Some(true)
        ));
        // Every Wi-Fi menu targets its own device, and only one menu is expanded.
        fixture.calls.borrow_mut().clear();
        second_wifi.toggle_menu();
        wait(&context, || {
            fixture
                .calls
                .borrow()
                .iter()
                .any(|(method, _)| method == &format!("RequestScan:{WIFI2}"))
        });
        assert!(!wifi.revealer().reveals_child());
        let second_menu = second_wifi.revealer().child().unwrap();
        let other = wifi_row(&second_menu, "Other Wi-Fi");
        find::<gtk::Button>(&other).emit_clicked();
        let other_password = find::<gtk::PasswordEntry>(&other);
        other_password.set_text(SECRET);
        controls.hide_passwords();
        assert!(other_password.text().is_empty());
        // Request failures are visible without logging the password or request.
        fixture.deny.set(true);
        wifi.toggle_menu();
        let saved = wifi_row(&menu, "Saved Wi-Fi");
        find::<gtk::Button>(&saved).emit_clicked();
        let password = find::<gtk::PasswordEntry>(&saved);
        password.set_text(SECRET);
        password.emit_by_name::<()>("activate", &[]);
        let failure = descendants(&menu)
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
            .find(|button| button.label().as_deref() == Some("Failed to connect to network"))
            .unwrap();
        wait(&context, || failure.tooltip_text().is_some());
        assert!(password.text().is_empty());
        fixture.deny.set(false);
        let vpn = controls.buttons().last().unwrap().clone();
        vpn.toggle_menu();
        let vpn_menu = vpn.revealer().child().unwrap();
        let rows: Vec<adw::SwitchRow> = descendants(&vpn_menu)
            .into_iter()
            .filter_map(|widget| widget.downcast().ok())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title(), rows[1].title());
        let profiles: Vec<_> = service
            .state()
            .connections
            .into_iter()
            .filter(|connection| connection.is_vpn())
            .collect();
        for (row, profile) in rows.iter().zip(profiles) {
            fixture.calls.borrow_mut().clear();
            row.set_active(true);
            wait(&context, || {
                fixture
                    .calls
                    .borrow()
                    .iter()
                    .any(|(method, _)| method == "ActivateConnection")
            });
            let args = fixture
                .calls
                .borrow()
                .iter()
                .find(|(method, _)| method == "ActivateConnection")
                .unwrap()
                .1
                .get::<(
                    glib::variant::ObjectPath,
                    glib::variant::ObjectPath,
                    glib::variant::ObjectPath,
                )>()
                .unwrap();
            assert_eq!(args.0.as_str(), profile.id);
            assert_eq!(
                args.1.as_str(),
                if profile.kind == "wireguard" {
                    "/"
                } else {
                    DEVICE
                }
            );
            wait(&context, || row.is_sensitive());
        }
        controls.airplane().toggle().emit_clicked();
        wait(&context, || !service.state().networking_enabled);
        assert!(rows.iter().all(|row| !row.is_sensitive()));
        wait(&context, || controls.airplane().toggle().is_sensitive());
        controls.airplane().toggle().emit_clicked();
        wait(&context, || service.state().networking_enabled);
        ownership(&fixture.daemon, false);
        wait(&context, || {
            !service.state().available && controls.buttons().is_empty()
        });
        let count = fixture.calls.borrow().len();
        wifi.toggle().emit_clicked();
        context.block_on(glib::timeout_future(Duration::from_millis(30)));
        assert_eq!(fixture.calls.borrow().len(), count);
        ownership(&fixture.daemon, true);
        wait(&context, || {
            service.state().available && controls.buttons().len() == 4
        });
        let index = service
            .state()
            .devices
            .iter()
            .filter(|device| matches!(device.kind, 1 | 2))
            .position(|device| device.id == DEVICE)
            .unwrap();
        let recovered = controls.buttons()[index].clone();
        assert!(!Rc::ptr_eq(&recovered, &wifi));
        let wifi = recovered;
        let menu = wifi.revealer().child().unwrap();
        // Late scan completion after closing must not stop the newer scan.
        fixture.delay.set(true);
        wifi.toggle_menu();
        wait(&context, || !fixture.pending.borrow().is_empty());
        grid.hide_menus();
        wifi.toggle_menu();
        wait(&context, || fixture.pending.borrow().len() == 2);
        let spinner = descendants(&menu)
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Spinner>().ok())
            .find(|spinner| spinner.is_spinning())
            .unwrap();
        let old = fixture.pending.borrow_mut().remove(0);
        old.return_value(None);
        context.block_on(glib::timeout_future(Duration::from_millis(40)));
        assert!(spinner.is_spinning());
        let retained = wifi.toggle().clone();
        let held = failure.clone();
        controls.stop();
        assert!(other_password.text().is_empty());
        for call in fixture.pending.take() {
            call.return_value(None);
        }
        context.block_on(glib::timeout_future(Duration::from_millis(40)));
        assert!(!spinner.is_spinning());
        let count = fixture.calls.borrow().len();
        retained.emit_clicked();
        held.emit_clicked();
        rows[0].set_active(true);
        context.block_on(glib::timeout_future(Duration::from_millis(40)));
        assert_eq!(fixture.calls.borrow().len(), count);
        let weak = Rc::downgrade(&controls);
        drop(controls);
        assert!(weak.upgrade().is_none());
        grid.stop();
        window.close();
        service.stop();
        connection.close_sync(gio::Cancellable::NONE).unwrap();
    })?;
    println!(
        "quick-settings network UI, libnm actions, secrets, cancellation and ownership passed"
    );
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("WAY_SHELL_NETWORK_UI_CHILD").is_some() {
        return run();
    }
    let output = std::process::Command::new(std::env::current_exe()?)
        .env("WAY_SHELL_NETWORK_UI_CHILD", "1")
        .env("G_MESSAGES_DEBUG", "all")
        .output()?;
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains(SECRET)
            && !String::from_utf8_lossy(&output.stderr).contains(SECRET),
        "password appeared in application logs"
    );
    use std::io::Write;
    std::io::stdout().write_all(&output.stdout)?;
    std::io::stderr().write_all(&output.stderr)?;
    assert!(output.status.success(), "network GTK fixture failed");
    Ok(())
}
