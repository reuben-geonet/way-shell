//! The desktop notification server and its owned notification inventory.
use gio::prelude::*;
use glib::subclass::prelude::*;
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::{HashMap, VecDeque},
    sync::OnceLock,
    time::{Duration, Instant},
};
use way_shell_core::notifications::NotificationStore;
pub use way_shell_core::notifications::{
    Action, Hints, Image, Notification, NotificationRequest, Origin,
};

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";
const XML: &str =
    include_str!("../../../../data/dbus-interfaces/org.freedesktop.Notifications.xml");
const DEADLINE_MS: i32 = 2_000;
const CAPABILITIES: &[&str] = &[
    "action-icons",
    "actions",
    "body",
    "body-hyperlinks",
    "body-images",
    "body-markup",
    "icon-multi",
    "icon-static",
    "persistence",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum CloseReason {
    Expired = 1,
    Dismissed = 2,
    Requested = 3,
    Undefined = 4,
}

#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "WayShellNotificationEvent")]
pub enum NotificationEvent {
    Added {
        notification: Notification,
        index: usize,
    },
    Replaced {
        notification: Notification,
        previous: Box<Notification>,
        index: usize,
    },
    Closed {
        notification: Notification,
        index: usize,
        reason: CloseReason,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationsState {
    pub available: bool,
    pub notifications: Vec<Notification>,
}

struct Connection {
    connection: gio::DBusConnection,
    registration: Option<gio::RegistrationId>,
    _names: gio::SignalSubscription,
    closed: Option<glib::SignalHandlerId>,
    cancellable: gio::Cancellable,
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.cancellable.cancel();
        if let Some(handler) = self.closed.take() {
            self.connection.disconnect(handler);
        }
        if let Some(registration) = self.registration.take() {
            let _ = self.connection.unregister_object(registration);
        }
        if !self.connection.is_closed() {
            // Release both an acquired name and a queued ownership request.
            // This is ordered after RequestName on the same connection.
            self.connection.call(
                Some(BUS),
                BUS_PATH,
                BUS,
                "ReleaseName",
                Some(&(NAME,).to_variant()),
                Some(glib::VariantTy::new("(u)").unwrap()),
                gio::DBusCallFlags::NONE,
                DEADLINE_MS,
                gio::Cancellable::NONE,
                |_| {},
            );
        }
    }
}

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct NotificationsService {
        pub state: RefCell<NotificationsState>,
        pub store: RefCell<NotificationStore>,
        pub source: RefCell<Option<gio::DBusConnection>>,
        pub running: Cell<bool>,
        pub owned: Cell<bool>,
        pub generation: Cell<u64>,
        pub clock: OnceCell<Instant>,
        pub(super) connection: RefCell<Option<Connection>>,
        pub connecting: RefCell<Option<gio::Cancellable>>,
        pub connect_deadline: RefCell<Option<glib::JoinHandle<()>>>,
        pub retry: RefCell<Option<glib::JoinHandle<()>>>,
        pub expiration: RefCell<Option<glib::JoinHandle<()>>>,
        pub events: RefCell<VecDeque<NotificationEvent>>,
        pub publishing: Cell<bool>,
        pub changed_pending: Cell<bool>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for NotificationsService {
        const NAME: &'static str = "WayShellNotificationsService";
        type Type = super::NotificationsService;
    }
    impl ObjectImpl for NotificationsService {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("changed").build(),
                    glib::subclass::Signal::builder("event")
                        .param_types([NotificationEvent::static_type()])
                        .build(),
                ]
            })
        }
        fn dispose(&self) {
            self.obj().stop();
        }
    }
}
glib::wrapper! { pub struct NotificationsService(ObjectSubclass<imp::NotificationsService>); }
impl Default for NotificationsService {
    fn default() -> Self {
        Self::new()
    }
}
impl NotificationsService {
    pub fn new() -> Self {
        let service: Self = glib::Object::new();
        service.start();
        service
    }

    /// The supplied connection must belong to this GLib main context.
    pub fn on_connection(connection: &gio::DBusConnection) -> Self {
        let service: Self = glib::Object::new();
        service.imp().source.replace(Some(connection.clone()));
        service.start();
        service
    }

    pub fn start(&self) {
        if self.imp().running.replace(true) {
            return;
        }
        self.connect();
        self.schedule_expiration();
    }

    pub fn stop(&self) {
        self.imp().running.set(false);
        if let Some(task) = self.imp().expiration.borrow_mut().take() {
            task.abort();
        }
        self.detach();
    }

    pub fn state(&self) -> NotificationsState {
        self.imp().state.borrow().clone()
    }

    /// Internal notifications remain available when the session bus is absent.
    /// They never emit notification actions or close signals onto D-Bus.
    pub fn send_internal(&self, mut request: NotificationRequest) -> Result<u32, glib::Error> {
        request.origin = Origin::Internal;
        request.actions.clear();
        let (id, event) = self.insert(request)?;
        self.publish(Some(event));
        self.schedule_expiration();
        Ok(id)
    }

    pub fn close(&self, id: u32, reason: CloseReason) -> bool {
        let removed = {
            let mut store = self.imp().store.borrow_mut();
            let Some(index) = store
                .notifications()
                .iter()
                .position(|notification| notification.id == id)
            else {
                return false;
            };
            store.close(id).map(|notification| (index, notification))
        };
        let Some((index, notification)) = removed else {
            return false;
        };
        self.publish(Some(NotificationEvent::Closed {
            notification,
            index,
            reason,
        }));
        self.schedule_expiration();
        true
    }

    /// Emit only declared actions of a current external notification. Dismissal
    /// remains an explicit UI operation, preserving the existing controls.
    pub fn invoke_action(&self, id: u32, key: &str) -> bool {
        let exists = self.imp().store.borrow().action(id, key).is_some();
        if !exists || !self.imp().owned.get() {
            return false;
        }
        self.emit_bus("ActionInvoked", &(id, key).to_variant())
    }

    fn now(&self) -> Duration {
        self.imp().clock.get_or_init(Instant::now).elapsed()
    }

    fn insert(
        &self,
        request: NotificationRequest,
    ) -> Result<(u32, NotificationEvent), glib::Error> {
        let mut store = self.imp().store.borrow_mut();
        let result = store
            .notify(request, self.now(), glib::real_time())
            .map_err(|_| {
                glib::Error::new(
                    gio::IOErrorEnum::NoSpace,
                    "Notification identifiers are exhausted",
                )
            })?;
        let notification = store
            .get(result.id)
            .expect("Inserted notification exists")
            .clone();
        let event = match result.replaced {
            Some(previous) => NotificationEvent::Replaced {
                notification,
                previous: Box::new(previous),
                index: result.index,
            },
            None => NotificationEvent::Added {
                notification,
                index: result.index,
            },
        };
        Ok((result.id, event))
    }

    fn publish(&self, event: Option<NotificationEvent>) {
        let state = NotificationsState {
            available: self.imp().owned.get(),
            notifications: self.imp().store.borrow().notifications().to_vec(),
        };
        if *self.imp().state.borrow() != state {
            self.imp().state.replace(state);
            self.imp().changed_pending.set(true);
        }
        if let Some(event) = event {
            self.imp().events.borrow_mut().push_back(event);
        }
        if self.imp().publishing.replace(true) {
            return;
        }
        loop {
            loop {
                let event = self.imp().events.borrow_mut().pop_front();
                let Some(event) = event else {
                    break;
                };
                self.emit_by_name::<()>("event", &[&event]);
                if let NotificationEvent::Closed {
                    notification,
                    reason,
                    ..
                } = &event
                    && notification.request.origin == Origin::External
                {
                    self.emit_bus(
                        "NotificationClosed",
                        &(notification.id, *reason as u32).to_variant(),
                    );
                }
            }
            if self.imp().changed_pending.replace(false) {
                self.emit_by_name::<()>("changed", &[]);
            }
            if self.imp().events.borrow().is_empty() && !self.imp().changed_pending.get() {
                break;
            }
        }
        self.imp().publishing.set(false);
    }

    fn schedule_expiration(&self) {
        if let Some(task) = self.imp().expiration.borrow_mut().take() {
            task.abort();
        }
        if !self.imp().running.get() {
            return;
        }
        let deadline = self.imp().store.borrow().next_expiration();
        let Some(deadline) = deadline else {
            return;
        };
        let delay = deadline.saturating_sub(self.now());
        let weak = self.downgrade();
        let task = glib::MainContext::ref_thread_default().spawn_local(async move {
            glib::timeout_future(delay).await;
            if let Some(service) = weak.upgrade() {
                service.imp().expiration.borrow_mut().take();
                service.expire();
            }
        });
        self.imp().expiration.replace(Some(task));
    }

    fn expire(&self) {
        let now = self.now();
        while self.imp().running.get() {
            let next = self.imp().store.borrow().next_expired(now);
            let Some(id) = next else {
                break;
            };
            // Observers can replace or dismiss a later item. Recheck the
            // current deadline before each close, retaining live array indices.
            self.close(id, CloseReason::Expired);
        }
        self.schedule_expiration();
    }

    fn emit_bus(&self, name: &str, parameters: &glib::Variant) -> bool {
        let connection = self
            .imp()
            .connection
            .borrow()
            .as_ref()
            .map(|source| source.connection.clone());
        let Some(connection) = connection.filter(|_| self.imp().owned.get()) else {
            return false;
        };
        match connection.emit_signal(None, PATH, NAME, name, Some(parameters)) {
            Ok(()) => true,
            Err(error) => {
                glib::g_message!("way-shell", "Cannot emit notification {name}: {error}");
                false
            }
        }
    }

    fn connect(&self) {
        let source = self.imp().source.borrow().clone();
        if let Some(connection) = source {
            if !connection.is_closed() {
                self.attach(&connection);
            }
            return;
        }
        let cancellable = gio::Cancellable::new();
        self.imp().connecting.replace(Some(cancellable.clone()));
        let cancel = cancellable.clone();
        self.imp().connect_deadline.replace(Some(
            glib::MainContext::ref_thread_default().spawn_local(async move {
                glib::timeout_future(Duration::from_millis(DEADLINE_MS as u64)).await;
                cancel.cancel();
            }),
        ));
        let generation = self.imp().generation.get();
        let weak = self.downgrade();
        gio::bus_get(gio::BusType::Session, Some(&cancellable), move |result| {
            let Some(service) = weak.upgrade().filter(|service| {
                service.imp().running.get() && service.imp().generation.get() == generation
            }) else {
                return;
            };
            service.imp().connecting.borrow_mut().take();
            if let Some(task) = service.imp().connect_deadline.borrow_mut().take() {
                task.abort();
            }
            match result {
                Ok(connection) => service.attach(&connection),
                Err(error) => service.failed(&error),
            }
        });
    }

    fn attach(&self, connection: &gio::DBusConnection) {
        self.detach();
        connection.set_exit_on_close(false);
        let info =
            gio::DBusNodeInfo::for_xml(XML).expect("Tracked notification interface is valid");
        let weak = self.downgrade();
        let registration = connection
            .register_object(PATH, &info.lookup_interface(NAME).unwrap())
            .method_call(move |_, _, _, _, method, parameters, invocation| {
                let Some(service) = weak.upgrade().filter(|service| service.imp().owned.get())
                else {
                    invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.Disconnected",
                        "Notification service is unavailable",
                    );
                    return;
                };
                service.method(method, &parameters, invocation);
            })
            .build();
        let registration = match registration {
            Ok(registration) => registration,
            Err(error) => {
                self.failed(&error);
                return;
            }
        };
        let weak = self.downgrade();
        let names = connection.subscribe_to_signal(
            Some(BUS),
            Some(BUS),
            Some("NameOwnerChanged"),
            Some(BUS_PATH),
            Some(NAME),
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let Some((_, _, owner)) = signal.parameters.get::<(String, String, String)>()
                else {
                    return;
                };
                if let Some(service) = weak.upgrade() {
                    service.set_owned(
                        signal.connection.unique_name().as_deref() == Some(owner.as_str()),
                    );
                }
            },
        );
        let weak = self.downgrade();
        let closed = connection.connect_local("closed", false, move |_| {
            if let Some(service) = weak.upgrade() {
                service.detach();
                glib::g_message!(
                    "way-shell",
                    "Notification session bus disconnected; waiting for recovery"
                );
                if service.imp().source.borrow().is_none() && service.imp().running.get() {
                    service.retry();
                }
            }
            None
        });
        let cancellable = gio::Cancellable::new();
        self.imp().connection.replace(Some(Connection {
            connection: connection.clone(),
            registration: Some(registration),
            _names: names,
            closed: Some(closed),
            cancellable: cancellable.clone(),
        }));
        let generation = self.imp().generation.get();
        let weak = self.downgrade();
        // Queue behind an existing notification daemon without replacing it.
        connection.call(
            Some(BUS),
            BUS_PATH,
            BUS,
            "RequestName",
            Some(&(NAME, 0_u32).to_variant()),
            Some(glib::VariantTy::new("(u)").unwrap()),
            gio::DBusCallFlags::NONE,
            DEADLINE_MS,
            Some(&cancellable),
            move |result| {
                let Some(service) = weak
                    .upgrade()
                    .filter(|service| service.imp().generation.get() == generation)
                else {
                    return;
                };
                match result {
                    Ok(reply) if matches!(reply.get::<(u32,)>(), Some((1 | 4,))) => {
                        service.set_owned(true)
                    }
                    Ok(reply) if matches!(reply.get::<(u32,)>(), Some((2 | 3,))) => {
                        // A subsequent NameOwnerChanged can already have reported
                        // acquisition before this queued-request reply is handled.
                        if !service.imp().owned.get() {
                            glib::g_message!(
                                "way-shell",
                                "Another notification daemon owns {NAME}; waiting for it to exit"
                            );
                        }
                    }
                    Ok(_) => service.failed(&glib::Error::new(
                        gio::IOErrorEnum::InvalidData,
                        "Invalid notification name ownership reply",
                    )),
                    Err(error) => service.failed(&error),
                }
            },
        );
    }

    fn set_owned(&self, owned: bool) {
        if self.imp().owned.replace(owned) != owned {
            self.publish(None);
        }
    }

    fn failed(&self, error: &glib::Error) {
        glib::g_message!(
            "way-shell",
            "Notification service unavailable: {error}; retrying"
        );
        self.detach();
        if self.imp().running.get() {
            self.retry();
        }
    }

    fn retry(&self) {
        if self.imp().connection.borrow().is_some() || self.imp().connecting.borrow().is_some() {
            return;
        }
        let weak = self.downgrade();
        let task = glib::MainContext::ref_thread_default().spawn_local(async move {
            glib::timeout_future(Duration::from_secs(1)).await;
            if let Some(service) = weak.upgrade() {
                service.imp().retry.borrow_mut().take();
                if service.imp().running.get() {
                    service.connect();
                }
            }
        });
        self.imp().retry.replace(Some(task));
    }

    fn detach(&self) {
        self.imp()
            .generation
            .set(self.imp().generation.get().wrapping_add(1));
        if let Some(cancellable) = self.imp().connecting.borrow_mut().take() {
            cancellable.cancel();
        }
        if let Some(task) = self.imp().connect_deadline.borrow_mut().take() {
            task.abort();
        }
        if let Some(task) = self.imp().retry.borrow_mut().take() {
            task.abort();
        }
        self.imp().connection.borrow_mut().take();
        self.set_owned(false);
    }

    fn method(
        &self,
        method: &str,
        parameters: &glib::Variant,
        invocation: gio::DBusMethodInvocation,
    ) {
        match method {
            "GetCapabilities" => invocation.return_value(Some(&(CAPABILITIES,).to_variant())),
            "GetServerInformation" => invocation
                .return_value(Some(&("way-shell", "way-shell", "0.1", "1.2").to_variant())),
            "Notify" => {
                let Some(request) = parse_request(parameters) else {
                    invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.InvalidArgs",
                        "Invalid notification request",
                    );
                    return;
                };
                match self.insert(request) {
                    Ok((id, event)) => {
                        invocation.return_value(Some(&(id,).to_variant()));
                        self.publish(Some(event));
                        self.schedule_expiration();
                    }
                    Err(error) => invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.LimitsExceeded",
                        error.message(),
                    ),
                }
            }
            "CloseNotification" => {
                if let Some((id,)) = parameters.get::<(u32,)>() {
                    if self.close(id, CloseReason::Requested) {
                        invocation.return_value(Some(&().to_variant()));
                    } else {
                        invocation.return_dbus_error(
                            "org.freedesktop.DBus.Error.InvalidArgs",
                            "Notification does not exist",
                        );
                    }
                } else {
                    invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.InvalidArgs",
                        "Invalid notification identifier",
                    );
                }
            }
            _ => invocation.return_dbus_error(
                "org.freedesktop.DBus.Error.UnknownMethod",
                "Unknown notification method",
            ),
        }
    }
}

fn parse_request(parameters: &glib::Variant) -> Option<NotificationRequest> {
    let (mut app_name, replaces_id, app_icon, summary, body, actions, hints, expire_timeout) =
        parameters.get::<(
            String,
            u32,
            String,
            String,
            String,
            Vec<String>,
            HashMap<String, glib::Variant>,
            i32,
        )>()?;
    let hints = parse_hints(hints);
    if app_name.is_empty()
        && let Some(entry) = hints
            .desktop_entry
            .as_ref()
            .filter(|entry| !entry.is_empty())
    {
        app_name = gio::DesktopAppInfo::new(&format!("{entry}.desktop"))
            .map(|info| info.display_name().to_string())
            .unwrap_or_else(|| entry.clone());
    }
    Some(NotificationRequest {
        app_name,
        replaces_id,
        app_icon,
        summary,
        body,
        actions: Action::from_pairs(actions),
        hints,
        expire_timeout,
        origin: Origin::External,
    })
}

fn parse_hints(values: HashMap<String, glib::Variant>) -> Hints {
    let string = |name| values.get(name).and_then(|value| value.get::<String>());
    let boolean = |name| {
        values
            .get(name)
            .and_then(|value| value.get::<bool>())
            .unwrap_or(false)
    };
    let image = ["image-data", "image_data", "icon_data"]
        .iter()
        .find_map(|name| {
            let value = values.get(*name)?;
            let (width, height, rowstride, alpha, bits, channels, bytes) =
                value.get::<(i32, i32, i32, bool, i32, i32, Vec<u8>)>()?;
            Image::new(width, height, rowstride, alpha, bits, channels, bytes).ok()
        });
    Hints {
        category: string("category"),
        desktop_entry: string("desktop-entry"),
        image_path: string("image-path").or_else(|| string("image_path")),
        image,
        action_icons: boolean("action-icons"),
        resident: boolean("resident"),
        transient: boolean("transient"),
        urgency: values
            .get("urgency")
            .and_then(|value| value.get())
            .unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_types_aliases_and_ownership() {
        for alias in ["image-data", "image_data", "icon_data"] {
            let hints = parse_hints(HashMap::from([
                (
                    alias.into(),
                    (1_i32, 1_i32, 4_i32, true, 8_i32, 4_i32, vec![1_u8, 2, 3, 4]).to_variant(),
                ),
                ("category".into(), "email.arrived".to_variant()),
                ("desktop-entry".into(), "example".to_variant()),
                ("image_path".into(), "file:///cover.png".to_variant()),
                ("action-icons".into(), true.to_variant()),
                ("resident".into(), true.to_variant()),
                ("transient".into(), true.to_variant()),
                ("urgency".into(), 2_u8.to_variant()),
            ]));
            assert_eq!(hints.image.unwrap().data(), [1, 2, 3, 4]);
            assert_eq!(hints.category.as_deref(), Some("email.arrived"));
            assert_eq!(hints.desktop_entry.as_deref(), Some("example"));
            assert_eq!(hints.image_path.as_deref(), Some("file:///cover.png"));
            assert!(hints.action_icons && hints.resident && hints.transient);
            assert_eq!(hints.urgency, 2);
        }
        let wrong = parse_hints(HashMap::from([
            ("image-data".into(), "bad image".to_variant()),
            ("category".into(), true.to_variant()),
            ("desktop-entry".into(), 7_u32.to_variant()),
            ("image-path".into(), false.to_variant()),
            ("action-icons".into(), "true".to_variant()),
            ("resident".into(), 1_u8.to_variant()),
            ("transient".into(), 3_i32.to_variant()),
            ("urgency".into(), "critical".to_variant()),
        ]));
        assert_eq!(wrong, Hints::default());
    }

    #[test]
    fn empty_application_falls_back_to_desktop_entry_and_incomplete_actions_are_ignored() {
        let parameters = (
            "",
            0_u32,
            "",
            "Title",
            "Body",
            vec!["reply", "Reply", "dangling"],
            HashMap::from([(
                "desktop-entry",
                "way-shell-nonexistent-test-entry-c0874d39".to_variant(),
            )]),
            -1_i32,
        )
            .to_variant();
        let request = parse_request(&parameters).unwrap();
        assert_eq!(
            request.app_name,
            "way-shell-nonexistent-test-entry-c0874d39"
        );
        assert_eq!(
            request.actions,
            [Action {
                key: "reply".into(),
                label: "Reply".into()
            }]
        );
        assert!(parse_request(&"invalid request".to_variant()).is_none());
    }
}
