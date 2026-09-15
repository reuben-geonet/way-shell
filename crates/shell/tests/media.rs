use gio::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};
use way_shell::services::media::{MediaAction, MediaService, Metadata, PlaybackStatus};
#[path = "common/media.rs"]
mod fixture;
use fixture::{Bus, PATH, PLAYER, Player, ROOT, Reply, connect, metadata, wait};

#[test]
fn session_bus_startup_and_reconnection_use_the_production_constructor() {
    const CHILD: &str = "WAY_SHELL_MEDIA_BUS_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let service = MediaService::new();
                context.block_on(glib::timeout_future(Duration::from_millis(250)));
                assert!(!service.state().available);

                let (bus, _) = Bus::start_at(Some(&address));
                let first = Player::new(
                    &address,
                    "org.mpris.MediaPlayer2.reconnect",
                    "Before restart",
                );
                first.acquire(false);
                wait(&context, || service.state().players.len() == 1);
                assert_eq!(
                    service.state().players[0].metadata.title.as_deref(),
                    Some("Before restart")
                );
                drop(bus);
                wait(&context, || {
                    !service.state().available && service.state().players.is_empty()
                });
                drop(first);

                let (_bus, _) = Bus::start_at(Some(&address));
                let second = Player::new(
                    &address,
                    "org.mpris.MediaPlayer2.reconnect",
                    "After restart",
                );
                second.acquire(false);
                wait(&context, || service.state().players.len() == 1);
                assert_eq!(
                    service.state().players[0].metadata.title.as_deref(),
                    Some("After restart")
                );
                let weak = service.downgrade();
                drop(service);
                assert!(weak.upgrade().is_none());
                second.property(PLAYER, "Metadata", metadata("After disposal"));
                context.block_on(glib::timeout_future(Duration::from_millis(30)));
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
        std::env::temp_dir().join(format!("way-shell-media-{}-{nonce}", std::process::id()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "session_bus_startup_and_reconnection_use_the_production_constructor",
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
fn delayed_property_export_retries_without_reacquiring_the_player_name() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            for unavailable_interface in [ROOT, PLAYER] {
                let owner = connect(&address);
                let attempts = Rc::new(Cell::new(0));
                let recorded = attempts.clone();
                let info = gio::DBusNodeInfo::for_xml(
                    "<node><interface name='org.freedesktop.DBus.Properties'>\
                     <method name='GetAll'><arg type='s' direction='in'/>\
                     <arg type='a{sv}' direction='out'/></method></interface></node>",
                )
                .unwrap();
                let registration = owner
                    .register_object(PATH, &info.interfaces()[0])
                    .method_call(move |_, _, _, _, _, args, invocation| {
                        let (interface,) = args.get::<(String,)>().unwrap();
                        if interface == unavailable_interface {
                            recorded.set(recorded.get() + 1);
                            invocation.return_dbus_error(
                                "org.freedesktop.DBus.Error.UnknownInterface",
                                "Player properties are not exported yet",
                            );
                        } else {
                            let properties = match interface.as_str() {
                                ROOT => HashMap::from([("Identity", "Late player".to_variant())]),
                                PLAYER => HashMap::from([
                                    ("PlaybackStatus", "Playing".to_variant()),
                                    ("Metadata", metadata("Late track")),
                                ]),
                                _ => panic!("Unexpected media interface: {interface}"),
                            };
                            invocation.return_value(Some(&(properties,).to_variant()));
                        }
                    })
                    .build()
                    .unwrap();
                let name = "org.mpris.MediaPlayer2.delayed";
                owner
                    .call_sync(
                        Some("org.freedesktop.DBus"),
                        "/org/freedesktop/DBus",
                        "org.freedesktop.DBus",
                        "RequestName",
                        Some(&(name, 4_u32).to_variant()),
                        None,
                        gio::DBusCallFlags::NONE,
                        2_000,
                        gio::Cancellable::NONE,
                    )
                    .unwrap();
                let client = connect(&address);
                let service = MediaService::on_connection(&client);
                wait(&context, || attempts.get() > 0);
                // Flush the failed GetAll reply and the proxy completion callback.
                context.block_on(glib::timeout_future(Duration::from_millis(100)));
                assert!(service.state().available);
                assert!(
                    service.state().players.is_empty(),
                    "An incomplete {unavailable_interface} interface must not publish a blank player"
                );

                owner.unregister_object(registration).unwrap();
                // Keep the same connection and name owner. Exporting an object
                // alone sends neither NameOwnerChanged nor PropertiesChanged.
                let player = Player::on_connection(owner, name, "Late track");
                if unavailable_interface == PLAYER {
                    // An empty, correctly typed Metadata dictionary is valid
                    // for a player with no selected track.
                    player.property(
                        PLAYER,
                        "Metadata",
                        HashMap::<String, glib::Variant>::new().to_variant(),
                    );
                }
                wait(&context, || service.state().players.len() == 1);
                let snapshot = service.state().players.remove(0);
                assert_eq!(snapshot.owner, player.owner());
                assert_eq!(snapshot.identity.as_deref(), Some("Fixture player"));
                assert_eq!(snapshot.playback_status, PlaybackStatus::Playing);
                if unavailable_interface == PLAYER {
                    assert_eq!(snapshot.metadata, Metadata::default());
                } else {
                    assert_eq!(snapshot.metadata.title.as_deref(), Some("Late track"));
                }
                service.stop();
                client.close_sync(gio::Cancellable::NONE).unwrap();
            }
        })
        .unwrap();
}

#[test]
fn discovery_metadata_changes_and_command_results() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let first = Player::new(&address, "org.mpris.MediaPlayer2.first", "Track one");
            first.acquire(false);
            let impostor = Player::new(&address, "org.mpris.MediaPlayer2Impostor", "Ignore me");
            impostor.acquire(false);
            let connection = connect(&address);
            let service = MediaService::on_connection(&connection);
            wait(&context, || service.state().players.len() == 1);
            assert!(service.state().available);
            let state = service.state();
            let player = &state.players[0];
            assert_eq!(player.name, first.name);
            assert_eq!(player.owner, first.owner());
            assert_eq!(player.identity.as_deref(), Some("Fixture player"));
            assert_eq!(player.playback_status, PlaybackStatus::Playing);
            assert_eq!(player.metadata.title.as_deref(), Some("Track one"));
            assert_eq!(player.metadata.artist().as_deref(), Some("One, Two"));
            assert!(player.capabilities.can_control && player.capabilities.can_raise);

            let second = Player::new(&address, "org.mpris.MediaPlayer2.second", "Track two");
            second.acquire(false);
            wait(&context, || service.state().players.len() == 2);
            let changes = Rc::new(Cell::new(0));
            let recorded = changes.clone();
            service.connect_local("changed", false, move |_| {
                recorded.set(recorded.get() + 1);
                None
            });
            first.property(
                PLAYER,
                "Metadata",
                HashMap::from([("xesam:title", "Replacement".to_variant())]).to_variant(),
            );
            wait(&context, || {
                service.state().players[0].metadata.title.as_deref() == Some("Replacement")
            });
            let replacement = service.state().players[0].metadata.clone();
            assert_eq!(replacement.album, None);
            assert_eq!(replacement.art_url, None);
            assert!(replacement.artists.is_empty());
            assert_eq!(changes.get(), 1);
            assert_eq!(
                state.players[0].metadata.title.as_deref(),
                Some("Track one"),
                "Snapshots own their strings"
            );

            first.property(
                PLAYER,
                "Metadata",
                HashMap::from([
                    ("xesam:title", 123_u32.to_variant()),
                    ("xesam:album", false.to_variant()),
                    ("xesam:artist", "Wrong type".to_variant()),
                    ("mpris:artUrl", vec!["Wrong type"].to_variant()),
                ])
                .to_variant(),
            );
            wait(&context, || {
                service.state().players[0].metadata == Metadata::default()
            });
            first.invalidated(PLAYER, "Metadata", metadata("Invalidated then fetched"));
            wait(&context, || {
                service.state().players[0].metadata.title.as_deref()
                    == Some("Invalidated then fetched")
            });
            first.property(ROOT, "Identity", "Renamed player".to_variant());
            first.property(ROOT, "CanRaise", false.to_variant());
            first.property(PLAYER, "PlaybackStatus", "Unexpected status".to_variant());
            first.property(PLAYER, "CanGoNext", false.to_variant());
            wait(&context, || {
                let player = &service.state().players[0];
                player.identity.as_deref() == Some("Renamed player")
                    && player.playback_status == PlaybackStatus::Unknown
                    && !player.capabilities.can_raise
                    && !player.capabilities.can_go_next
            });

            // Preserve valid existing controls; applications ultimately decide
            // whether to accept requests even when capability properties disagree.
            for (action, interface, method) in [
                (MediaAction::Play, PLAYER, "Play"),
                (MediaAction::Pause, PLAYER, "Pause"),
                (MediaAction::PlayPause, PLAYER, "PlayPause"),
                (MediaAction::Stop, PLAYER, "Stop"),
                (MediaAction::Next, PLAYER, "Next"),
                (MediaAction::Previous, PLAYER, "Previous"),
                (MediaAction::Raise, ROOT, "Raise"),
            ] {
                let result = Rc::new(RefCell::new(None));
                let recorded = result.clone();
                service.command(&first.name, action, move |reply| {
                    recorded.replace(Some(reply));
                });
                wait(&context, || result.borrow().is_some());
                result.borrow_mut().take().unwrap().unwrap();
                assert_eq!(
                    first.calls.borrow().last().unwrap(),
                    &(interface.into(), method.into())
                );
            }
            first.mode.set(Reply::Failure);
            let result = Rc::new(RefCell::new(None));
            let recorded = result.clone();
            service.command(&first.name, MediaAction::Play, move |reply| {
                recorded.replace(Some(reply));
            });
            wait(&context, || result.borrow().is_some());
            assert!(
                result
                    .borrow_mut()
                    .take()
                    .unwrap()
                    .unwrap_err()
                    .message()
                    .contains("Fixture rejected")
            );
            first.mode.set(Reply::Malformed);
            let recorded = result.clone();
            service.command(&first.name, MediaAction::Play, move |reply| {
                recorded.replace(Some(reply));
            });
            wait(&context, || result.borrow().is_some());
            assert!(
                result
                    .borrow_mut()
                    .take()
                    .unwrap()
                    .unwrap_err()
                    .matches(gio::IOErrorEnum::InvalidData)
            );

            second.release();
            wait(&context, || service.state().players.len() == 1);
            let recorded = result.clone();
            service.command(&second.name, MediaAction::Play, move |reply| {
                recorded.replace(Some(reply));
            });
            assert!(
                result
                    .borrow_mut()
                    .take()
                    .unwrap()
                    .unwrap_err()
                    .matches(gio::IOErrorEnum::NotConnected)
            );
            first.release();
            wait(&context, || service.state().players.is_empty());
            assert!(
                service.state().available,
                "An empty media inventory is valid"
            );
            service.stop();
            connection.close_sync(gio::Cancellable::NONE).unwrap();
        })
        .unwrap();
}

#[test]
fn discovery_cancellation_and_reentrant_observers() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let player = Player::new(&address, "org.mpris.MediaPlayer2.race", "Initial track");
            player.acquire(false);
            let connection = connect(&address);
            let service = MediaService::on_connection(&connection);
            // The name is replaced before the asynchronous initial inventory can
            // complete. Neither enumeration nor the name event may publish twice.
            let replacement = Player::new(&address, &player.name, "Replacement track");
            replacement.acquire(true);
            let observed = Rc::new(RefCell::new(Vec::new()));
            let recorded = observed.clone();
            let weak = service.downgrade();
            service.connect_local("changed", false, move |_| {
                if let Some(service) = weak.upgrade() {
                    let state = service.state();
                    if !state.players.is_empty() {
                        recorded.borrow_mut().push(state.players.clone());
                        service.stop();
                    }
                }
                None
            });
            wait(&context, || !observed.borrow().is_empty());
            assert_eq!(observed.borrow().len(), 1);
            assert_eq!(observed.borrow()[0].len(), 1);
            assert_eq!(observed.borrow()[0][0].owner, replacement.owner());
            assert!(service.state().players.is_empty());
            assert!(!service.state().available);
            context.block_on(glib::timeout_future(Duration::from_millis(30)));
            assert_eq!(
                observed.borrow().len(),
                1,
                "Late discovery replies remain cancelled"
            );
            connection.close_sync(gio::Cancellable::NONE).unwrap();
        })
        .unwrap();
}

#[test]
fn replacement_restart_deadline_and_owner_cleanup() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let first = Player::new(&address, "org.mpris.MediaPlayer2.replaced", "Old owner");
            first.acquire(false);
            let connection = connect(&address);
            let service = MediaService::on_connection(&connection);
            wait(&context, || service.state().players.len() == 1);
            first.mode.set(Reply::Hold);
            let pending = Rc::new(RefCell::new(None));
            let recorded = pending.clone();
            service.command(&first.name, MediaAction::Next, move |reply| {
                recorded.replace(Some(reply));
            });
            wait(&context, || !first.held.borrow().is_empty());
            let replacement = Player::new(&address, &first.name, "New owner");
            replacement.acquire(true);
            wait(&context, || {
                pending.borrow().is_some()
                    && service
                        .state()
                        .players
                        .first()
                        .is_some_and(|player| player.owner == replacement.owner())
            });
            assert!(
                pending
                    .borrow_mut()
                    .take()
                    .unwrap()
                    .unwrap_err()
                    .matches(gio::IOErrorEnum::Cancelled)
            );
            assert!(
                replacement.calls.borrow().is_empty(),
                "Replacement did not receive an old owner's request"
            );
            assert_eq!(
                service.state().players[0].metadata.title.as_deref(),
                Some("New owner")
            );
            first.finish_held();
            first.property(PLAYER, "Metadata", metadata("Stale update"));
            context.block_on(glib::timeout_future(Duration::from_millis(30)));
            assert_eq!(
                service.state().players[0].metadata.title.as_deref(),
                Some("New owner")
            );

            replacement.mode.set(Reply::Hold);
            let recorded = pending.clone();
            service.command(&replacement.name, MediaAction::Stop, move |reply| {
                recorded.replace(Some(reply));
            });
            wait(&context, || !replacement.held.borrow().is_empty());
            wait(&context, || pending.borrow().is_some());
            let error = pending.borrow_mut().take().unwrap().unwrap_err();
            assert!(
                error.matches(gio::IOErrorEnum::TimedOut) || error.matches(gio::DBusError::NoReply),
                "{error}"
            );
            replacement.finish_held();

            service.stop();
            assert!(!service.state().available && service.state().players.is_empty());
            service.start();
            wait(&context, || service.state().players.len() == 1);
            assert_eq!(service.state().players[0].owner, replacement.owner());
            let recorded = pending.clone();
            service.command(&replacement.name, MediaAction::Pause, move |reply| {
                recorded.replace(Some(reply));
            });
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
            wait(&context, || pending.borrow().is_some());
            assert!(
                pending
                    .borrow_mut()
                    .take()
                    .unwrap()
                    .unwrap_err()
                    .matches(gio::IOErrorEnum::Cancelled)
            );
            replacement.finish_held();
            for _ in 0..20 {
                let service = MediaService::on_connection(&connection);
                let weak = service.downgrade();
                drop(service);
                assert!(
                    weak.upgrade().is_none(),
                    "Pending discovery must not own the service"
                );
            }
            context.block_on(glib::timeout_future(Duration::from_millis(30)));
            let service = MediaService::on_connection(&connection);
            wait(&context, || service.state().players.len() == 1);
            replacement.release();
            wait(&context, || service.state().players.is_empty());
            replacement.acquire(false);
            wait(&context, || service.state().players.len() == 1);
            connection.close_sync(gio::Cancellable::NONE).unwrap();
            wait(&context, || {
                !service.state().available && service.state().players.is_empty()
            });
        })
        .unwrap();
}
