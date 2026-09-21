use gio::prelude::*;
use std::{
    fs,
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use way_shell::services::wm::WindowManager;
use way_shell_core::{sway, wm::Action};

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn receive(peer: &mut UnixStream) -> (u32, Vec<u8>) {
    let mut header = [0; 14];
    peer.read_exact(&mut header).unwrap();
    assert_eq!(&header[..6], b"i3-ipc");
    let length = u32::from_le_bytes(header[6..10].try_into().unwrap()) as usize;
    assert!(length < 1024 * 1024);
    let mut payload = vec![0; length];
    peer.read_exact(&mut payload).unwrap();
    (
        u32::from_le_bytes(header[10..].try_into().unwrap()),
        payload,
    )
}

fn send(peer: &mut UnixStream, kind: u32, payload: &[u8]) {
    for part in sway::encode(kind, payload).unwrap().chunks(7) {
        peer.write_all(part).unwrap();
    }
}

fn wait(context: &glib::MainContext, predicate: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(
            Instant::now() < end,
            "Sway service did not reach expected state"
        );
        context.block_on(glib::timeout_future(Duration::from_millis(10)));
    }
}

#[test]
fn snapshots_settings_actions_reconnection_and_cleanup() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = Directory(std::env::temp_dir().join(format!(
        "way-shell-sway.{}.{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("socket");
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        for cycle in 0..2 {
            let end = Instant::now() + Duration::from_secs(5);
            let mut peer = loop {
                match listener.accept() {
                    Ok((peer, _)) => break peer,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < end, "service did not reconnect");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut workspaces: serde_json::Value = serde_json::from_slice(include_bytes!(
                "../../../tests/fixtures/sway-workspaces.json"
            ))
            .unwrap();
            if cycle == 1 {
                workspaces[0]["id"] = serde_json::json!(4294967313_u64);
            }
            let (kind, subscribe) = receive(&mut peer);
            assert_eq!(kind, sway::SUBSCRIBE);
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&subscribe).unwrap(),
                serde_json::json!(["workspace", "output"])
            );
            send(&mut peer, sway::SUBSCRIBE, br#"{"success":true}"#);
            assert_eq!(receive(&mut peer).0, sway::WORKSPACES);
            send(
                &mut peer,
                sway::WORKSPACES,
                &serde_json::to_vec(&workspaces).unwrap(),
            );
            assert_eq!(receive(&mut peer).0, sway::OUTPUTS);
            send(
                &mut peer,
                sway::OUTPUTS,
                include_bytes!("../../../tests/fixtures/sway-outputs.json"),
            );
            if cycle == 0 {
                // Changing the sort setting requests the compositor's original order.
                assert_eq!(receive(&mut peer).0, sway::WORKSPACES);
                send(
                    &mut peer,
                    sway::WORKSPACES,
                    &serde_json::to_vec(&workspaces).unwrap(),
                );
                let (kind, command) = receive(&mut peer);
                assert_eq!(kind, sway::COMMAND);
                assert_eq!(
                    String::from_utf8(command).unwrap(),
                    "rename workspace to \"renamed; 日本語\""
                );
                send(&mut peer, sway::COMMAND, br#"[{"success":true}]"#);
                workspaces[0]["name"] = serde_json::json!("renamed; 日本語");
                send(
                    &mut peer,
                    sway::WORKSPACE_EVENT,
                    &serde_json::to_vec(
                        &serde_json::json!({"change":"rename", "current":workspaces[0]}),
                    )
                    .unwrap(),
                );
                assert_eq!(receive(&mut peer).0, sway::WORKSPACES);
                send(
                    &mut peer,
                    sway::WORKSPACES,
                    &serde_json::to_vec(&workspaces).unwrap(),
                );
            } else {
                let mut byte = [0];
                assert_eq!(
                    peer.read(&mut byte).unwrap(),
                    0,
                    "dropping the service must close its connection"
                );
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
                WindowManager::sway_with_settings(settings.clone(), path, directory.0.clone())
                    .unwrap();
            let renamed = std::rc::Rc::new(std::cell::Cell::new(false));
            let recorded = renamed.clone();
            service.connect_local("workspaces-changed", false, move |values| {
                if values[0]
                    .get::<WindowManager>()
                    .unwrap()
                    .workspaces()
                    .iter()
                    .any(|workspace| workspace.name == "renamed; 日本語")
                {
                    recorded.set(true);
                }
                None
            });
            wait(&context, || {
                service.workspaces().len() == 2 && service.outputs().len() == 2
            });
            assert!(service.is_connected());
            assert_eq!(service.outputs()[1].current_workspace, None);
            settings
                .set_boolean("sort-workspaces-alphabetical", false)
                .unwrap();
            service
                .perform(&Action::RenameWorkspace("renamed; 日本語".into()))
                .unwrap();
            wait(&context, || {
                renamed.get()
                    && service
                        .workspaces()
                        .iter()
                        .any(|workspace| workspace.id == 4294967313)
                    && service.outputs().len() == 2
            });
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
            settings
                .set_boolean("sort-workspaces-alphabetical", true)
                .unwrap();
            context.block_on(glib::timeout_future(Duration::from_millis(20)));
        })
        .unwrap();
    server.join().unwrap();
}
