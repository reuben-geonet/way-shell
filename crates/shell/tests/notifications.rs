use gio::prelude::*;
use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Duration};
use way_shell::services::notifications::{
    CloseReason, NotificationEvent, NotificationRequest, NotificationsService, Origin,
};
#[path = "common/media.rs"]
mod bus_fixture;
use bus_fixture::{Bus, connect, wait};

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

#[test]
fn missing_and_restarted_session_bus_use_the_production_constructor() {
    const CHILD: &str = "WAY_SHELL_NOTIFICATIONS_BUS_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let service = NotificationsService::new();
                context.block_on(glib::timeout_future(Duration::from_millis(250)));
                assert!(!service.state().available);
                let (bus, _) = Bus::start_at(Some(&address));
                wait(&context, || service.state().available);
                let client = connect(&address);
                let before = notify(&context, &client, &standard("Before restart", 0, -1));
                drop(bus);
                wait(&context, || !service.state().available);
                assert_eq!(
                    service.state().notifications[0].id,
                    before,
                    "Bus loss preserves notification history"
                );
                let (_bus, _) = Bus::start_at(Some(&address));
                wait(&context, || service.state().available);
                let reconnected = connect(&address);
                let after = notify(&context, &reconnected, &standard("After restart", 0, -1));
                assert_ne!(after, before);
                assert_eq!(service.state().notifications.len(), 2);
                call(
                    &context,
                    &reconnected,
                    "CloseNotification",
                    Some(&(before,).to_variant()),
                )
                .unwrap();
                assert_eq!(service.state().notifications[0].id, after);
                let weak = service.downgrade();
                drop(service);
                assert!(weak.upgrade().is_none());
                // ReleaseName is asynchronous but owns no reference to the service.
                wait(&context, || {
                    context
                        .block_on(reconnected.call_future(
                            Some("org.freedesktop.DBus"),
                            "/org/freedesktop/DBus",
                            "org.freedesktop.DBus",
                            "GetNameOwner",
                            Some(&(NAME,).to_variant()),
                            None,
                            gio::DBusCallFlags::NONE,
                            2_000,
                        ))
                        .is_err()
                });
                reconnected.close_sync(gio::Cancellable::NONE).unwrap();
            })
            .unwrap();
        return;
    }
    use std::{os::unix::fs::DirBuilderExt, process::Command, time::SystemTime};
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "way-shell-notifications-{}-{nonce}",
        std::process::id()
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "missing_and_restarted_session_bus_use_the_production_constructor",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}/bus", directory.display()),
        )
        .output()
        .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn call(
    context: &glib::MainContext,
    client: &gio::DBusConnection,
    method: &str,
    parameters: Option<&glib::Variant>,
) -> Result<glib::Variant, glib::Error> {
    context.block_on(client.call_future(
        Some(NAME),
        PATH,
        NAME,
        method,
        parameters,
        None,
        gio::DBusCallFlags::NONE,
        2_000,
    ))
}
fn wire(
    summary: &str,
    app: &str,
    replaces: u32,
    actions: Vec<&str>,
    hints: HashMap<&str, glib::Variant>,
    timeout: i32,
) -> glib::Variant {
    (app, replaces, "", summary, "Body", actions, hints, timeout).to_variant()
}
fn standard(summary: &str, replaces: u32, timeout: i32) -> glib::Variant {
    wire(
        summary,
        "Example",
        replaces,
        vec!["default", "Open", "reply", "Reply"],
        HashMap::new(),
        timeout,
    )
}
fn notify(
    context: &glib::MainContext,
    client: &gio::DBusConnection,
    parameters: &glib::Variant,
) -> u32 {
    call(context, client, "Notify", Some(parameters))
        .unwrap()
        .get::<(u32,)>()
        .unwrap()
        .0
}

#[test]
fn dbus_requests_replacement_metadata_actions_and_close() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context.with_thread_default(|| {
        let server = connect(&address);
        let client = connect(&address);
        let service = NotificationsService::on_connection(&server);
        wait(&context, || service.state().available);
        let signals = Rc::new(RefCell::new(Vec::new()));
        let recorded = signals.clone();
        let _signals = client.subscribe_to_signal(Some(NAME), Some(NAME), None, Some(PATH), None,
            gio::DBusSignalFlags::NONE, move |signal| {
                recorded.borrow_mut().push((signal.signal_name.to_owned(), signal.parameters.clone()));
            });
        let events = Rc::new(RefCell::new(Vec::new()));
        let recorded = events.clone();
        service.connect_local("event", false, move |values| {
            recorded.borrow_mut().push(values[1].get::<NotificationEvent>().unwrap()); None
        });
        let caps = call(&context, &client, "GetCapabilities", None).unwrap().get::<(Vec<String>,)>().unwrap().0;
        assert!(caps.iter().any(|value| value == "actions"));
        let info = call(&context, &client, "GetServerInformation", None).unwrap()
            .get::<(String, String, String, String)>().unwrap();
        assert_eq!(info, ("way-shell".into(), "way-shell".into(), "0.1".into(), "1.2".into()));

        let first = notify(&context, &client, &wire("Blank application", "", 0, vec!["dangling"], HashMap::new(), -1));
        assert_ne!(first, 0);
        let initial = service.state().notifications[0].clone();
        assert!(initial.request.app_name.is_empty());
        assert!(initial.request.actions.is_empty());
        assert!(initial.created_on_us > 0);
        let image = (2_i32, 2_i32, 8_i32, false, 8_i32, 3_i32, (0_u8..14).collect::<Vec<_>>()).to_variant();
        let replaced = notify(&context, &client, &wire("Replacement", "New group", first,
            vec!["default", "Open", "reply", "Reply"], HashMap::from([
                ("image-data", image), ("category", "email.arrived".to_variant()),
                ("resident", true.to_variant()), ("urgency", 2_u8.to_variant()),
            ]), 0));
        assert_eq!(replaced, first);
        assert_eq!(service.state().notifications.len(), 1);
        let replacement = service.state().notifications[0].clone();
        assert_eq!(replacement.request.app_name, "New group");
        assert_eq!(replacement.request.hints.image.as_ref().unwrap().data(), &(0_u8..14).collect::<Vec<_>>());
        assert!(replacement.request.hints.resident);
        assert_eq!(replacement.request.hints.urgency, 2);
        assert!(replacement.created_on_us >= initial.created_on_us);
        assert!(matches!(&events.borrow()[1], NotificationEvent::Replaced { previous, index: 0, .. } if previous.request.summary == "Blank application"));
        assert!(signals.borrow().iter().all(|(name, _)| name != "NotificationClosed"));
        assert!(service.invoke_action(first, "reply"));
        assert!(!service.invoke_action(first, "unknown"));
        assert!(!service.invoke_action(u32::MAX, "reply"));
        wait(&context, || signals.borrow().iter().any(|(name, _)| name == "ActionInvoked"));
        assert_eq!(signals.borrow().iter().find(|(name, _)| name == "ActionInvoked").unwrap().1.get::<(u32, String)>().unwrap(), (first, "reply".into()));
        assert_eq!(service.state().notifications.len(), 1, "Action invocation does not implicitly dismiss");

        let fresh = notify(&context, &client, &standard("Unknown replacement", 500, -1));
        assert_ne!(fresh, 500);
        assert_ne!(fresh, first);
        assert_eq!(service.state().notifications[1].id, fresh);
        let malformed = HashMap::from([
            ("image-data", (20_i32, 20_i32, 60_i32, false, 8_i32, 3_i32, vec![0_u8]).to_variant()),
            ("desktop-entry", false.to_variant()), ("resident", "wrong type".to_variant()),
            ("category", 1_u32.to_variant()),
        ]);
        notify(&context, &client, &wire("Invalid hints", "", fresh, vec![], malformed, -1));
        let invalid = service.state().notifications[1].clone();
        assert!(invalid.request.hints.image.is_none());
        assert!(invalid.request.hints.category.is_none());
        assert!(invalid.request.hints.desktop_entry.is_none());
        assert!(!invalid.request.hints.resident);
        assert!(call(&context, &client, "Notify", Some(&("wrong signature",).to_variant())).is_err());
        call(&context, &client, "CloseNotification", Some(&(first,).to_variant())).unwrap();
        wait(&context, || signals.borrow().iter().any(|(name, _)| name == "NotificationClosed"));
        assert_eq!(signals.borrow().iter().find(|(name, _)| name == "NotificationClosed").unwrap().1.get::<(u32, u32)>(), Some((first, 3)));
        assert!(call(&context, &client, "CloseNotification", Some(&(first,).to_variant())).is_err());
        let internal = service.send_internal(NotificationRequest {
            summary: "Internal battery warning".into(),
            ..Default::default()
        }).unwrap();
        assert!(service.state().notifications.iter().find(|notification| notification.id == internal).unwrap().created_on_us > 0);
        assert!(!service.invoke_action(internal, "default"));
        assert!(service.close(internal, CloseReason::Dismissed));
        context.block_on(glib::timeout_future(Duration::from_millis(15)));
        assert_eq!(signals.borrow().iter().filter(|(name, _)| name == "NotificationClosed").count(), 1, "Internal notifications never emit a D-Bus close signal");
        assert_eq!(initial.request.summary, "Blank application", "Old snapshots own their text");
        assert_eq!(replacement.request.hints.image.as_ref().unwrap().data().len(), 14);
        service.stop();
        client.close_sync(gio::Cancellable::NONE).unwrap();
        server.close_sync(gio::Cancellable::NONE).unwrap();
    }).unwrap();
}

#[test]
fn explicit_expiration_history_and_reentrant_replacement() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let server = connect(&address);
            let client = connect(&address);
            let service = NotificationsService::on_connection(&server);
            wait(&context, || service.state().available);
            let closed = Rc::new(RefCell::new(Vec::new()));
            let recorded = closed.clone();
            service.connect_local("event", false, move |values| {
                if let NotificationEvent::Closed {
                    notification,
                    reason,
                    ..
                } = values[1].get::<NotificationEvent>().unwrap()
                {
                    recorded.borrow_mut().push((notification.id, reason));
                }
                None
            });
            let persistent = notify(&context, &client, &standard("Default history", 0, -1));
            let forever = notify(&context, &client, &standard("Never expire", 0, 0));
            let expires = notify(&context, &client, &standard("Explicit timeout", 0, 20));
            wait(&context, || {
                closed.borrow().contains(&(expires, CloseReason::Expired))
            });
            assert_eq!(
                service
                    .state()
                    .notifications
                    .iter()
                    .map(|notification| notification.id)
                    .collect::<Vec<_>>(),
                [persistent, forever]
            );
            let replaced = notify(&context, &client, &standard("Timed original", 0, 5_000));
            notify(&context, &client, &standard("Now persistent", replaced, 0));
            context.block_on(glib::timeout_future(Duration::from_millis(30)));
            assert!(
                service
                    .state()
                    .notifications
                    .iter()
                    .any(|notification| notification.id == replaced)
            );

            let first = service
                .send_internal(NotificationRequest {
                    summary: "First short".into(),
                    expire_timeout: 1,
                    ..Default::default()
                })
                .unwrap();
            let second = service
                .send_internal(NotificationRequest {
                    summary: "Second short".into(),
                    expire_timeout: 1,
                    ..Default::default()
                })
                .unwrap();
            let weak = service.downgrade();
            service.connect_local("event", false, move |values| {
                if let NotificationEvent::Closed { notification, .. } =
                    values[1].get::<NotificationEvent>().unwrap()
                    && notification.id == first
                    && let Some(service) = weak.upgrade()
                {
                    service
                        .send_internal(NotificationRequest {
                            summary: "Renewed by observer".into(),
                            replaces_id: second,
                            expire_timeout: 0,
                            ..Default::default()
                        })
                        .unwrap();
                }
                None
            });
            wait(&context, || {
                closed.borrow().contains(&(first, CloseReason::Expired))
            });
            assert!(!closed.borrow().iter().any(|(id, _)| *id == second));
            assert_eq!(
                service
                    .state()
                    .notifications
                    .iter()
                    .find(|notification| notification.id == second)
                    .unwrap()
                    .request
                    .summary,
                "Renewed by observer"
            );
            assert!(!service.invoke_action(second, "default"));
            service.stop();
            client.close_sync(gio::Cancellable::NONE).unwrap();
            server.close_sync(gio::Cancellable::NONE).unwrap();
        })
        .unwrap();
}

#[test]
fn reentrant_events_reach_every_observer_in_order() {
    let context = glib::MainContext::new();
    context.with_thread_default(|| {
        let service: NotificationsService = glib::Object::new();
        let weak = service.downgrade();
        service.connect_local("event", false, move |values| {
            let event = values[1].get::<NotificationEvent>().unwrap();
            let id = match event {
                NotificationEvent::Added { notification, .. } if notification.request.summary == "Close immediately" => Some(notification.id),
                NotificationEvent::Replaced { notification, .. } => Some(notification.id),
                _ => None,
            };
            if let Some(id) = id && let Some(service) = weak.upgrade() { service.close(id, CloseReason::Dismissed); }
            None
        });
        let observed = Rc::new(RefCell::new(Vec::new()));
        let recorded = observed.clone();
        service.connect_local("event", false, move |values| {
            recorded.borrow_mut().push(values[1].get::<NotificationEvent>().unwrap()); None
        });
        service.send_internal(NotificationRequest { summary: "Close immediately".into(), ..Default::default() }).unwrap();
        assert!(matches!(&observed.borrow()[0], NotificationEvent::Added { notification, .. } if notification.request.summary == "Close immediately"));
        assert!(matches!(&observed.borrow()[1], NotificationEvent::Closed { reason: CloseReason::Dismissed, .. }));
        assert!(service.state().notifications.is_empty());
        let id = service.send_internal(NotificationRequest { summary: "Original".into(), ..Default::default() }).unwrap();
        service.send_internal(NotificationRequest { summary: "Replacement".into(), replaces_id: id, ..Default::default() }).unwrap();
        assert!(matches!(&observed.borrow()[3], NotificationEvent::Replaced { previous, notification, .. } if previous.request.summary == "Original" && notification.request.summary == "Replacement"));
        assert!(matches!(&observed.borrow()[4], NotificationEvent::Closed { notification, .. } if notification.id == id && notification.request.origin == Origin::Internal));
        assert!(service.state().notifications.is_empty());
        let weak = service.downgrade();
        drop(service);
        assert!(weak.upgrade().is_none());
    }).unwrap();
}

#[test]
fn name_collision_recovery_pending_drop_and_bus_loss() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let first_connection = connect(&address);
            let first = NotificationsService::on_connection(&first_connection);
            wait(&context, || first.state().available);
            let second_connection = connect(&address);
            let second = NotificationsService::on_connection(&second_connection);
            context.block_on(glib::timeout_future(Duration::from_millis(30)));
            assert!(!second.state().available);
            let internal = second
                .send_internal(NotificationRequest {
                    summary: "Offline internal notification".into(),
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(second.state().notifications[0].id, internal);
            first.stop();
            wait(&context, || second.state().available);
            let weak = second.downgrade();
            drop(second);
            assert!(weak.upgrade().is_none());
            for _ in 0..10 {
                let service = NotificationsService::on_connection(&second_connection);
                let weak = service.downgrade();
                drop(service);
                assert!(weak.upgrade().is_none());
            }
            let service = NotificationsService::on_connection(&second_connection);
            wait(&context, || service.state().available);
            second_connection
                .close_sync(gio::Cancellable::NONE)
                .unwrap();
            wait(&context, || !service.state().available);
            first_connection.close_sync(gio::Cancellable::NONE).unwrap();
        })
        .unwrap();
}
