use gio::prelude::*;
use std::{cell::RefCell, rc::Rc, time::Duration};
use way_shell::services::tray::{ItemCommand, ItemKey, Orientation, TrayEvent, TrayService};
#[path = "common/media.rs"]
mod media_fixture;
#[path = "common/tray.rs"]
mod fixture;
use fixture::*;

#[test]
fn registration_identity_properties_icons_and_removal() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let server = connect(&address);
            let client = connect(&address);
            let service = TrayService::on_connection(&server);
            wait(&context, || service.state().available);
            assert_eq!(
                property(&context, &client, "ProtocolVersion").get::<i32>(),
                Some(0)
            );
            assert_eq!(
                property(&context, &client, "IsStatusNotifierHostRegistered").get::<bool>(),
                Some(true)
            );
            let events = Rc::new(RefCell::new(Vec::new()));
            let recorded = events.clone();
            service.connect_local("event", false, move |values| {
                recorded
                    .borrow_mut()
                    .push(values[1].get::<TrayEvent>().unwrap());
                None
            });
            let first = Item::new(&client, "/First", "first");
            let second = Item::new(&client, "/Second", "second");
            first.values.borrow_mut().extend([
                (
                    "IconPixmap".into(),
                    vec![
                        (1_i32, 1_i32, vec![128_u8, 10, 20, 30]),
                        (-1, 4, vec![]),
                        (2, 2, vec![7; 15]),
                    ]
                    .to_variant(),
                ),
                (
                    "OverlayIconPixmap".into(),
                    vec![(1_i32, 1_i32, vec![255_u8, 90, 80, 70])].to_variant(),
                ),
                ("WindowId".into(), u32::MAX.to_variant()),
                (
                    "Menu".into(),
                    glib::variant::ObjectPath::try_from("/Menu")
                        .unwrap()
                        .to_variant(),
                ),
            ]);
            register(&context, &client, "/First").unwrap();
            register(&context, &client, "/First").unwrap();
            register(&context, &client, "/Second").unwrap();
            wait(&context, || service.state().items.len() == 2);
            let state = service.state();
            let key = ItemKey {
                owner: client.unique_name().unwrap().into(),
                path: "/First".into(),
            };
            assert_eq!(state.items[0].key, key);
            assert_eq!(
                state.items[0].icon.pixmap.as_ref().unwrap().pixels,
                [10, 20, 30, 128]
            );
            assert_eq!(
                state.items[0].overlay.pixmap.as_ref().unwrap().pixels,
                [90, 80, 70, 255]
            );
            assert_eq!(state.items[0].window_id, u32::MAX);
            assert_eq!(state.items[0].menu_path.as_deref(), Some("/Menu"));
            assert_eq!(
                events
                    .borrow()
                    .iter()
                    .filter(|event| matches!(event, TrayEvent::Added(_)))
                    .count(),
                2
            );
            assert_eq!(
                property(&context, &client, "RegisteredStatusNotifierItems")
                    .get::<Vec<String>>()
                    .unwrap(),
                vec![
                    format!("{}/First", key.owner),
                    format!("{}/Second", key.owner)
                ]
            );
            assert!(register(&context, &client, "bad name").is_err());
            assert!(register(&context, &client, "/bad//path").is_err());
            assert!(register(&context, &client, "org.example.Missing").is_err());
            first
                .values
                .borrow_mut()
                .insert("Title".into(), "Updated".to_variant());
            first.values.borrow_mut().insert(
                "IconPixmap".into(),
                vec![(i32::MAX, i32::MAX, vec![0_u8; 4])].to_variant(),
            );
            first.values.borrow_mut().insert(
                "Menu".into(),
                glib::variant::ObjectPath::try_from("/")
                    .unwrap()
                    .to_variant(),
            );
            first.signal("NewTitle", None);
            wait(&context, || service.state().items[0].title == "Updated");
            assert!(service.state().items[0].icon.pixmap.is_none());
            assert!(service.state().items[0].menu_path.is_none());
            first
                .values
                .borrow_mut()
                .insert("Status".into(), "NeedsAttention".to_variant());
            first.properties_changed();
            wait(&context, || {
                service.state().items[0].status == "NeedsAttention"
            });
            drop(first);
            drop(second);
            client.close_sync(gio::Cancellable::NONE).unwrap();
            wait(&context, || service.state().items.is_empty());
            assert_eq!(
                events
                    .borrow()
                    .iter()
                    .filter(|event| matches!(event, TrayEvent::Removed(_)))
                    .count(),
                2
            );
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
        })
        .unwrap();
}

#[test]
fn aliases_follow_owner_replacement_and_commands_target_the_current_item() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let service = TrayService::on_connection(&connect(&address));
            wait(&context, || service.state().available);
            let first_connection = connect(&address);
            let first = Item::new(&first_connection, "/StatusNotifierItem", "old");
            acquire(&first_connection, "org.example.Tray", false);
            register(&context, &first_connection, "org.example.Tray").unwrap();
            register(
                &context,
                &first_connection,
                &first_connection.unique_name().unwrap(),
            )
            .unwrap();
            wait(&context, || service.state().items.len() == 1);
            let old_key = service.state().items[0].key.clone();
            let second_connection = connect(&address);
            let second = Item::new(&second_connection, "/StatusNotifierItem", "new");
            acquire(&second_connection, "org.example.Tray", true);
            wait(&context, || service.state().items.len() == 2);
            // The explicit unique-name alias keeps the first owner's item alive.
            first_connection.close_sync(gio::Cancellable::NONE).unwrap();
            wait(&context, || {
                service.state().items.len() == 1 && service.state().items[0].id == "new"
            });
            let new_key = service.state().items[0].key.clone();
            assert_ne!(old_key, new_key);
            for command in [
                ItemCommand::Activate { x: 3, y: 4 },
                ItemCommand::SecondaryActivate { x: 5, y: 6 },
                ItemCommand::ContextMenu { x: 7, y: 8 },
                ItemCommand::Scroll {
                    delta: -120,
                    orientation: Orientation::Vertical,
                },
            ] {
                let result = Rc::new(RefCell::new(None));
                let output = result.clone();
                service.command(&new_key, command, move |value| {
                    output.replace(Some(value));
                });
                wait(&context, || result.borrow().is_some());
                assert!(result.borrow_mut().take().unwrap().is_ok());
            }
            assert_eq!(
                second.actions.borrow()[0].1.get::<(i32, i32)>(),
                Some((3, 4))
            );
            assert_eq!(
                second.actions.borrow()[3].1.get::<(i32, String)>(),
                Some((-120, "vertical".into()))
            );
            second.action_mode.set(Reply::Failure);
            let result = Rc::new(RefCell::new(None));
            let output = result.clone();
            service.command(
                &new_key,
                ItemCommand::Activate { x: 0, y: 0 },
                move |value| {
                    output.replace(Some(value));
                },
            );
            wait(&context, || result.borrow().is_some());
            assert!(result.borrow_mut().take().unwrap().is_err());
            service.stop();
            assert!(service.state().items.is_empty());
            drop(first);
            drop(second);
        })
        .unwrap();
}

#[test]
fn delayed_properties_stale_replies_cancellation_and_watcher_conflict() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let server = connect(&address);
            let service = TrayService::on_connection(&server);
            wait(&context, || service.state().available);
            let second_service = TrayService::on_connection(&connect(&address));
            context.block_on(glib::timeout_future(Duration::from_millis(50)));
            assert!(!second_service.state().available);
            let client = connect(&address);
            let delayed = Item::new(&client, "/Delayed", "delayed");
            delayed.values.borrow_mut().clear();
            register(&context, &client, "/Delayed").unwrap();
            wait(&context, || delayed.reads.get() > 0);
            assert!(service.state().items.is_empty());
            delayed.values.borrow_mut().extend([
                ("Id".into(), "ready".to_variant()),
                ("Status".into(), "Active".to_variant()),
            ]);
            wait(&context, || service.state().items.len() == 1);
            delayed.read_mode.set(Reply::Hold);
            delayed.signal("NewIcon", None);
            wait(&context, || !delayed.held.borrow().is_empty());
            delayed.read_mode.set(Reply::Success);
            delayed
                .values
                .borrow_mut()
                .insert("Title".into(), "Newest".to_variant());
            delayed.signal("NewTitle", None);
            wait(&context, || service.state().items[0].title == "Newest");
            let stale = delayed.held.borrow_mut().remove(0);
            stale.return_value(Some(
                &(std::collections::HashMap::from([
                    ("Id", "stale".to_variant()),
                    ("Status", "Active".to_variant()),
                ]),)
                    .to_variant(),
            ));
            context.block_on(glib::timeout_future(Duration::from_millis(20)));
            assert_eq!(service.state().items[0].title, "Newest");
            delayed.action_mode.set(Reply::Hold);
            let result = Rc::new(RefCell::new(None));
            let output = result.clone();
            service.command(
                &service.state().items[0].key,
                ItemCommand::Activate { x: 0, y: 0 },
                move |value| {
                    output.replace(Some(value));
                },
            );
            wait(&context, || !delayed.held.borrow().is_empty());
            service.stop();
            wait(&context, || result.borrow().is_some());
            assert!(result.borrow_mut().take().unwrap().is_err());
            wait(&context, || second_service.state().available);
            register(&context, &client, "/Delayed").unwrap();
            wait(&context, || second_service.state().items.len() == 1);
            drop(delayed);
            drop(client);
            drop(second_service);
        })
        .unwrap();
}

#[test]
fn reentrant_stop_orders_events_and_disconnects_pending_item_reads() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let service = TrayService::on_connection(&connect(&address));
            wait(&context, || service.state().available);
            service.connect_local("event", false, move |values| {
                if matches!(values[1].get::<TrayEvent>().unwrap(), TrayEvent::Added(_)) {
                    values[0].get::<TrayService>().unwrap().stop();
                }
                None
            });
            let events = Rc::new(RefCell::new(Vec::new()));
            let recorded = events.clone();
            service.connect_local("event", false, move |values| {
                recorded
                    .borrow_mut()
                    .push(values[1].get::<TrayEvent>().unwrap());
                None
            });
            let client = connect(&address);
            let item = Item::new(&client, "/Item", "reentrant");
            register(&context, &client, "/Item").unwrap();
            wait(&context, || events.borrow().len() == 2);
            assert!(matches!(
                &events.borrow()[..],
                [TrayEvent::Added(_), TrayEvent::Removed(_)]
            ));
            assert!(!service.state().available);
            assert!(service.state().items.is_empty());
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
            item.signal("NewIcon", None);
            context.block_on(glib::timeout_future(Duration::from_millis(20)));
            assert_eq!(events.borrow().len(), 2);

            let service = TrayService::on_connection(&connect(&address));
            wait(&context, || service.state().available);
            item.read_mode.set(Reply::Hold);
            register(&context, &client, "/Item").unwrap();
            wait(&context, || !item.held.borrow().is_empty());
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
            item.held
                .borrow_mut()
                .remove(0)
                .return_value(Some(&(item.values.borrow().clone(),).to_variant()));
            context.block_on(glib::timeout_future(Duration::from_millis(20)));
        })
        .unwrap();
}

#[test]
fn missing_and_restarted_bus_recover_through_the_production_constructor() {
    const CHILD: &str = "WAY_SHELL_TRAY_BUS_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let service = TrayService::new();
                context.block_on(glib::timeout_future(Duration::from_millis(100)));
                assert!(!service.state().available);
                let (bus, _) = Bus::start_at(Some(&address));
                wait(&context, || service.state().available);
                let connection = connect(&address);
                let item = Item::new(&connection, "/Item", "before");
                register(&context, &connection, "/Item").unwrap();
                wait(&context, || service.state().items.len() == 1);
                drop(bus);
                wait(&context, || {
                    !service.state().available && service.state().items.is_empty()
                });
                drop(item);
                let (_bus, _) = Bus::start_at(Some(&address));
                wait(&context, || service.state().available);
                let connection = connect(&address);
                let _item = Item::new(&connection, "/Item", "after");
                register(&context, &connection, "/Item").unwrap();
                wait(&context, || service.state().items.len() == 1);
                assert_eq!(service.state().items[0].id, "after");
                let weak = service.downgrade();
                drop(service);
                assert!(weak.upgrade().is_none());
            })
            .unwrap();
        return;
    }
    use std::{os::unix::fs::DirBuilderExt, process::Command, time::SystemTime};
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("way-shell-tray-{}-{nonce}", std::process::id()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "missing_and_restarted_bus_recover_through_the_production_constructor",
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

#[test]
fn simultaneous_duplicate_registration_replies_are_idempotent() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let service = TrayService::on_connection(&connect(&address));
            wait(&context, || service.state().available);
            let client = connect(&address);
            let _item = Item::new(&client, "/Item", "concurrent");
            let replies = Rc::new(RefCell::new(Vec::new()));
            for _ in 0..3 {
                let replies = replies.clone();
                client.call(
                    Some(WATCHER),
                    WATCHER_PATH,
                    WATCHER,
                    "RegisterStatusNotifierItem",
                    Some(&("/Item",).to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    2_000,
                    gio::Cancellable::NONE,
                    move |reply| replies.borrow_mut().push(reply),
                );
            }
            wait(&context, || replies.borrow().len() == 3);
            assert!(
                replies.borrow().iter().all(Result::is_ok),
                "{:?}",
                replies.borrow()
            );
            wait(&context, || service.state().items.len() == 1);
            assert_eq!(
                property(&context, &client, "RegisteredStatusNotifierItems")
                    .get::<Vec<String>>()
                    .unwrap()
                    .len(),
                1
            );
        })
        .unwrap();
}

#[test]
fn host_registration_and_public_watcher_signals_match_inventory() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let server = connect(&address);
            let service = TrayService::on_connection(&server);
            wait(&context, || service.state().available);
            let client = connect(&address);
            let signals = Rc::new(RefCell::new(Vec::new()));
            let recorded = signals.clone();
            let _subscription = client.subscribe_to_signal(
                Some(WATCHER),
                Some(WATCHER),
                None,
                Some(WATCHER_PATH),
                None,
                gio::DBusSignalFlags::NONE,
                move |signal| {
                    recorded
                        .borrow_mut()
                        .push((signal.signal_name.to_owned(), signal.parameters.clone()))
                },
            );
            acquire(&client, "org.example.Host", false);
            for _ in 0..2 {
                context
                    .block_on(client.call_future(
                        Some(WATCHER),
                        WATCHER_PATH,
                        WATCHER,
                        "RegisterStatusNotifierHost",
                        Some(&("org.example.Host",).to_variant()),
                        None,
                        gio::DBusCallFlags::NONE,
                        2_000,
                    ))
                    .unwrap();
            }
            wait(&context, || !signals.borrow().is_empty());
            assert_eq!(
                signals
                    .borrow()
                    .iter()
                    .filter(|(signal, _)| signal == "StatusNotifierHostRegistered")
                    .count(),
                1
            );
            let host_name = format!("org.kde.StatusNotifierHost-{}", std::process::id());
            let host_path = format!("/StatusNotifierHost/{}", std::process::id());
            let introspection = context
                .block_on(client.call_future(
                    Some(&host_name),
                    &host_path,
                    "org.freedesktop.DBus.Introspectable",
                    "Introspect",
                    None,
                    None,
                    gio::DBusCallFlags::NONE,
                    2_000,
                ))
                .unwrap();
            assert!(
                introspection
                    .get::<(String,)>()
                    .unwrap()
                    .0
                    .contains("org.kde.StatusNotifierHost")
            );
            assert!(
                context
                    .block_on(client.call_future(
                        Some(WATCHER),
                        WATCHER_PATH,
                        WATCHER,
                        "RegisterStatusNotifierHost",
                        Some(&("org.example.MissingHost",).to_variant()),
                        None,
                        gio::DBusCallFlags::NONE,
                        2_000
                    ))
                    .is_err()
            );
            let item_connection = connect(&address);
            let item = Item::new(&item_connection, "/One", "wire");
            register(&context, &item_connection, "/One").unwrap();
            let canonical = format!("{}/One", item_connection.unique_name().unwrap());
            wait(&context, || {
                signals
                    .borrow()
                    .iter()
                    .any(|(signal, _)| signal == "StatusNotifierItemRegistered")
            });
            assert_eq!(
                signals
                    .borrow()
                    .iter()
                    .find(|(signal, _)| signal == "StatusNotifierItemRegistered")
                    .unwrap()
                    .1
                    .get::<(String,)>(),
                Some((canonical.clone(),))
            );
            drop(item);
            item_connection.close_sync(gio::Cancellable::NONE).unwrap();
            wait(&context, || {
                signals
                    .borrow()
                    .iter()
                    .any(|(signal, _)| signal == "StatusNotifierItemUnregistered")
            });
            assert_eq!(
                signals
                    .borrow()
                    .iter()
                    .find(|(signal, _)| signal == "StatusNotifierItemUnregistered")
                    .unwrap()
                    .1
                    .get::<(String,)>(),
                Some((canonical,))
            );
            assert!(
                property(&context, &client, "RegisteredStatusNotifierItems")
                    .get::<Vec<String>>()
                    .unwrap()
                    .is_empty()
            );
        })
        .unwrap();
}
