use gio::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};
use way_shell::services::network::NetworkService;
#[path = "common/network.rs"]
mod fixture;
use fixture::*;

const DEVICE_TWO: &str = "/org/freedesktop/NetworkManager/Devices/2";
const AP_TWO: &str = "/org/freedesktop/NetworkManager/AccessPoint/2";
const AP: &str = "/org/freedesktop/NetworkManager/AccessPoint/1";
const VPN: &str = "/org/freedesktop/NetworkManager/Settings/2";
const WIREGUARD: &str = "/org/freedesktop/NetworkManager/Settings/3";
const ACTIVE_VPN: &str = "/org/freedesktop/NetworkManager/ActiveConnection/2";
const ACTIVE_WIREGUARD: &str = "/org/freedesktop/NetworkManager/ActiveConnection/3";
const SAVED: &str = "/org/freedesktop/NetworkManager/Settings/1";
const SETTINGS: &str = "/org/freedesktop/NetworkManager/Settings";
const SETTINGS_IFACE: &str = "org.freedesktop.NetworkManager.Settings";
const CONNECTION_IFACE: &str = "org.freedesktop.NetworkManager.Settings.Connection";
type Settings = HashMap<String, Properties>;
fn wifi_settings() -> Settings {
    HashMap::from([
        (
            "connection".into(),
            HashMap::from([
                ("id".into(), "fixture-wifi".to_variant()),
                (
                    "uuid".into(),
                    "89b90c4a-7bc2-4ab4-8d87-46e1b1c965f3".to_variant(),
                ),
                ("type".into(), "802-11-wireless".to_variant()),
                ("autoconnect".into(), true.to_variant()),
            ]),
        ),
        (
            "802-11-wireless".into(),
            HashMap::from([("ssid".into(), b"fixture-wifi".to_vec().to_variant())]),
        ),
    ])
}
#[test]
fn access_points_saved_connections_and_updates() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| exercise(&context, &address))
        .unwrap();
}
fn exercise(context: &glib::MainContext, address: &str) {
    let daemon = connect(address);
    let connection = connect(address);
    let mut objects = inventory();
    let wireless = objects
        .get_mut(&path(DEVICE))
        .unwrap()
        .get_mut(&format!("{DEVICE_IFACE}.Wireless"))
        .unwrap();
    wireless.insert("AccessPoints".into(), vec![path(AP)].to_variant());
    wireless.insert("ActiveAccessPoint".into(), path(AP).to_variant());
    objects.insert(
        path(AP),
        HashMap::from([(
            format!("{NAME}.AccessPoint"),
            HashMap::from([
                ("Ssid".into(), b"fixture-wifi".to_vec().to_variant()),
                ("Strength".into(), 73_u8.to_variant()),
                ("Flags".into(), 1_u32.to_variant()),
                ("WpaFlags".into(), 0_u32.to_variant()),
                ("RsnFlags".into(), 256_u32.to_variant()),
                ("Mode".into(), 2_u32.to_variant()),
                ("Frequency".into(), 5180_u32.to_variant()),
                ("HwAddress".into(), "02:00:00:00:00:01".to_variant()),
            ]),
        )]),
    );
    objects.insert(
        path(SETTINGS),
        HashMap::from([(
            SETTINGS_IFACE.into(),
            HashMap::from([
                ("Connections".into(), vec![path(SAVED)].to_variant()),
                ("CanModify".into(), true.to_variant()),
            ]),
        )]),
    );
    objects.insert(
        path(SAVED),
        HashMap::from([(
            CONNECTION_IFACE.into(),
            HashMap::from([
                ("Unsaved".into(), false.to_variant()),
                ("Flags".into(), 0_u32.to_variant()),
            ]),
        )]),
    );
    objects
        .get_mut(&path(ACTIVE))
        .unwrap()
        .get_mut(&format!("{NAME}.Connection.Active"))
        .unwrap()
        .insert("Connection".into(), path(SAVED).to_variant());
    objects
        .get_mut(&path(ROOT))
        .unwrap()
        .get_mut(NAME)
        .unwrap()
        .insert("ActiveConnections".into(), vec![path(ACTIVE)].to_variant());
    let mut second_device = objects[&path(DEVICE)].clone();
    let values = second_device.get_mut(DEVICE_IFACE).unwrap();
    values.insert("Interface".into(), "wlan1".to_variant());
    values.insert("State".into(), 30_u32.to_variant());
    values.insert("StateReason".into(), (30_u32, 0_u32).to_variant());
    values.insert("ActiveConnection".into(), path("/").to_variant());
    let wireless = second_device
        .get_mut(&format!("{DEVICE_IFACE}.Wireless"))
        .unwrap();
    wireless.insert("AccessPoints".into(), vec![path(AP_TWO)].to_variant());
    wireless.insert("ActiveAccessPoint".into(), path("/").to_variant());
    objects.insert(path(DEVICE_TWO), second_device);
    objects.insert(path(AP_TWO), objects[&path(AP)].clone());
    let root = objects.get_mut(&path(ROOT)).unwrap().get_mut(NAME).unwrap();
    root.insert(
        "Devices".into(),
        vec![path(DEVICE), path(DEVICE_TWO)].to_variant(),
    );
    root.insert(
        "AllDevices".into(),
        vec![path(DEVICE), path(DEVICE_TWO)].to_variant(),
    );
    for (saved, active, kind) in [
        (VPN, ACTIVE_VPN, "vpn"),
        (WIREGUARD, ACTIVE_WIREGUARD, "wireguard"),
    ] {
        objects.insert(path(saved), objects[&path(SAVED)].clone());
        let mut connection = objects[&path(ACTIVE)].clone();
        let values = connection
            .get_mut(&format!("{NAME}.Connection.Active"))
            .unwrap();
        values.insert("Id".into(), "fixture-tunnel".to_variant());
        values.insert("Type".into(), kind.to_variant());
        values.insert("Connection".into(), path(saved).to_variant());
        objects.insert(path(active), connection);
    }
    objects
        .get_mut(&path(SETTINGS))
        .unwrap()
        .get_mut(SETTINGS_IFACE)
        .unwrap()
        .insert(
            "Connections".into(),
            vec![path(SAVED), path(VPN), path(WIREGUARD)].to_variant(),
        );
    objects
        .get_mut(&path(ROOT))
        .unwrap()
        .get_mut(NAME)
        .unwrap()
        .insert(
            "ActiveConnections".into(),
            vec![path(ACTIVE), path(ACTIVE_VPN), path(ACTIVE_WIREGUARD)].to_variant(),
        );
    let manager = gio::DBusNodeInfo::for_xml("<node><interface name='org.freedesktop.DBus.ObjectManager'><method name='GetManagedObjects'><arg type='a{oa{sa{sv}}}' direction='out'/></method></interface></node>").unwrap();
    let mut registrations = Vec::new();
    for root in ["/org/freedesktop", ROOT] {
        let objects = objects.clone();
        registrations.push(
            daemon
                .register_object(root, &manager.interfaces()[0])
                .method_call(move |_, _, _, _, _, _, call| {
                    call.return_value(Some(&(objects.clone(),).to_variant()))
                })
                .build()
                .unwrap(),
        );
    }
    let settings = Rc::new(RefCell::new(wifi_settings()));
    let calls = Rc::new(RefCell::new(Vec::<(String, glib::Variant)>::new()));
    let deny = Rc::new(Cell::new(false));
    let delay = Rc::new(Cell::new(false));
    let delay_activation = Rc::new(Cell::new(false));
    let pending = Rc::new(RefCell::new(Vec::<gio::DBusMethodInvocation>::new()));
    let saved_node = gio::DBusNodeInfo::for_xml("<node><interface name='org.freedesktop.NetworkManager.Settings.Connection'><method name='GetSettings'><arg type='a{sa{sv}}' direction='out'/></method><method name='Update'><arg type='a{sa{sv}}' direction='in'/></method></interface></node>").unwrap();
    let value = settings.clone();
    let recorded = calls.clone();
    let rejected = deny.clone();
    let delayed = delay.clone();
    let held = pending.clone();
    registrations.push(
        daemon
            .register_object(SAVED, &saved_node.interfaces()[0])
            .method_call(move |_, _, _, _, method, args, call| {
                if method == "GetSettings" {
                    call.return_value(Some(&(value.borrow().clone(),).to_variant()));
                    return;
                }
                assert_eq!(method, "Update");
                recorded.borrow_mut().push((method.into(), args));
                if rejected.get() {
                    call.return_dbus_error(
                        "org.freedesktop.NetworkManager.PermissionDenied",
                        "fixture denied",
                    );
                } else if delayed.get() {
                    held.borrow_mut().push(call);
                } else {
                    call.return_value(None);
                }
            })
            .build()
            .unwrap(),
    );
    for (saved, kind, uuid) in [
        (VPN, "vpn", "9fcad6cc-28a2-43fd-bf8a-aafda519c416"),
        (
            WIREGUARD,
            "wireguard",
            "9fcad6cc-28a2-43fd-bf8a-aafda519c417",
        ),
    ] {
        let mut profile: Settings = HashMap::from([(
            "connection".into(),
            HashMap::from([
                ("id".into(), "fixture-tunnel".to_variant()),
                ("type".into(), kind.to_variant()),
                ("uuid".into(), uuid.to_variant()),
            ]),
        )]);
        profile.insert(
            kind.into(),
            if kind == "vpn" {
                HashMap::from([(
                    "service-type".into(),
                    "org.freedesktop.NetworkManager.openvpn".to_variant(),
                )])
            } else {
                HashMap::new()
            },
        );
        registrations.push(
            daemon
                .register_object(saved, &saved_node.interfaces()[0])
                .method_call(move |_, _, _, _, method, _, call| {
                    assert_eq!(method, "GetSettings");
                    call.return_value(Some(&(profile.clone(),).to_variant()));
                })
                .build()
                .unwrap(),
        );
    }
    let manager_node = gio::DBusNodeInfo::for_xml("<node><interface name='org.freedesktop.NetworkManager'><method name='ActivateConnection'><arg type='o' direction='in'/><arg type='o' direction='in'/><arg type='o' direction='in'/><arg type='o' direction='out'/></method><method name='AddAndActivateConnection'><arg type='a{sa{sv}}' direction='in'/><arg type='o' direction='in'/><arg type='o' direction='in'/><arg type='o' direction='out'/><arg type='o' direction='out'/></method><method name='DeactivateConnection'><arg type='o' direction='in'/></method></interface></node>").unwrap();
    let recorded = calls.clone();
    let delayed = delay_activation.clone();
    let held = pending.clone();
    registrations.push(
        daemon
            .register_object(ROOT, &manager_node.interfaces()[0])
            .method_call(move |_, _, _, _, method, args, call| {
                recorded.borrow_mut().push((method.into(), args));
                if method == "ActivateConnection" && delayed.get() {
                    held.borrow_mut().push(call);
                    return;
                }
                match method {
                    "ActivateConnection" => call.return_value(Some(&(path(ACTIVE),).to_variant())),
                    "AddAndActivateConnection" => {
                        call.return_value(Some(&(path(SAVED), path(ACTIVE)).to_variant()))
                    }
                    "DeactivateConnection" => call.return_value(None),
                    _ => panic!("Unexpected fixture method"),
                }
            })
            .build()
            .unwrap(),
    );
    let scan_node = gio::DBusNodeInfo::for_xml("<node><interface name='org.freedesktop.NetworkManager.Device.Wireless'><method name='RequestScan'><arg type='a{sv}' direction='in'/></method></interface></node>").unwrap();
    let recorded = calls.clone();
    registrations.push(
        daemon
            .register_object(DEVICE, &scan_node.interfaces()[0])
            .method_call(move |_, _, _, _, method, args, call| {
                recorded.borrow_mut().push((method.into(), args));
                call.return_value(None);
            })
            .build()
            .unwrap(),
    );
    ownership(&daemon, true);
    let service = NetworkService::on_connection(&connection);
    wait(context, || {
        let state = service.state();
        state.connections.len() == 3
            && state.access_points.len() == 2
            && state.active_connections.len() == 3
    });
    let state = service.state();
    let saved = state
        .connections
        .iter()
        .find(|saved| saved.id == SAVED)
        .unwrap();
    assert_eq!(saved.id, SAVED);
    assert_eq!(saved.name, "fixture-wifi");
    assert_eq!(saved.ssid.as_deref(), Some(b"fixture-wifi".as_slice()));
    let point = state.access_points.iter().find(|ap| ap.id == AP).unwrap();
    assert_eq!(point.id, AP);
    assert_eq!(point.device, DEVICE);
    assert_eq!(point.strength, 73);
    assert!(point.active);
    assert_eq!(
        state
            .active_connections
            .iter()
            .find(|active| active.id == ACTIVE)
            .unwrap()
            .connection
            .as_deref(),
        Some(SAVED)
    );
    settings
        .borrow_mut()
        .get_mut("connection")
        .unwrap()
        .insert("id".into(), "renamed-wifi".to_variant());
    daemon
        .emit_signal(None, SAVED, CONNECTION_IFACE, "Updated", None)
        .unwrap();
    wait(context, || {
        service
            .state()
            .connections
            .iter()
            .any(|saved| saved.id == SAVED && saved.name == "renamed-wifi")
    });
    properties(
        &daemon,
        AP,
        &format!("{NAME}.AccessPoint"),
        HashMap::from([("Strength".into(), 21_u8.to_variant())]),
    );
    wait(context, || {
        service
            .state()
            .access_points
            .iter()
            .any(|ap| ap.id == AP && ap.strength == 21)
    });
    // Saved settings are copied for the request; a rejected update must not
    // mutate the libnm cache or proceed to activation.
    let result = Rc::new(RefCell::new(None));
    let out = result.clone();
    service.join_access_point(DEVICE, AP, Some("fixture-secret"), move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || result.borrow().is_some());
    result.borrow_mut().take().unwrap().unwrap();
    assert_eq!(
        calls
            .borrow()
            .iter()
            .map(|(method, _)| method.as_str())
            .collect::<Vec<_>>(),
        ["Update", "ActivateConnection"]
    );
    let (request,) = calls.borrow()[0].1.get::<(Settings,)>().unwrap();
    assert_eq!(
        request["802-11-wireless-security"]["psk"]
            .get::<String>()
            .as_deref(),
        Some("fixture-secret")
    );
    assert!(!settings.borrow().contains_key("802-11-wireless-security"));
    calls.borrow_mut().clear();
    deny.set(true);
    let out = result.clone();
    service.join_access_point(DEVICE, AP, Some("denied-secret"), move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || result.borrow().is_some());
    assert!(result.borrow_mut().take().unwrap().is_err());
    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(calls.borrow()[0].0, "Update");
    calls.borrow_mut().clear();
    deny.set(false);
    // With no new password, activate the stored profile without rewriting it.
    let out = result.clone();
    service.join_access_point(DEVICE, AP, None, move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || result.borrow().is_some());
    result.borrow_mut().take().unwrap().unwrap();
    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(calls.borrow()[0].0, "ActivateConnection");
    calls.borrow_mut().clear();
    let out = result.clone();
    service.disconnect_device(DEVICE, move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || result.borrow().is_some());
    result.borrow_mut().take().unwrap().unwrap();
    assert_eq!(calls.borrow()[0].0, "DeactivateConnection");
    assert_eq!(
        calls.borrow()[0]
            .1
            .get::<(glib::variant::ObjectPath,)>()
            .unwrap()
            .0
            .as_str(),
        ACTIVE
    );
    calls.borrow_mut().clear();
    let out = result.clone();
    service.request_scan(DEVICE, move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || result.borrow().is_some());
    result.borrow_mut().take().unwrap().unwrap();
    assert_eq!(calls.borrow()[0].0, "RequestScan");
    calls.borrow_mut().clear();
    // Stable paths distinguish two tunnel profiles with the same display name.
    assert_eq!(
        service
            .state()
            .connections
            .iter()
            .filter(|saved| saved.name == "fixture-tunnel" && saved.is_vpn())
            .count(),
        2
    );
    for (saved, active, expected_device) in [
        (VPN, ACTIVE_VPN, DEVICE),
        (WIREGUARD, ACTIVE_WIREGUARD, "/"),
    ] {
        let out = result.clone();
        service.set_vpn(saved, true, move |reply| {
            out.replace(Some(reply));
        });
        wait(context, || result.borrow().is_some());
        result.borrow_mut().take().unwrap().unwrap();
        let (profile, device, specific) = calls.borrow()[0]
            .1
            .get::<(
                glib::variant::ObjectPath,
                glib::variant::ObjectPath,
                glib::variant::ObjectPath,
            )>()
            .unwrap();
        assert_eq!(profile.as_str(), saved);
        assert_eq!(device.as_str(), expected_device);
        assert_eq!(specific.as_str(), "/");
        calls.borrow_mut().clear();
        let out = result.clone();
        service.set_vpn(saved, false, move |reply| {
            out.replace(Some(reply));
        });
        wait(context, || result.borrow().is_some());
        result.borrow_mut().take().unwrap().unwrap();
        assert_eq!(calls.borrow()[0].0, "DeactivateConnection");
        assert_eq!(
            calls.borrow()[0]
                .1
                .get::<(glib::variant::ObjectPath,)>()
                .unwrap()
                .0
                .as_str(),
            active
        );
        calls.borrow_mut().clear();
    }
    // The same cancellation token covers a settings write and activation.
    delay.set(true);
    let out = result.clone();
    let cancel = service.join_access_point(DEVICE, AP, Some("fixture-secret"), move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || !pending.borrow().is_empty());
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
    pending.borrow_mut().pop().unwrap().return_value(None);
    context.block_on(glib::timeout_future(Duration::from_millis(20)));
    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(calls.borrow()[0].0, "Update");
    calls.borrow_mut().clear();
    delay.set(false);
    // Overlapping saved-profile writes keep the device chosen by each caller.
    delay.set(true);
    let results = Rc::new(RefCell::new(Vec::new()));
    let out = results.clone();
    service.join_access_point(DEVICE, AP, Some("first-secret"), move |reply| {
        out.borrow_mut().push(reply);
    });
    let out = results.clone();
    service.join_access_point(DEVICE_TWO, AP_TWO, Some("second-secret"), move |reply| {
        out.borrow_mut().push(reply);
    });
    wait(context, || pending.borrow().len() == 2);
    pending.borrow_mut().pop().unwrap().return_value(None);
    pending.borrow_mut().pop().unwrap().return_value(None);
    wait(context, || results.borrow().len() == 2);
    assert!(results.borrow().iter().all(Result::is_ok));
    let mut devices = calls
        .borrow()
        .iter()
        .filter(|(method, _)| method == "ActivateConnection")
        .map(|(_, args)| {
            let (connection, device, _) = args
                .get::<(
                    glib::variant::ObjectPath,
                    glib::variant::ObjectPath,
                    glib::variant::ObjectPath,
                )>()
                .unwrap();
            assert_eq!(connection.as_str(), SAVED);
            device.as_str().to_owned()
        })
        .collect::<Vec<_>>();
    devices.sort();
    assert_eq!(devices, [DEVICE, DEVICE_TWO]);
    calls.borrow_mut().clear();
    delay.set(false);
    // Cancellation remains effective after Update has completed and activation is pending.
    delay_activation.set(true);
    let out = result.clone();
    let cancel = service.join_access_point(DEVICE, AP, Some("fixture-secret"), move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || !pending.borrow().is_empty());
    assert_eq!(calls.borrow().len(), 2);
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
    pending
        .borrow_mut()
        .pop()
        .unwrap()
        .return_value(Some(&(path(ACTIVE),).to_variant()));
    context.block_on(glib::timeout_future(Duration::from_millis(20)));
    assert!(result.borrow().is_none()); // The caller is completed once.
    calls.borrow_mut().clear();
    delay_activation.set(false);
    // Raw SSID bytes survive creating a new profile, including non-UTF-8 bytes.
    let raw_ssid = vec![0xff, b'n', b'e', b'w'];
    properties(
        &daemon,
        AP,
        &format!("{NAME}.AccessPoint"),
        HashMap::from([("Ssid".into(), raw_ssid.to_variant())]),
    );
    wait(context, || {
        service
            .state()
            .access_points
            .iter()
            .any(|ap| ap.id == AP && ap.ssid == raw_ssid)
    });
    let out = result.clone();
    service.join_access_point(DEVICE, AP, Some("new-secret"), move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || result.borrow().is_some());
    result.borrow_mut().take().unwrap().unwrap();
    assert_eq!(calls.borrow()[0].0, "AddAndActivateConnection");
    let (request, device, _) = calls.borrow()[0]
        .1
        .get::<(
            Settings,
            glib::variant::ObjectPath,
            glib::variant::ObjectPath,
        )>()
        .unwrap();
    assert_eq!(
        request["802-11-wireless"]["ssid"].get::<Vec<u8>>().unwrap(),
        raw_ssid
    );
    assert_eq!(device.as_str(), DEVICE);
    calls.borrow_mut().clear();
    service.join_access_point("/missing", AP, None, |reply| {
        assert!(
            reply
                .unwrap_err()
                .matches(gio::IOErrorEnum::InvalidArgument)
        );
    });
    assert!(calls.borrow().is_empty());
    // Daemon replacement cancels a pending settings write before activation.
    delay.set(true);
    let out = result.clone();
    service.join_access_point(DEVICE_TWO, AP_TWO, Some("fixture-secret"), move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || !pending.borrow().is_empty());
    ownership(&daemon, false);
    wait(context, || !service.state().available);
    assert!(service.state().connections.is_empty());
    assert!(service.state().access_points.is_empty());
    assert!(service.state().active_connections.is_empty());
    // A caller's inventory remains owned and readable after libnm discards its
    // objects. In particular, daemon loss must not empty a retained VPN list.
    assert!(state.available);
    assert_eq!(state.connections.len(), 3);
    assert_eq!(state.active_connections.len(), 3);
    assert_eq!(
        state
            .connections
            .iter()
            .filter(|saved| saved.is_vpn())
            .count(),
        2
    );
    wait(context, || result.borrow().is_some());
    assert!(
        result
            .borrow_mut()
            .take()
            .unwrap()
            .unwrap_err()
            .matches(gio::IOErrorEnum::Cancelled)
    );
    pending.borrow_mut().pop().unwrap().return_value(None);
    delay.set(false);
    ownership(&daemon, true);
    wait(context, || {
        service.state().available && service.state().connections.len() == 3
    });
    daemon
        .emit_signal(
            None,
            "/org/freedesktop",
            "org.freedesktop.DBus.ObjectManager",
            "InterfacesRemoved",
            Some(&(path(AP), vec![format!("{NAME}.AccessPoint")]).to_variant()),
        )
        .unwrap();
    properties(
        &daemon,
        DEVICE,
        &format!("{DEVICE_IFACE}.Wireless"),
        HashMap::from([(
            "AccessPoints".into(),
            Vec::<glib::variant::ObjectPath>::new().to_variant(),
        )]),
    );
    wait(context, || {
        !service
            .state()
            .access_points
            .iter()
            .any(|point| point.id == AP)
    });
    daemon
        .emit_signal(
            None,
            "/org/freedesktop",
            "org.freedesktop.DBus.ObjectManager",
            "InterfacesRemoved",
            Some(&(path(SAVED), vec![CONNECTION_IFACE]).to_variant()),
        )
        .unwrap();
    properties(
        &daemon,
        SETTINGS,
        SETTINGS_IFACE,
        HashMap::from([(
            "Connections".into(),
            vec![path(VPN), path(WIREGUARD)].to_variant(),
        )]),
    );
    wait(context, || {
        !service
            .state()
            .connections
            .iter()
            .any(|saved| saved.id == SAVED)
    });
    delay_activation.set(true);
    let out = result.clone();
    service.activate_connection(WIREGUARD, Some(DEVICE), move |reply| {
        out.replace(Some(reply));
    });
    wait(context, || !pending.borrow().is_empty());
    let weak = service.downgrade();
    drop(service);
    assert!(weak.upgrade().is_none());
    // Subsequent updates, daemon replacement, object removal and final service
    // destruction cannot mutate or invalidate the snapshot returned earlier.
    assert_eq!(saved.name, "fixture-wifi");
    assert_eq!(saved.ssid.as_deref(), Some(b"fixture-wifi".as_slice()));
    assert_eq!(point.ssid, b"fixture-wifi");
    assert_eq!(point.strength, 73);
    for (id, kind, uuid) in [
        (VPN, "vpn", "9fcad6cc-28a2-43fd-bf8a-aafda519c416"),
        (
            WIREGUARD,
            "wireguard",
            "9fcad6cc-28a2-43fd-bf8a-aafda519c417",
        ),
    ] {
        let retained = state.connections.iter().find(|item| item.id == id).unwrap();
        assert_eq!(retained.name, "fixture-tunnel");
        assert_eq!(retained.kind, kind);
        assert_eq!(retained.uuid, uuid);
    }
    wait(context, || result.borrow().is_some());
    assert!(
        result
            .borrow_mut()
            .take()
            .unwrap()
            .unwrap_err()
            .matches(gio::IOErrorEnum::Cancelled)
    );
    pending
        .borrow_mut()
        .pop()
        .unwrap()
        .return_value(Some(&(path(ACTIVE_WIREGUARD),).to_variant()));

    context.block_on(glib::timeout_future(Duration::from_millis(20)));
    for registration in registrations {
        daemon.unregister_object(registration).unwrap();
    }
    connection.close_sync(gio::Cancellable::NONE).unwrap();
    daemon.close_sync(gio::Cancellable::NONE).unwrap();
}
