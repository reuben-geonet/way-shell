#![allow(dead_code)]
use gio::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    rc::Rc,
    time::{Duration, Instant},
};

pub const PATH: &str = "/org/mpris/MediaPlayer2";
pub const ROOT: &str = "org.mpris.MediaPlayer2";
pub const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const XML: &str = "<node>
<interface name='org.mpris.MediaPlayer2'>
 <method name='Raise'/>
 <property name='Identity' type='s' access='read'/>
 <property name='CanRaise' type='b' access='read'/>
</interface>
<interface name='org.mpris.MediaPlayer2.Player'>
 <method name='Play'/><method name='Pause'/><method name='PlayPause'/>
 <method name='Stop'/><method name='Next'/><method name='Previous'/>
 <property name='PlaybackStatus' type='s' access='read'/>
 <property name='Metadata' type='a{sv}' access='read'/>
 <property name='CanControl' type='b' access='read'/>
 <property name='CanPlay' type='b' access='read'/>
 <property name='CanPause' type='b' access='read'/>
 <property name='CanGoNext' type='b' access='read'/>
 <property name='CanGoPrevious' type='b' access='read'/>
</interface></node>";

pub struct Bus(Child);
impl Bus {
    pub fn start() -> (Self, String) {
        Self::start_at(None)
    }

    pub fn start_at(address: Option<&str>) -> (Self, String) {
        let config = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/session-bus.conf"
        );
        let mut command = Command::new("dbus-daemon");
        command.args([
            "--nofork",
            "--print-address=1",
            &format!("--config-file={config}"),
        ]);
        if let Some(address) = address {
            command.arg(format!("--address={address}"));
        }
        let mut child = command.stdout(Stdio::piped()).spawn().unwrap();
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        assert!(!address.trim().is_empty());
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

pub fn wait(context: &glib::MainContext, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "Media service did not reach expected state"
        );
        context.block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}

pub fn metadata(title: &str) -> glib::Variant {
    HashMap::from([
        ("xesam:title", title.to_variant()),
        ("xesam:album", "Album".to_variant()),
        ("xesam:artist", vec!["One", "Two"].to_variant()),
        ("mpris:artUrl", "file:///cover.png".to_variant()),
    ])
    .to_variant()
}

#[derive(Clone, Copy)]
pub enum Reply {
    Success,
    Failure,
    Malformed,
    Hold,
}
pub struct Player {
    pub connection: gio::DBusConnection,
    pub name: String,
    pub mode: Rc<Cell<Reply>>,
    pub calls: Rc<RefCell<Vec<(String, String)>>>,
    pub held: Rc<RefCell<Vec<gio::DBusMethodInvocation>>>,
    properties: Rc<RefCell<HashMap<(String, String), glib::Variant>>>,
    registrations: Vec<gio::RegistrationId>,
}
impl Player {
    pub fn new(address: &str, name: &str, title: &str) -> Self {
        let connection = connect(address);
        Self::on_connection(connection, name, title)
    }

    pub fn on_connection(connection: gio::DBusConnection, name: &str, title: &str) -> Self {
        let properties = Rc::new(RefCell::new(HashMap::from([
            (
                (ROOT.into(), "Identity".into()),
                "Fixture player".to_variant(),
            ),
            ((ROOT.into(), "CanRaise".into()), true.to_variant()),
            (
                (PLAYER.into(), "PlaybackStatus".into()),
                "Playing".to_variant(),
            ),
            ((PLAYER.into(), "Metadata".into()), metadata(title)),
            ((PLAYER.into(), "CanControl".into()), true.to_variant()),
            ((PLAYER.into(), "CanPlay".into()), true.to_variant()),
            ((PLAYER.into(), "CanPause".into()), true.to_variant()),
            ((PLAYER.into(), "CanGoNext".into()), true.to_variant()),
            ((PLAYER.into(), "CanGoPrevious".into()), true.to_variant()),
        ])));
        let mode = Rc::new(Cell::new(Reply::Success));
        let calls = Rc::new(RefCell::new(Vec::new()));
        let held = Rc::new(RefCell::new(Vec::new()));
        let info = gio::DBusNodeInfo::for_xml(XML).unwrap();
        let mut registrations = Vec::new();
        for interface in [ROOT, PLAYER] {
            let values = properties.clone();
            let request_mode = mode.clone();
            let requests = calls.clone();
            let pending = held.clone();
            registrations.push(
                connection
                    .register_object(PATH, &info.lookup_interface(interface).unwrap())
                    .property(move |_, _, _, interface, name| {
                        values
                            .borrow()
                            .get(&(interface.to_owned(), name.to_owned()))
                            .unwrap()
                            .clone()
                    })
                    .method_call(move |connection, _, _, interface, method, _, invocation| {
                        requests
                            .borrow_mut()
                            .push((interface.unwrap().to_owned(), method.to_owned()));
                        match request_mode.get() {
                            Reply::Success => invocation.return_value(Some(&().to_variant())),
                            Reply::Failure => invocation.return_dbus_error(
                                "org.mpris.MediaPlayer2.Error.Failed",
                                "Fixture rejected command",
                            ),
                            Reply::Malformed => {
                                // Send the wire reply directly so the fixture can
                                // violate its declared method signature deliberately.
                                let reply = invocation.message().new_method_reply();
                                reply.set_body(&(42_u32,).to_variant());
                                connection
                                    .send_message(&reply, gio::DBusSendMessageFlags::NONE)
                                    .unwrap();
                            }
                            Reply::Hold => pending.borrow_mut().push(invocation),
                        }
                    })
                    .build()
                    .unwrap(),
            );
        }
        Self {
            connection,
            name: name.into(),
            mode,
            calls,
            held,
            properties,
            registrations,
        }
    }

    pub fn acquire(&self, replace: bool) {
        let flags = if replace { 2_u32 | 4 } else { 1_u32 | 4 };
        let reply = self
            .connection
            .call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "RequestName",
                Some(&(&self.name, flags).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                2_000,
                gio::Cancellable::NONE,
            )
            .unwrap();
        assert!(matches!(reply.get::<(u32,)>(), Some((1 | 4,))));
    }

    pub fn release(&self) {
        self.connection
            .call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "ReleaseName",
                Some(&(&self.name,).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                2_000,
                gio::Cancellable::NONE,
            )
            .unwrap();
    }

    pub fn owner(&self) -> String {
        self.connection.unique_name().unwrap().to_string()
    }

    pub fn property(&self, interface: &str, name: &str, value: glib::Variant) {
        self.properties
            .borrow_mut()
            .insert((interface.to_owned(), name.to_owned()), value.clone());
        self.connection
            .emit_signal(
                None,
                PATH,
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

    pub fn invalidated(&self, interface: &str, name: &str, replacement: glib::Variant) {
        self.properties
            .borrow_mut()
            .insert((interface.to_owned(), name.to_owned()), replacement);
        self.connection
            .emit_signal(
                None,
                PATH,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                Some(
                    &(
                        interface,
                        HashMap::<String, glib::Variant>::new(),
                        vec![name],
                    )
                        .to_variant(),
                ),
            )
            .unwrap();
    }

    pub fn finish_held(&self) {
        let held = self.held.borrow_mut().drain(..).collect::<Vec<_>>();
        for invocation in held {
            invocation.return_value(Some(&().to_variant()));
        }
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.finish_held();
        for registration in self.registrations.drain(..) {
            self.connection.unregister_object(registration).unwrap();
        }
        let _ = self.connection.close_sync(gio::Cancellable::NONE);
    }
}
