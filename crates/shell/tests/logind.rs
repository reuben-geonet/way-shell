use gio::prelude::*;
use glib::variant::{Handle, ObjectPath};
use std::{
    cell::{Cell, RefCell},
    io::{BufRead, BufReader, Read},
    os::unix::net::UnixStream,
    process::{Child, Command, Stdio},
    rc::Rc,
    time::{Duration, Instant},
};
use way_shell::services::logind::{LogindService, PowerAction, SessionIdentity};
const NAME: &str = "org.freedesktop.login1";
const PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION: &str = "org.freedesktop.login1.Session";
const OWN: &str = "/org/freedesktop/login1/session/own";
const OTHER: &str = "/org/freedesktop/login1/session/other";
struct Bus(Child);
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn bus() -> (Bus, String) {
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
    (Bus(child), address.trim().to_owned())
}
fn connection(address: &str) -> gio::DBusConnection {
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
fn wait(context: &glib::MainContext, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while !predicate() {
        assert!(Instant::now() < deadline, "login1 fixture timed out");
        context.block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}
fn own_name(connection: &gio::DBusConnection, own: bool) {
    let args = if own {
        (NAME, 0_u32).to_variant()
    } else {
        (NAME,).to_variant()
    };
    connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            if own { "RequestName" } else { "ReleaseName" },
            Some(&args),
            None,
            gio::DBusCallFlags::NONE,
            2_000,
            gio::Cancellable::NONE,
        )
        .unwrap();
}
fn memory_settings() -> gio::Settings {
    let source =
        gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)
            .unwrap();
    gio::Settings::new_full(
        &source
            .lookup("org.ldelossa.way-shell.system", false)
            .unwrap(),
        Some(&gio::memory_settings_backend_new()),
        None,
    )
}
fn send_fd(invocation: gio::DBusMethodInvocation, peers: &RefCell<Vec<UnixStream>>, invalid: bool) {
    let (peer, descriptor) = UnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    peers.borrow_mut().push(peer);
    let list = gio::UnixFDList::new();
    let index = list.append(&descriptor).unwrap();
    invocation.return_value_with_unix_fd_list(
        Some(&(Handle(if invalid { 77 } else { index }),).to_variant()),
        Some(&list),
    );
}
fn closed(peers: &RefCell<Vec<UnixStream>>, index: usize) -> bool {
    match peers.borrow_mut()[index].read(&mut [0; 1]) {
        Ok(0) => true,
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
        reply => panic!("Unexpected inhibitor data: {reply:?}"),
    }
}

#[test]
fn session_actions_inhibitors_cancellation_restart_and_cleanup() {
    let (_bus, address) = bus();
    let context = glib::MainContext::new();
    context.with_thread_default(|| {
        let daemon = connection(&address); let client = connection(&address);
        let peers = Rc::new(RefCell::new(Vec::new()));
        let deny = Rc::new(Cell::new(false)); let invalid_fd = Rc::new(Cell::new(false));
        let delay = Rc::new(Cell::new(false)); let alive = Rc::new(Cell::new(true)); let suspend = Rc::new(Cell::new(false));
        let pending = Rc::new(RefCell::new(Vec::<gio::DBusMethodInvocation>::new()));
        let calls = Rc::new(RefCell::new(Vec::<(String, glib::Variant)>::new()));
        let mut xml = format!("<node><interface name='{MANAGER}'><method name='GetSessionByPID'><arg type='u' direction='in'/><arg type='o' direction='out'/></method><method name='GetSession'><arg type='s' direction='in'/><arg type='o' direction='out'/></method><method name='ListSessions'><arg type='a(susso)' direction='out'/></method><method name='Inhibit'><arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='h' direction='out'/></method>");
        for method in ["Reboot", "PowerOff", "Suspend", "Hibernate", "HybridSleep", "SuspendThenHibernate"] {
            xml.push_str(&format!("<method name='Can{method}'><arg type='s' direction='out'/></method><method name='{method}'><arg type='b' direction='in'/></method>"));
        }
        xml.push_str("</interface></node>");
        let info = gio::DBusNodeInfo::for_xml(&xml).unwrap();
        let callback_peers = peers.clone(); let callback_deny = deny.clone(); let callback_invalid = invalid_fd.clone(); let callback_delay = delay.clone(); let callback_pending = pending.clone(); let callback_calls = calls.clone(); let callback_alive = alive.clone(); let callback_suspend = suspend.clone();
        let manager = daemon.register_object(PATH, &info.lookup_interface(MANAGER).unwrap()).method_call(move |_, _, _, _, method, args, invocation| {
            match method {
                // Deliberately return another user's session first; the service must validate User.
                "GetSessionByPID" => invocation.return_value(Some(&(ObjectPath::try_from(OTHER).unwrap(),).to_variant())),
                "GetSession" => invocation.return_dbus_error("org.freedesktop.login1.NoSuchSession", "unknown session"),
                "ListSessions" => {
                    let mut rows = vec![("other", 1001_u32, "other", "seat1", ObjectPath::try_from(OTHER).unwrap())];
                    if callback_alive.get() { rows.push(("own", 1000_u32, "own", "seat0", ObjectPath::try_from(OWN).unwrap())); }
                    invocation.return_value(Some(&(rows,).to_variant()));
                }
                "Inhibit" => {
                    assert_eq!(args.get::<(String,String,String,String)>().unwrap(), ("idle".into(), "way-shell".into(), "User initiated idle block".into(), "block".into()));
                    if callback_deny.get() { invocation.return_dbus_error("org.freedesktop.DBus.Error.AccessDenied", "fixture denied"); }
                    else if callback_delay.get() { callback_pending.borrow_mut().push(invocation); }
                    else { send_fd(invocation, &callback_peers, callback_invalid.get()); }
                }
                method if method.starts_with("Can") => {
                    let result = match method { "CanHibernate" => "challenge", "CanSuspend" if !callback_suspend.get() => "no", _ => "yes" };
                    invocation.return_value(Some(&(result,).to_variant()));
                }
                _ => {
                    assert_eq!(args.get::<(bool,)>(), Some((false,)));
                    callback_calls.borrow_mut().push((method.into(), args));
                    if callback_deny.get() { invocation.return_dbus_error("org.freedesktop.DBus.Error.AccessDenied", "fixture denied"); }
                    else { invocation.return_value(None); }
                }
            }
        }).build().unwrap();
        let xml = format!("<node><interface name='{SESSION}'><property name='Id' type='s' access='read'/><property name='State' type='s' access='read'/><property name='User' type='(uo)' access='read'/><method name='Terminate'/><method name='SetBrightness'><arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='u' direction='in'/></method></interface></node>");
        let info = gio::DBusNodeInfo::for_xml(&xml).unwrap();
        let mut registrations = vec![manager];
        for (path, id, uid) in [(OTHER, "other", 1001_u32), (OWN, "own", 1000)] {
            let recorded = calls.clone(); let delayed = delay.clone(); let pending = pending.clone(); let denied = deny.clone();
            registrations.push(daemon.register_object(path, &info.lookup_interface(SESSION).unwrap())
                .property(move |_, _, _, _, property| match property { "Id" => id.to_variant(), "State" => "active".to_variant(), "User" => (uid, ObjectPath::try_from("/org/freedesktop/login1/user/fixture").unwrap()).to_variant(), _ => unreachable!() })
                .method_call(move |_, _, path, _, method, args, invocation| {
                    assert_eq!(path, OWN, "Actions must never target another user's session");
                    recorded.borrow_mut().push((method.into(), args));
                    if denied.get() { invocation.return_dbus_error("org.freedesktop.DBus.Error.AccessDenied", "fixture denied"); }
                    else if delayed.get() { pending.borrow_mut().push(invocation); }
                    else { invocation.return_value(None); }
                }).build().unwrap());
        }
        let settings = memory_settings(); settings.set_boolean("idle-inhibitor", true).unwrap();
        let identity = SessionIdentity { uid: 1000, pid: 9999, session_id: None };
        let service = LogindService::on_connection(&client, settings.clone(), identity.clone());
        assert!(!service.state().available);
        own_name(&daemon, true);
        wait(&context, || service.state().session.as_deref() == Some("own") && service.state().inhibited && service.can(PowerAction::PowerOff));
        assert!(!service.can(PowerAction::Hibernate)); assert!(!service.can(PowerAction::Suspend));
        assert_eq!(peers.borrow().len(), 1);
        service.set_idle_inhibit(true); assert_eq!(peers.borrow().len(), 1);
        settings.set_boolean("idle-inhibitor", false).unwrap();
        wait(&context, || !service.state().inhibited && closed(&peers, 0));
        let result = Rc::new(RefCell::new(None));
        let recorded = result.clone(); service.perform(PowerAction::PowerOff, move |reply| { recorded.replace(Some(reply)); });
        wait(&context, || result.borrow().is_some()); result.borrow_mut().take().unwrap().unwrap();
        let recorded = result.clone(); service.terminate_session(move |reply| { recorded.replace(Some(reply)); });
        wait(&context, || result.borrow().is_some()); result.borrow_mut().take().unwrap().unwrap();
        let recorded = result.clone(); service.set_brightness("backlight", "intel_backlight", 500, move |reply| { recorded.replace(Some(reply)); });
        wait(&context, || result.borrow().is_some()); result.borrow_mut().take().unwrap().unwrap();
        assert!(calls.borrow().iter().any(|(method, args)| method == "SetBrightness" && args.get::<(String,String,u32)>() == Some(("backlight".into(), "intel_backlight".into(), 500))));
        deny.set(true);
        let recorded = result.clone(); service.perform(PowerAction::PowerOff, move |reply| { recorded.replace(Some(reply)); });
        wait(&context, || result.borrow().is_some()); assert!(result.borrow_mut().take().unwrap().is_err());
        service.set_idle_inhibit(true); context.block_on(glib::timeout_future(Duration::from_millis(30))); assert!(!service.state().inhibited);
        deny.set(false); invalid_fd.set(true); service.set_idle_inhibit(true);
        wait(&context, || peers.borrow().len() == 2 && closed(&peers, 1)); assert!(!service.state().inhibited);
        invalid_fd.set(false); delay.set(true); service.set_idle_inhibit(true);
        wait(&context, || !pending.borrow().is_empty()); service.set_idle_inhibit(false);
        send_fd(pending.borrow_mut().pop().unwrap(), &peers, false);
        wait(&context, || closed(&peers, 2)); assert!(!service.state().inhibited);
        let recorded = result.clone(); let cancel = service.set_brightness("leds", "keyboard", 1, move |reply| { recorded.replace(Some(reply)); });
        wait(&context, || !pending.borrow().is_empty()); cancel.cancel();
        wait(&context, || result.borrow().is_some()); assert!(result.borrow_mut().take().unwrap().unwrap_err().matches(gio::IOErrorEnum::Cancelled));
        pending.borrow_mut().pop().unwrap().return_value(None);
        delay.set(false); service.set_idle_inhibit(true);
        wait(&context, || service.state().inhibited); assert_eq!(peers.borrow().len(), 4);
        own_name(&daemon, false); wait(&context, || !service.state().available && closed(&peers, 3)); assert!(service.state().session.is_none());
        own_name(&daemon, true); wait(&context, || service.state().inhibited && service.state().session.is_some());
        suspend.set(true);
        daemon.emit_signal(None, PATH, MANAGER, "PrepareForSleep", Some(&(false,).to_variant())).unwrap();
        wait(&context, || service.can(PowerAction::Suspend));
        alive.set(false); daemon.emit_signal(None, PATH, MANAGER, "SessionRemoved", Some(&("own", ObjectPath::try_from(OWN).unwrap()).to_variant())).unwrap();
        wait(&context, || service.state().session.is_none());
        let recorded = result.clone(); service.set_brightness("backlight", "intel", 10, move |reply| { recorded.replace(Some(reply)); });
        assert!(result.borrow_mut().take().unwrap().unwrap_err().matches(gio::IOErrorEnum::NotConnected));
        alive.set(true); daemon.emit_signal(None, PATH, MANAGER, "SessionNew", Some(&("own", ObjectPath::try_from(OWN).unwrap()).to_variant())).unwrap();
        wait(&context, || service.state().session.is_some());
        let weak = service.downgrade(); drop(service); assert!(weak.upgrade().is_none());
        wait(&context, || closed(&peers, 4));
        for _ in 0..20 { let service = LogindService::on_connection(&client, settings.clone(), identity.clone()); let weak = service.downgrade(); drop(service); assert!(weak.upgrade().is_none()); }
        settings.set_boolean("idle-inhibitor", true).unwrap(); context.block_on(glib::timeout_future(Duration::from_millis(20)));
        for registration in registrations { daemon.unregister_object(registration).unwrap(); }
        client.close_sync(gio::Cancellable::NONE).unwrap(); daemon.close_sync(gio::Cancellable::NONE).unwrap();
    }).unwrap();
}
