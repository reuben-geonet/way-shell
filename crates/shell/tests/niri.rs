use gio::prelude::*;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    time::{Duration, Instant},
};
use way_shell::services::wm::WindowManager;
use way_shell_core::wm::Action;

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn line(peer: &mut BufReader<UnixStream>) -> Value {
    let mut line = String::new();
    assert!(peer.read_line(&mut line).unwrap() > 0, "unexpected EOF");
    serde_json::from_str(&line).unwrap()
}
fn send(peer: &mut BufReader<UnixStream>, value: &Value) {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.push(b'\n');
    for part in bytes.chunks(7) {
        peer.get_mut().write_all(part).unwrap();
    }
}
fn accept(listener: &UnixListener) -> BufReader<UnixStream> {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((peer, _)) => {
                peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                return BufReader::new(peer);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < end, "service did not reconnect");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("accept: {error}"),
        }
    }
}
fn peers(listener: &UnixListener) -> (BufReader<UnixStream>, BufReader<UnixStream>) {
    let mut first = accept(listener);
    let mut second = accept(listener);
    // Either asynchronous connect may finish first. Only the event connection
    // writes before receiving the initial snapshot.
    first
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let mut initial = String::new();
    let first_is_events = match first.read_line(&mut initial) {
        Ok(length) => {
            assert!(length > 0);
            assert_eq!(
                serde_json::from_str::<Value>(&initial).unwrap(),
                json!("EventStream")
            );
            true
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            assert!(initial.is_empty());
            assert_eq!(line(&mut second), json!("EventStream"));
            false
        }
        Err(error) => panic!("subscription: {error}"),
    };
    first
        .get_mut()
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    if first_is_events {
        (second, first)
    } else {
        (first, second)
    }
}
fn wait(context: &glib::MainContext, predicate: impl Fn() -> bool) {
    context
        .block_on(glib::future_with_timeout(Duration::from_secs(8), async {
            while !predicate() {
                glib::timeout_future(Duration::from_millis(10)).await;
            }
        }))
        .expect("Niri service did not reach expected state");
}
#[test]
fn separate_streams_actions_malformed_reply_reconnection_and_cleanup() {
    let directory =
        Directory(std::env::temp_dir().join(format!("way-shell-niri.{}", std::process::id())));
    fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("socket");
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        for cycle in 0..2 {
            let (mut commands, mut events) = peers(&listener);
            let mut workspaces: Value = serde_json::from_slice(include_bytes!(
                "../../../tests/fixtures/niri-workspaces.json"
            ))
            .unwrap();
            if cycle == 1 {
                workspaces["Ok"]["Workspaces"][0]["id"] = json!(u64::MAX);
            }
            send(&mut events, &json!({"Ok":"Handled"}));
            send(&mut events, &json!({"FutureEvent":{"ignored":true}}));
            send(
                &mut events,
                &json!({"WorkspacesChanged":{"workspaces":workspaces["Ok"]["Workspaces"]}}),
            );
            assert_eq!(line(&mut commands), json!("Outputs"));
            let outputs =
                serde_json::from_slice(include_bytes!("../../../tests/fixtures/niri-outputs.json"))
                    .unwrap();
            send(&mut commands, &outputs);
            if cycle == 0 {
                assert_eq!(
                    line(&mut commands),
                    json!({"Action":{"FocusWorkspace":{"reference":{"Id":4294967313_u64}}}})
                );
                send(&mut commands, &json!({"Err":"fixture denied action"}));
                let rename = line(&mut commands);
                assert_eq!(
                    rename,
                    json!({"Action":{"SetWorkspaceName":{"name":"renamed \"日本語\"","workspace":null}}})
                );
                send(&mut commands, &json!({"Ok":"Handled"}));
                workspaces["Ok"]["Workspaces"][0]["name"] = json!("renamed \"日本語\"");
                send(
                    &mut events,
                    &json!({"WorkspacesChanged":{"workspaces":workspaces["Ok"]["Workspaces"]}}),
                );
                assert_eq!(line(&mut commands), json!("Outputs"));
                send(&mut commands, &outputs);
                let movement = line(&mut commands);
                assert_eq!(
                    movement["Action"]["MoveWindowToWorkspace"]["reference"],
                    json!({"Id":4294967313_u64})
                );
                assert_eq!(movement["Action"]["MoveWindowToWorkspace"]["focus"], false);
                send(&mut commands, &json!({"Ok":{"Outputs":[]}}));
                // Malformed response must close both streams before reconnect.
                let mut byte = [0];
                assert_eq!(commands.read(&mut byte).unwrap(), 0);
                assert_eq!(events.read(&mut byte).unwrap(), 0);
            } else {
                // Dropping the owner closes both sources and file descriptors.
                let mut byte = [0];
                assert_eq!(commands.read(&mut byte).unwrap(), 0);
                assert_eq!(events.read(&mut byte).unwrap(), 0);
            }
        }
    });
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let source = gio::SettingsSchemaSource::from_directory(
                env!("WAY_SHELL_TEST_SCHEMAS"),
                None,
                false,
            )
            .unwrap();
            let schema = source
                .lookup("org.ldelossa.way-shell.window-manager", false)
                .unwrap();
            let settings =
                gio::Settings::new_full(&schema, Some(&gio::memory_settings_backend_new()), None);
            let service =
                WindowManager::niri_with_settings(settings, path, directory.0.clone()).unwrap();
            let lost = std::rc::Rc::new(std::cell::Cell::new(false));
            let record = lost.clone();
            service.connect_local("connection-changed", false, move |values| {
                if !values[1].get::<bool>().unwrap() {
                    record.set(true);
                }
                None
            });
            wait(&context, || {
                service.is_connected()
                    && service.workspaces().len() == 2
                    && service.outputs().len() == 2
            });
            let target = service
                .workspaces()
                .into_iter()
                .find(|workspace| workspace.id == 4294967313)
                .unwrap();
            assert!(target.visible && !target.focused);
            assert_eq!(target.num, 2);
            service
                .perform(&Action::FocusWorkspace((&target).into()))
                .unwrap();
            service
                .perform(&Action::RenameWorkspace("renamed \"日本語\"".into()))
                .unwrap();
            wait(&context, || {
                service
                    .workspaces()
                    .iter()
                    .any(|workspace| workspace.name == "renamed \"日本語\"")
            });
            assert!(
                service.is_connected(),
                "a rejected action must leave IPC usable"
            );
            service
                .perform(&Action::MoveWindowToWorkspace((&target).into()))
                .unwrap();
            wait(&context, || {
                lost.get()
                    && service.is_connected()
                    && service
                        .workspaces()
                        .iter()
                        .any(|workspace| workspace.id == u64::MAX)
                    && service.outputs().len() == 2
            });
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
            context.block_on(glib::timeout_future(Duration::from_millis(20)));
        })
        .unwrap();
    server.join().unwrap();
}
