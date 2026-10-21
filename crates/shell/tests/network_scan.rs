use gio::prelude::*;
use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Duration};
use way_shell::services::network::NetworkService;
#[path = "common/network.rs"]
mod fixture;
use fixture::*;

const SECOND: &str = "/org/freedesktop/NetworkManager/Devices/2";
#[test]
fn scan_waits_for_completion_and_cleans_up_every_exit() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context.with_thread_default(|| {
        let daemon = connect(&address);
        let connection = connect(&address);
        let mut objects = inventory();
        objects.insert(path(SECOND), objects[&path(DEVICE)].clone());
        for property in ["Devices", "AllDevices"] {
            objects.get_mut(&path(ROOT)).unwrap().get_mut(NAME).unwrap()
                .insert(property.into(), vec![path(DEVICE), path(SECOND)].to_variant());
        }
        let manager = gio::DBusNodeInfo::for_xml("<node><interface name='org.freedesktop.DBus.ObjectManager'><method name='GetManagedObjects'><arg type='a{oa{sa{sv}}}' direction='out'/></method></interface></node>").unwrap();
        let mut registrations = Vec::new();
        for root in ["/org/freedesktop", ROOT] {
            let objects = objects.clone();
            registrations.push(daemon.register_object(root, &manager.interfaces()[0])
                .method_call(move |_, _, _, _, _, _, call| call.return_value(Some(&(objects.clone(),).to_variant())))
                .build().unwrap());
        }
        let node = gio::DBusNodeInfo::for_xml("<node><interface name='org.freedesktop.NetworkManager.Device.Wireless'><method name='RequestScan'><arg type='a{sv}' direction='in'/></method></interface></node>").unwrap();
        let pending = Rc::new(RefCell::new(Vec::<gio::DBusMethodInvocation>::new()));
        let held = pending.clone();
        registrations.push(daemon.register_object(DEVICE, &node.interfaces()[0])
            .method_call(move |_, _, _, _, _, _, call| held.borrow_mut().push(call)).build().unwrap());
        ownership(&daemon, true);
        let service = NetworkService::on_connection(&connection);
        wait(&context, || service.state().devices.len() == 2);
        assert_eq!(service.state().devices[0].last_scan, None);
        let replies = Rc::new(RefCell::new(Vec::new()));
        let start = || {
            let replies = replies.clone();
            let cancel = service.request_scan(DEVICE, move |result| replies.borrow_mut().push(result));
            wait(&context, || !pending.borrow().is_empty());
            cancel
        };
        let last_scan = |device: &str, timestamp: i64| {
            properties(&daemon, device, &format!("{DEVICE_IFACE}.Wireless"), HashMap::from([("LastScan".into(), timestamp.to_variant())]));
            wait(&context, || service.state().devices.iter().any(|d| d.id == device && d.last_scan == Some(timestamp)));
        };
        let take = || {
            wait(&context, || !replies.borrow().is_empty());
            assert_eq!(replies.borrow().len(), 1);
            replies.borrow_mut().pop().unwrap()
        };
        // Acceptance alone and updates from another adapter do not finish a scan.
        let _cancel = start();
        pending.borrow_mut().pop().unwrap().return_value(None);
        last_scan(SECOND, 100);
        context.block_on(glib::timeout_future(Duration::from_millis(40)));
        assert!(replies.borrow().is_empty());
        last_scan(DEVICE, 100);
        take().unwrap();
        // Completion can precede acknowledgement; neither event alone is sufficient.
        let _cancel = start();
        last_scan(DEVICE, 200);
        assert!(replies.borrow().is_empty());
        pending.borrow_mut().pop().unwrap().return_value(None);
        take().unwrap();
        let _cancel = start();
        pending.borrow_mut().pop().unwrap().return_dbus_error("org.freedesktop.NetworkManager.NotAllowed", "fixture rejected");
        assert!(take().is_err());
        // Cancellation remains registered after acceptance, and completes once.
        let cancel = start();
        pending.borrow_mut().pop().unwrap().return_value(None);
        context.block_on(glib::timeout_future(Duration::from_millis(40)));
        cancel.cancel();
        assert!(take().unwrap_err().matches(gio::IOErrorEnum::Cancelled));
        last_scan(DEVICE, 300);
        assert!(replies.borrow().is_empty());
        // Preserve the two-second method timeout.
        let _cancel = start();
        assert!(take().is_err());
        pending.borrow_mut().pop().unwrap().return_value(None);
        // Accepted scans have a separate 30-second overall deadline.
        let _cancel = start();
        pending.borrow_mut().pop().unwrap().return_value(None);
        context.block_on(glib::timeout_future(Duration::from_millis(30_100)));
        assert!(take().unwrap_err().matches(gio::IOErrorEnum::TimedOut));
        last_scan(DEVICE, 400);
        assert!(replies.borrow().is_empty());
        let _cancel = start();
        pending.borrow_mut().pop().unwrap().return_value(None);
        properties(&daemon, ROOT, NAME, HashMap::from([("WirelessEnabled".into(), false.to_variant())]));
        assert!(take().unwrap_err().matches(gio::IOErrorEnum::Cancelled));
        properties(&daemon, ROOT, NAME, HashMap::from([("WirelessEnabled".into(), true.to_variant())]));
        wait(&context, || service.state().wireless_enabled);
        let _cancel = start();
        pending.borrow_mut().pop().unwrap().return_value(None);
        ownership(&daemon, false);
        assert!(take().unwrap_err().matches(gio::IOErrorEnum::Cancelled));
        ownership(&daemon, true);
        wait(&context, || service.state().available && service.state().devices.len() == 2);
        let _cancel = start();
        pending.borrow_mut().pop().unwrap().return_value(None);
        daemon.emit_signal(None, "/org/freedesktop", "org.freedesktop.DBus.ObjectManager", "InterfacesRemoved",
            Some(&(path(DEVICE), vec![DEVICE_IFACE, "org.freedesktop.NetworkManager.Device.Wireless"]).to_variant())).unwrap();
        properties(&daemon, ROOT, NAME, HashMap::from([("Devices".into(), vec![path(SECOND)].to_variant()), ("AllDevices".into(), vec![path(SECOND)].to_variant())]));
        assert!(take().unwrap_err().matches(gio::IOErrorEnum::Cancelled));
        service.stop();
        context.block_on(glib::timeout_future(Duration::from_millis(40)));
        assert!(replies.borrow().is_empty());
        for registration in registrations { daemon.unregister_object(registration).unwrap(); }
        connection.close_sync(gio::Cancellable::NONE).unwrap();
        daemon.close_sync(gio::Cancellable::NONE).unwrap();
    }).unwrap();
}
