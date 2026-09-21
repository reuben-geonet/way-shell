//! A cancellable D-Bus menu endpoint with an owned, bounded layout tree.
use crate::platform::bus_watch::Watcher;
use gio::prelude::*;
use glib::subclass::prelude::*;
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    sync::OnceLock,
    time::Duration,
};
const INTERFACE: &str = "com.canonical.dbusmenu";
const DEADLINE_MS: i32 = 2_000;
const MAX_DEPTH: usize = 64;
const MAX_NODES: usize = 4096;
const MAX_LABEL_BYTES: usize = 1_048_576;
const PROPERTIES: &[&str] = &["label", "visible", "type"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuNode {
    pub id: i32,
    pub label: String,
    pub visible: bool,
    pub separator: bool,
    pub children: Vec<MenuNode>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, glib::Boxed)]
#[boxed_type(name = "WayShellTrayMenuState")]
pub struct MenuState {
    pub available: bool,
    pub revision: u32,
    /// Last valid tree, retained through a failed refresh. Stop and endpoint
    /// owner loss clear it; `available` controls whether actions can be sent.
    pub root: Option<MenuNode>,
}
struct Endpoint {
    connection: gio::DBusConnection,
    owner: String,
    path: String,
}
mod imp {
    use super::*;
    #[derive(Default)]
    pub struct MenuService {
        pub(super) endpoint: OnceCell<Endpoint>,
        pub state: RefCell<MenuState>,
        pub running: Cell<bool>,
        pub owned: Cell<bool>,
        pub generation: Cell<u64>,
        pub revision: Cell<u64>,
        pub(super) watch: RefCell<Option<Watcher>>,
        pub subscription: RefCell<Option<gio::SignalSubscription>>,
        pub closed: RefCell<Option<glib::SignalHandlerId>>,
        pub lifetime: RefCell<Option<gio::Cancellable>>,
        pub refresh: RefCell<Option<gio::Cancellable>>,
        pub retry: RefCell<Option<glib::JoinHandle<()>>>,
        pub snapshots: RefCell<VecDeque<MenuState>>,
        pub publishing: Cell<bool>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for MenuService {
        const NAME: &'static str = "WayShellTrayMenuService";
        type Type = super::MenuService;
    }
    impl ObjectImpl for MenuService {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("changed").build(),
                    glib::subclass::Signal::builder("snapshot")
                        .param_types([MenuState::static_type()])
                        .build(),
                ]
            })
        }
        fn dispose(&self) {
            self.obj().stop();
        }
    }
}
glib::wrapper! { pub struct MenuService(ObjectSubclass<imp::MenuService>); }
impl MenuService {
    /// Bind to one unique owner. The tray inventory creates a new service when
    /// the owner, connection, or menu path changes, keeping old actions isolated.
    pub fn new(
        connection: &gio::DBusConnection,
        owner: &str,
        path: &str,
    ) -> Result<Self, glib::Error> {
        if !gio::dbus_is_unique_name(owner) || path == "/" || !glib::Variant::is_object_path(path) {
            return Err(glib::Error::new(
                gio::IOErrorEnum::InvalidArgument,
                "A menu needs a unique D-Bus owner and a non-root object path",
            ));
        }
        let service: Self = glib::Object::new();
        service
            .imp()
            .endpoint
            .set(Endpoint {
                connection: connection.clone(),
                owner: owner.into(),
                path: path.into(),
            })
            .ok()
            .expect("New menu endpoint is unset");
        service.start();
        Ok(service)
    }
    pub fn state(&self) -> MenuState {
        self.imp().state.borrow().clone()
    }
    pub fn start(&self) {
        if self.imp().running.replace(true) {
            return;
        }
        let Some(endpoint) = self.imp().endpoint.get() else {
            return;
        };
        if endpoint.connection.is_closed() {
            return;
        }
        let generation = self.imp().generation.get();
        let weak = self.downgrade();
        let subscription = endpoint.connection.subscribe_to_signal(
            Some(&endpoint.owner),
            Some(INTERFACE),
            None,
            Some(&endpoint.path),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let Some(service) = weak.upgrade().filter(|service| service.current(generation))
                else {
                    return;
                };
                let valid = match signal.signal_name {
                    "LayoutUpdated" => signal.parameters.get::<(u32, i32)>().is_some(),
                    "ItemsPropertiesUpdated" => signal
                        .parameters
                        .is_type(glib::VariantTy::new("(a(ia{sv})a(ias))").unwrap()),
                    _ => false,
                };
                if valid {
                    service.refresh();
                }
            },
        );
        self.imp().subscription.replace(Some(subscription));
        let weak = self.downgrade();
        let closed = endpoint
            .connection
            .connect_local("closed", false, move |_| {
                if let Some(service) = weak.upgrade().filter(|service| service.current(generation))
                {
                    service.owner_lost();
                }
                None
            });
        self.imp().closed.replace(Some(closed));
        let appeared = self.downgrade();
        let vanished = self.downgrade();
        let watch = Watcher::on_connection(
            &endpoint.connection,
            &endpoint.owner,
            move |_, _| {
                if let Some(service) = appeared
                    .upgrade()
                    .filter(|service| service.current(generation))
                {
                    service.imp().owned.set(true);
                    service
                        .imp()
                        .lifetime
                        .replace(Some(gio::Cancellable::new()));
                    service.refresh();
                }
            },
            move || {
                if let Some(service) = vanished
                    .upgrade()
                    .filter(|service| service.current(generation))
                {
                    service.owner_lost();
                }
            },
        );
        self.imp().watch.replace(Some(watch));
    }
    pub fn stop(&self) {
        self.imp().running.set(false);
        self.imp()
            .generation
            .set(self.imp().generation.get().wrapping_add(1));
        self.imp().watch.borrow_mut().take();
        self.imp().subscription.borrow_mut().take();
        if let Some(handler) = self.imp().closed.borrow_mut().take()
            && let Some(endpoint) = self.imp().endpoint.get()
        {
            endpoint.connection.disconnect(handler);
        }
        self.owner_lost();
    }
    /// Refresh the whole tree. Revision numbers can wrap or be reused for
    /// property-only changes, so request generations decide which reply wins.
    pub fn refresh(&self) {
        if !self.imp().running.get() || !self.imp().owned.get() {
            return;
        }
        self.cancel_refresh();
        let endpoint = self.imp().endpoint.get().unwrap();
        let cancel = gio::Cancellable::new();
        self.imp().refresh.replace(Some(cancel.clone()));
        let generation = self.imp().generation.get();
        let revision = self.imp().revision.get();
        let weak = self.downgrade();
        endpoint.connection.call(
            Some(&endpoint.owner),
            &endpoint.path,
            INTERFACE,
            "GetLayout",
            Some(&(0_i32, -1_i32, PROPERTIES).to_variant()),
            Some(glib::VariantTy::new("(u(ia{sv}av))").unwrap()),
            gio::DBusCallFlags::NO_AUTO_START,
            DEADLINE_MS,
            Some(&cancel),
            move |result| {
                let Some(service) = weak.upgrade().filter(|service| {
                    service.current(generation)
                        && service.imp().owned.get()
                        && service.imp().revision.get() == revision
                }) else {
                    return;
                };
                service.imp().refresh.borrow_mut().take();
                let parsed = result.and_then(|reply| {
                    let revision = reply.child_get::<u32>(0);
                    parse_layout(&reply.child_value(1)).map(|root| MenuState {
                        available: true,
                        revision,
                        root: Some(root),
                    })
                });
                match parsed {
                    Ok(state) => service.publish(state),
                    Err(error) => service.read_failed(&error),
                }
            },
        );
    }
    /// Report the application's actual reply; a true reply schedules a layout
    /// refresh. Subscribe to snapshots to observe that updated layout.
    pub fn about_to_show(
        &self,
        id: i32,
        callback: impl FnOnce(Result<bool, glib::Error>) + 'static,
    ) {
        if id < 0 {
            callback(Err(invalid("Menu identifiers must be nonnegative")));
            return;
        }
        let Some(cancel) = self.action_cancellable() else {
            callback(Err(unavailable()));
            return;
        };
        let endpoint = self.imp().endpoint.get().unwrap();
        let weak = self.downgrade();
        let check = cancel.clone();
        endpoint.connection.call(
            Some(&endpoint.owner),
            &endpoint.path,
            INTERFACE,
            "AboutToShow",
            Some(&(id,).to_variant()),
            Some(glib::VariantTy::new("(b)").unwrap()),
            gio::DBusCallFlags::NO_AUTO_START,
            DEADLINE_MS,
            Some(&cancel),
            move |result| {
                let result = if check.is_cancelled() {
                    Err(cancelled())
                } else {
                    result.map(|reply| reply.child_get::<bool>(0))
                };
                if matches!(result, Ok(true))
                    && let Some(service) = weak.upgrade()
                {
                    service.refresh();
                }
                callback(result);
            },
        );
    }
    pub fn activate(
        &self,
        id: i32,
        timestamp: u32,
        callback: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) {
        let actionable = {
            let state = self.imp().state.borrow();
            state.available
                && state
                    .root
                    .as_ref()
                    .is_some_and(|root| root.children.iter().any(|child| has_action(child, id)))
        };
        if id <= 0 || !actionable {
            callback(Err(invalid("Menu item is not a current visible action")));
            return;
        }
        let Some(cancel) = self.action_cancellable() else {
            callback(Err(unavailable()));
            return;
        };
        let endpoint = self.imp().endpoint.get().unwrap();
        let check = cancel.clone();
        // Variant's ToVariant implementation wraps the integer once, matching
        // Event's (isvu) signature. The supplied timestamp remains unsigned.
        let parameters = (id, "clicked", 0_i32.to_variant(), timestamp).to_variant();
        endpoint.connection.call(
            Some(&endpoint.owner),
            &endpoint.path,
            INTERFACE,
            "Event",
            Some(&parameters),
            Some(glib::VariantTy::new("()").unwrap()),
            gio::DBusCallFlags::NO_AUTO_START,
            DEADLINE_MS,
            Some(&cancel),
            move |result| {
                callback(if check.is_cancelled() {
                    Err(cancelled())
                } else {
                    result.map(|_| ())
                });
            },
        );
    }
    fn action_cancellable(&self) -> Option<gio::Cancellable> {
        if !self.imp().running.get() || !self.imp().owned.get() {
            return None;
        }
        self.imp().lifetime.borrow().clone()
    }
    fn current(&self, generation: u64) -> bool {
        self.imp().running.get() && self.imp().generation.get() == generation
    }
    fn cancel_refresh(&self) {
        self.imp()
            .revision
            .set(self.imp().revision.get().wrapping_add(1));
        if let Some(cancel) = self.imp().refresh.borrow_mut().take() {
            cancel.cancel();
        }
        if let Some(task) = self.imp().retry.borrow_mut().take() {
            task.abort();
        }
    }
    fn owner_lost(&self) {
        self.imp().owned.set(false);
        self.cancel_refresh();
        if let Some(cancel) = self.imp().lifetime.borrow_mut().take() {
            cancel.cancel();
        }
        self.publish(MenuState::default());
    }
    fn read_failed(&self, error: &glib::Error) {
        let endpoint = self.imp().endpoint.get().unwrap();
        glib::g_message!(
            "way-shell",
            "Cannot read tray menu {}{}: {error}; retrying",
            endpoint.owner,
            endpoint.path
        );
        let generation = self.imp().generation.get();
        let revision = self.imp().revision.get();
        let mut state = self.state();
        state.available = false;
        self.publish(state);
        // A snapshot observer may stop, restart, or explicitly refresh while
        // handling this failure. Its newer request supersedes this retry.
        if !self.current(generation)
            || !self.imp().owned.get()
            || self.imp().revision.get() != revision
        {
            return;
        }
        let weak = self.downgrade();
        self.imp()
            .retry
            .replace(Some(glib::MainContext::ref_thread_default().spawn_local(
                async move {
                    glib::timeout_future(Duration::from_secs(1)).await;
                    if let Some(service) = weak.upgrade() {
                        service.imp().retry.borrow_mut().take();
                        service.refresh();
                    }
                },
            )));
    }
    fn publish(&self, state: MenuState) {
        if *self.imp().state.borrow() == state {
            return;
        }
        self.imp().state.replace(state.clone());
        self.imp().snapshots.borrow_mut().push_back(state);
        if self.imp().publishing.replace(true) {
            return;
        }
        loop {
            let next = self.imp().snapshots.borrow_mut().pop_front();
            let Some(next) = next else {
                break;
            };
            self.emit_by_name::<()>("snapshot", &[&next]);
            self.emit_by_name::<()>("changed", &[]);
        }
        self.imp().publishing.set(false);
    }
}
fn unavailable() -> glib::Error {
    glib::Error::new(
        gio::IOErrorEnum::NotConnected,
        "Menu endpoint is unavailable",
    )
}
fn cancelled() -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::Cancelled, "Menu endpoint stopped")
}
fn invalid(message: &str) -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::InvalidArgument, message)
}
fn malformed(message: &str) -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::InvalidData, message)
}
fn has_action(node: &MenuNode, id: i32) -> bool {
    node.visible
        && !node.separator
        && (node.id == id || node.children.iter().any(|child| has_action(child, id)))
}
struct Budget {
    nodes: usize,
    labels: usize,
    ids: HashSet<i32>,
}
fn parse_layout(layout: &glib::Variant) -> Result<MenuNode, glib::Error> {
    let mut budget = Budget {
        nodes: 0,
        labels: 0,
        ids: HashSet::new(),
    };
    let root = parse_node(layout, 0, &mut budget)?
        .ok_or_else(|| malformed("Menu root must be nonnegative"))?;
    if root.id != 0 {
        return Err(malformed("GetLayout root must have identifier zero"));
    }
    Ok(root)
}
fn parse_node(
    value: &glib::Variant,
    depth: usize,
    budget: &mut Budget,
) -> Result<Option<MenuNode>, glib::Error> {
    if depth > MAX_DEPTH {
        return Err(malformed("Menu layout exceeds the 64-level depth limit"));
    }
    budget.nodes += 1;
    if budget.nodes > MAX_NODES {
        return Err(malformed("Menu layout exceeds the 4096-node limit"));
    }
    if !value.is_type(glib::VariantTy::new("(ia{sv}av)").unwrap()) {
        return Err(malformed("Menu node must have signature (ia{sv}av)"));
    }
    let id = value.child_get::<i32>(0);
    if id < 0 {
        return Ok(None);
    }
    if !budget.ids.insert(id) {
        return Err(malformed("Menu layout contains duplicate identifiers"));
    }
    let properties = value.child_get::<HashMap<String, glib::Variant>>(1);
    let label = match properties.get("label") {
        Some(value) => value
            .get::<String>()
            .ok_or_else(|| malformed("Menu label must be a string"))?,
        None => String::new(),
    };
    budget.labels = budget
        .labels
        .checked_add(label.len())
        .ok_or_else(|| malformed("Menu label sizes overflow"))?;
    if budget.labels > MAX_LABEL_BYTES {
        return Err(malformed("Menu labels exceed the 1 MiB total byte limit"));
    }
    let visible = match properties.get("visible") {
        Some(value) => value
            .get::<bool>()
            .ok_or_else(|| malformed("Menu visibility must be a boolean"))?,
        None => true,
    };
    let separator = match properties.get("type") {
        Some(value) => {
            value
                .get::<String>()
                .ok_or_else(|| malformed("Menu type must be a string"))?
                == "separator"
        }
        None => false,
    };
    let mut children = Vec::new();
    for child in value.child_value(2).iter() {
        let child = child
            .as_variant()
            .ok_or_else(|| malformed("Menu children must be variants"))?;
        if let Some(node) = parse_node(&child, depth + 1, budget)? {
            children.push(node);
        }
    }
    Ok(Some(MenuNode {
        id,
        label,
        visible,
        separator,
        children,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(
        id: i32,
        properties: HashMap<&str, glib::Variant>,
        children: Vec<glib::Variant>,
    ) -> glib::Variant {
        (id, properties, children).to_variant()
    }
    fn leaf(id: i32) -> glib::Variant {
        node(id, HashMap::new(), vec![])
    }
    fn nested(depth: usize) -> glib::Variant {
        let mut root = leaf(depth as i32);
        for id in (0..depth).rev() {
            root = node(id as i32, HashMap::new(), vec![root]);
        }
        root
    }
    #[test]
    fn defaults_and_ignored_negative_nodes_preserve_existing_layout_semantics() {
        let layout = node(
            0,
            HashMap::new(),
            vec![
                node(
                    -1,
                    HashMap::from([("label", 42_i32.to_variant())]),
                    vec!["ignored invalid child".to_variant()],
                ),
                node(
                    1,
                    HashMap::from([
                        ("label", "_Open".to_variant()),
                        ("enabled", false.to_variant()),
                    ]),
                    vec![],
                ),
                node(
                    2,
                    HashMap::from([("type", "separator".to_variant())]),
                    vec![],
                ),
                node(
                    3,
                    HashMap::from([("label", "Submenu".to_variant())]),
                    vec![node(
                        4,
                        HashMap::from([("visible", false.to_variant())]),
                        vec![],
                    )],
                ),
            ],
        );
        let root = parse_layout(&layout).unwrap();
        assert_eq!(root.children.len(), 3);
        assert_eq!(root.children[0].label, "_Open");
        assert!(root.children[0].visible);
        assert!(root.children[1].separator);
        assert_eq!(
            root.children[2].children.len(),
            1,
            "Invisible-only submenus retain their children for the adapter"
        );
        assert!(!root.children[2].children[0].visible);
        assert!(has_action(&root.children[0], 1));
        assert!(!has_action(&root.children[1], 2));
        assert!(!has_action(&root.children[2], 4));
    }
    #[test]
    fn malformed_shapes_known_properties_and_duplicate_ids_are_rejected() {
        assert!(parse_layout(&42_i32.to_variant()).is_err());
        assert!(parse_layout(&leaf(-1)).is_err());
        assert!(parse_layout(&leaf(1)).is_err());
        assert!(parse_layout(&node(0, HashMap::new(), vec![leaf(0)])).is_err());
        assert!(parse_layout(&node(0, HashMap::new(), vec![leaf(1), leaf(1)])).is_err());
        for (name, value) in [
            ("label", true.to_variant()),
            ("visible", "false".to_variant()),
            ("type", 42_i32.to_variant()),
        ] {
            assert!(parse_layout(&node(0, HashMap::from([(name, value)]), vec![])).is_err());
        }
        assert!(parse_layout(&node(0, HashMap::new(), vec![42_i32.to_variant()])).is_err());
    }
    #[test]
    fn depth_node_count_and_total_label_limits_accept_exact_boundaries() {
        assert!(parse_layout(&nested(MAX_DEPTH)).is_ok());
        assert!(
            parse_layout(&nested(MAX_DEPTH + 1))
                .unwrap_err()
                .message()
                .contains("64-level")
        );
        let children = (1..MAX_NODES as i32).map(leaf).collect();
        assert!(parse_layout(&node(0, HashMap::new(), children)).is_ok());
        let children = (1..=MAX_NODES as i32).map(leaf).collect();
        assert!(
            parse_layout(&node(0, HashMap::new(), children))
                .unwrap_err()
                .message()
                .contains("4096-node")
        );
        let label = "x".repeat(MAX_LABEL_BYTES);
        assert!(
            parse_layout(&node(
                0,
                HashMap::from([("label", label.to_variant())]),
                vec![]
            ))
            .is_ok()
        );
        assert!(
            parse_layout(&node(
                0,
                HashMap::from([("label", label.to_variant())]),
                vec![node(
                    1,
                    HashMap::from([("label", "x".to_variant())]),
                    vec![]
                )]
            ))
            .unwrap_err()
            .message()
            .contains("1 MiB")
        );
    }
}
