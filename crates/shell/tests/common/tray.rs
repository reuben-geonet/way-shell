#![allow(dead_code)]
pub use crate::media_fixture::{Bus, connect, wait};
use gio::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};
pub const WATCHER: &str = "org.kde.StatusNotifierWatcher";
pub const WATCHER_PATH: &str = "/StatusNotifierWatcher";
pub const ITEM: &str = "org.kde.StatusNotifierItem";
pub const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const XML: &str = "<node><interface name='org.kde.StatusNotifierItem'>
<method name='Activate'><arg type='i' direction='in'/><arg type='i' direction='in'/></method>
<method name='SecondaryActivate'><arg type='i' direction='in'/><arg type='i' direction='in'/></method>
<method name='ContextMenu'><arg type='i' direction='in'/><arg type='i' direction='in'/></method>
<method name='Scroll'><arg type='i' direction='in'/><arg type='s' direction='in'/></method>
</interface><interface name='org.freedesktop.DBus.Properties'>
<method name='GetAll'><arg type='s' direction='in'/><arg type='a{sv}' direction='out'/></method>
</interface></node>";
#[derive(Clone, Copy)]
pub enum Reply {
    Success,
    Failure,
    Hold,
}
pub struct Item {
    pub connection: gio::DBusConnection,
    pub path: String,
    pub values: Rc<RefCell<HashMap<String, glib::Variant>>>,
    pub reads: Rc<Cell<usize>>,
    pub read_mode: Rc<Cell<Reply>>,
    pub action_mode: Rc<Cell<Reply>>,
    pub held: Rc<RefCell<Vec<gio::DBusMethodInvocation>>>,
    pub actions: Rc<RefCell<Vec<(String, glib::Variant)>>>,
    registrations: Vec<gio::RegistrationId>,
}
impl Item {
    pub fn new(connection: &gio::DBusConnection, path: &str, id: &str) -> Self {
        let values = Rc::new(RefCell::new(HashMap::from([
            ("Id".into(), id.to_variant()),
            ("Title".into(), id.to_variant()),
            ("Status".into(), "Active".to_variant()),
            ("Category".into(), "ApplicationStatus".to_variant()),
        ])));
        let reads = Rc::new(Cell::new(0));
        let read_mode = Rc::new(Cell::new(Reply::Success));
        let action_mode = Rc::new(Cell::new(Reply::Success));
        let held = Rc::new(RefCell::new(Vec::new()));
        let actions = Rc::new(RefCell::new(Vec::new()));
        let info = gio::DBusNodeInfo::for_xml(XML).unwrap();
        let mut registrations = Vec::new();
        for interface in [ITEM, PROPERTIES] {
            let (values, reads, read_mode, action_mode, held, actions) = (
                values.clone(),
                reads.clone(),
                read_mode.clone(),
                action_mode.clone(),
                held.clone(),
                actions.clone(),
            );
            registrations.push(
                connection
                    .register_object(path, &info.lookup_interface(interface).unwrap())
                    .method_call(move |_, _, _, _, method, parameters, invocation| {
                        let read = method == "GetAll";
                        let mode = if read {
                            reads.set(reads.get() + 1);
                            read_mode.get()
                        } else {
                            actions.borrow_mut().push((method.into(), parameters));
                            action_mode.get()
                        };
                        match mode {
                            Reply::Success => invocation.return_value(Some(&if read {
                                (values.borrow().clone(),).to_variant()
                            } else {
                                ().to_variant()
                            })),
                            Reply::Failure => invocation.return_dbus_error(
                                "org.kde.StatusNotifierItem.Error.Failed",
                                "Fixture rejected request",
                            ),
                            Reply::Hold => held.borrow_mut().push(invocation),
                        }
                    })
                    .build()
                    .unwrap(),
            );
        }
        Self {
            connection: connection.clone(),
            path: path.into(),
            values,
            reads,
            read_mode,
            action_mode,
            held,
            actions,
            registrations,
        }
    }
    pub fn signal(&self, name: &str, parameters: Option<&glib::Variant>) {
        self.connection
            .emit_signal(None, &self.path, ITEM, name, parameters)
            .unwrap();
    }
    pub fn properties_changed(&self) {
        self.connection
            .emit_signal(
                None,
                &self.path,
                PROPERTIES,
                "PropertiesChanged",
                Some(
                    &(
                        ITEM,
                        HashMap::<String, glib::Variant>::new(),
                        vec!["IconName"],
                    )
                        .to_variant(),
                ),
            )
            .unwrap();
    }
}
impl Drop for Item {
    fn drop(&mut self) {
        for invocation in self.held.borrow_mut().drain(..) {
            invocation
                .return_dbus_error("org.kde.StatusNotifierItem.Error.Gone", "Fixture removed");
        }
        for registration in self.registrations.drain(..) {
            self.connection.unregister_object(registration).unwrap();
        }
    }
}
pub fn acquire(connection: &gio::DBusConnection, name: &str, replace: bool) {
    let flags = if replace { 2_u32 | 4 } else { 1_u32 | 4 };
    let result = connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&(name, flags).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            2_000,
            gio::Cancellable::NONE,
        )
        .unwrap();
    assert!(matches!(result.get::<(u32,)>(), Some((1 | 4,))));
}
pub fn register(
    context: &glib::MainContext,
    connection: &gio::DBusConnection,
    name: &str,
) -> Result<glib::Variant, glib::Error> {
    context.block_on(connection.call_future(
        Some(WATCHER),
        WATCHER_PATH,
        WATCHER,
        "RegisterStatusNotifierItem",
        Some(&(name,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        2_000,
    ))
}
pub fn property(
    context: &glib::MainContext,
    connection: &gio::DBusConnection,
    name: &str,
) -> glib::Variant {
    context
        .block_on(connection.call_future(
            Some(WATCHER),
            WATCHER_PATH,
            PROPERTIES,
            "Get",
            Some(&(WATCHER, name).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            2_000,
        ))
        .unwrap()
        .child_value(0)
        .as_variant()
        .unwrap()
}
