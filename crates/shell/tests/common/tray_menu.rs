#![allow(dead_code)]
pub use crate::media_fixture::{Bus, connect, wait};
use gio::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};
pub const INTERFACE: &str = "com.canonical.dbusmenu";
const XML: &str = "<node><interface name='com.canonical.dbusmenu'>
<method name='GetLayout'><arg type='i' direction='in'/><arg type='i' direction='in'/><arg type='as' direction='in'/><arg type='u' direction='out'/><arg type='(ia{sv}av)' direction='out'/></method>
<method name='AboutToShow'><arg type='i' direction='in'/><arg type='b' direction='out'/></method>
<method name='Event'><arg type='i' direction='in'/><arg type='s' direction='in'/><arg type='v' direction='in'/><arg type='u' direction='in'/></method>
</interface></node>";
pub fn node(
    id: i32,
    label: &str,
    visible: bool,
    separator: bool,
    children: Vec<glib::Variant>,
) -> glib::Variant {
    let properties = HashMap::from([
        ("label", label.to_variant()),
        ("visible", visible.to_variant()),
        (
            "type",
            if separator { "separator" } else { "standard" }.to_variant(),
        ),
    ]);
    (id, properties, children).to_variant()
}
pub fn layout(label: &str) -> glib::Variant {
    node(
        0,
        "",
        true,
        false,
        vec![
            node(-1, "Comment", true, false, vec![]),
            node(1, label, true, false, vec![]),
            node(2, "Hidden", false, false, vec![]),
            node(3, "", true, true, vec![]),
            node(
                4,
                "Submenu",
                true,
                false,
                vec![node(5, "Child", true, false, vec![])],
            ),
            node(6, "", true, true, vec![]),
            node(7, "Quit", true, false, vec![]),
        ],
    )
}
pub fn reply(revision: u32, layout: &glib::Variant) -> glib::Variant {
    glib::Variant::tuple_from_iter([revision.to_variant(), layout.clone()])
}
#[derive(Clone, Copy)]
pub enum Reply {
    Success,
    Failure,
    Hold,
    Malformed,
}
pub struct Menu {
    pub connection: gio::DBusConnection,
    pub path: String,
    pub layout: Rc<RefCell<glib::Variant>>,
    pub revision: Rc<Cell<u32>>,
    pub reads: Rc<Cell<usize>>,
    pub layout_mode: Rc<Cell<Reply>>,
    pub action_mode: Rc<Cell<Reply>>,
    pub needs_update: Rc<Cell<bool>>,
    pub held: Rc<RefCell<Vec<gio::DBusMethodInvocation>>>,
    pub requests: Rc<RefCell<Vec<(String, glib::Variant)>>>,
    registration: Option<gio::RegistrationId>,
}
impl Menu {
    pub fn new(connection: &gio::DBusConnection, path: &str) -> Self {
        let layout = Rc::new(RefCell::new(layout("_Open")));
        let revision = Rc::new(Cell::new(1));
        let reads = Rc::new(Cell::new(0));
        let layout_mode = Rc::new(Cell::new(Reply::Success));
        let action_mode = Rc::new(Cell::new(Reply::Success));
        let needs_update = Rc::new(Cell::new(false));
        let held = Rc::new(RefCell::new(Vec::new()));
        let requests = Rc::new(RefCell::new(Vec::new()));
        let info = gio::DBusNodeInfo::for_xml(XML).unwrap();
        let (tree, version, count, layout_reply, action_reply, update, pending, calls) = (
            layout.clone(),
            revision.clone(),
            reads.clone(),
            layout_mode.clone(),
            action_mode.clone(),
            needs_update.clone(),
            held.clone(),
            requests.clone(),
        );
        let registration = connection
            .register_object(path, &info.lookup_interface(INTERFACE).unwrap())
            .method_call(move |connection, _, _, _, method, parameters, invocation| {
                calls.borrow_mut().push((method.into(), parameters.clone()));
                let mode = if method == "GetLayout" {
                    count.set(count.get() + 1);
                    layout_reply.get()
                } else {
                    action_reply.get()
                };
                match mode {
                    Reply::Success => invocation.return_value(Some(&match method {
                        "GetLayout" => reply(version.get(), &tree.borrow()),
                        "AboutToShow" => (update.get(),).to_variant(),
                        _ => ().to_variant(),
                    })),
                    Reply::Failure => invocation.return_dbus_error(
                        "com.canonical.dbusmenu.Error.Failed",
                        "Fixture rejected request",
                    ),
                    Reply::Hold => pending.borrow_mut().push(invocation),
                    Reply::Malformed => {
                        let message = invocation.message().new_method_reply();
                        message.set_body(&(42_i32,).to_variant());
                        connection
                            .send_message(&message, gio::DBusSendMessageFlags::NONE)
                            .unwrap();
                    }
                }
            })
            .build()
            .unwrap();
        Self {
            connection: connection.clone(),
            path: path.into(),
            layout,
            revision,
            reads,
            layout_mode,
            action_mode,
            needs_update,
            held,
            requests,
            registration: Some(registration),
        }
    }
    pub fn layout_updated(&self) {
        self.connection
            .emit_signal(
                None,
                &self.path,
                INTERFACE,
                "LayoutUpdated",
                Some(&(self.revision.get(), 0_i32).to_variant()),
            )
            .unwrap();
    }
    pub fn properties_updated(&self) {
        self.connection
            .emit_signal(
                None,
                &self.path,
                INTERFACE,
                "ItemsPropertiesUpdated",
                Some(
                    &(
                        Vec::<(i32, HashMap<String, glib::Variant>)>::new(),
                        Vec::<(i32, Vec<String>)>::new(),
                    )
                        .to_variant(),
                ),
            )
            .unwrap();
    }
}
impl Drop for Menu {
    fn drop(&mut self) {
        for invocation in self.held.borrow_mut().drain(..) {
            invocation.return_dbus_error("com.canonical.dbusmenu.Error.Gone", "Fixture removed");
        }
        if let Some(registration) = self.registration.take() {
            self.connection.unregister_object(registration).unwrap();
        }
    }
}
