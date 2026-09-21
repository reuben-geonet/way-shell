//! A protocol-level fixture exercises proxy ownership independently of GTK.
use gio::prelude::*;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    os::{
        fd::AsFd,
        unix::net::{UnixListener, UnixStream},
    },
    sync::mpsc,
    time::{Duration, Instant},
};
use way_shell::services::wayland::{ToplevelAction, WaylandService};
const BURST: usize = 30_000;

fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}
fn string(value: &str) -> Vec<u8> {
    let mut bytes = words(&[value.len() as u32 + 1]);
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(0);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
}
fn array(values: &[u32]) -> Vec<u8> {
    let mut bytes = words(&[values.len() as u32 * 4]);
    bytes.extend(words(values));
    bytes
}
fn send(stream: &mut UnixStream, id: u32, opcode: u16, body: Vec<u8>) {
    let mut bytes = words(&[id, ((body.len() as u32 + 8) << 16) | u32::from(opcode)]);
    bytes.extend(body);
    for fragment in bytes.chunks(3) {
        stream.write_all(fragment).unwrap();
    }
}
fn receive(stream: &mut UnixStream) -> Option<(u32, u16, Vec<u8>)> {
    let mut header = [0; 8];
    match stream.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return None,
        Err(error) => panic!("{error}"),
    };
    let id = u32::from_ne_bytes(header[..4].try_into().unwrap());
    let encoded = u32::from_ne_bytes(header[4..].try_into().unwrap());
    let mut body = vec![0; (encoded >> 16) as usize - 8];
    stream.read_exact(&mut body).unwrap();
    Some((id, encoded as u16, body))
}
fn word(bytes: &[u8], index: usize) -> u32 {
    u32::from_ne_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap())
}
fn server(mut stream: UnixStream, steps: mpsc::Sender<()>, resume: mpsc::Receiver<()>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let (_, get_registry, bytes) = receive(&mut stream).unwrap();
    assert_eq!(get_registry, 1);
    let registry = word(&bytes, 0);
    let (_, sync, bytes) = receive(&mut stream).unwrap();
    assert_eq!(sync, 0);
    let callback = word(&bytes, 0);
    for (id, interface, version) in [
        (10, "wl_output", 99),
        (20, "wl_seat", 99),
        (30, "zwlr_foreign_toplevel_manager_v1", 99),
        (40, "zwlr_gamma_control_manager_v1", 1),
        (50, "zwp_keyboard_shortcuts_inhibit_manager_v1", 1),
    ] {
        let mut bytes = words(&[id]);
        bytes.extend(string(interface));
        bytes.extend(words(&[version]));
        send(&mut stream, registry, 0, bytes);
    }
    send(&mut stream, callback, 0, words(&[1]));
    let mut proxies = BTreeMap::new();
    let second;
    loop {
        let (id, opcode, bytes) = receive(&mut stream).unwrap();
        if id == 1 {
            assert_eq!(opcode, 0);
            second = word(&bytes, 0);
            break;
        }
        assert_eq!(id, registry);
        assert_eq!(opcode, 0);
        let name = word(&bytes, 0);
        let len = word(&bytes, 1) as usize;
        let offset = 8 + len.next_multiple_of(4);
        let version = u32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap());
        assert_eq!(
            version,
            match name {
                10 => 4,
                20 => 7,
                30 => 3,
                40 => 1,
                _ => panic!("Unknown global"),
            }
        );
        proxies.insert(
            name,
            u32::from_ne_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()),
        );
    }
    let output = proxies[&10];
    let seat = proxies[&20];
    let manager = proxies[&30];
    let top = 0xff000000;
    let mut geometry = words(&[0, 0, 100, 100, 0]);
    geometry.extend(string("Example"));
    geometry.extend(string("Virtual Display"));
    geometry.extend(words(&[0]));
    send(&mut stream, output, 0, geometry);
    send(&mut stream, output, 4, string("TEST-1"));
    send(&mut stream, output, 5, string("Fixture output"));
    send(&mut stream, output, 3, words(&[2]));
    send(&mut stream, output, 1, words(&[1, 1920, 1080, 60000]));
    send(&mut stream, output, 2, vec![]);
    send(&mut stream, seat, 0, words(&[3]));
    send(&mut stream, seat, 1, string("seat0"));
    send(&mut stream, manager, 0, words(&[top]));
    send(&mut stream, top, 0, string("Window 日本語"));
    send(&mut stream, top, 1, string("org.example.Fixture"));
    send(&mut stream, top, 2, words(&[output]));
    send(&mut stream, top, 4, array(&[2]));
    send(&mut stream, top, 5, vec![]);
    send(&mut stream, second, 0, words(&[2]));
    resume.recv().unwrap();
    for _ in 0..BURST {
        let (id, opcode, bytes) = receive(&mut stream).unwrap();
        assert_eq!((id, opcode), (top, 4));
        assert_eq!(word(&bytes, 0), seat);
    }
    send(&mut stream, top, 0, string("Changed title"));
    send(&mut stream, top, 4, array(&[0, 1, 3, u32::MAX]));
    send(&mut stream, top, 5, vec![]);
    steps.send(()).unwrap();
    resume.recv().unwrap();
    let (id, opcode, bytes) = receive(&mut stream).unwrap();
    assert_eq!((id, opcode), (proxies[&40], 0));
    let gamma = word(&bytes, 0);
    assert_eq!(word(&bytes, 1), output);
    send(&mut stream, gamma, 0, words(&[4]));
    let socket = gio::Socket::from_fd(stream.as_fd().try_clone_to_owned().unwrap()).unwrap();
    socket.set_timeout(5);
    let mut bytes = [0_u8; 8];
    let mut controls = gio::SocketControlMessages::new();
    let (length, _) = gio::prelude::SocketExtManual::receive_message(
        &socket,
        None,
        &mut [gio::InputVector::new(&mut bytes)],
        Some(&mut controls),
        0,
        gio::Cancellable::NONE,
    )
    .unwrap();
    assert_eq!(length, 8);
    assert_eq!(word(&bytes, 0), gamma);
    assert_eq!(word(&bytes, 1), 8 << 16);
    assert_eq!(controls.len(), 1);
    let message = controls[0].downcast_ref::<gio::UnixFDMessage>().unwrap();
    let mut file = std::fs::File::from(message.fd_list().get(0).unwrap());
    let mut ramp = Vec::new();
    file.read_to_end(&mut ramp).unwrap();
    let expected: Vec<_> = way_shell_core::gamma::ramp(4, 4500)
        .unwrap()
        .into_iter()
        .flat_map(u16::to_ne_bytes)
        .collect();
    assert_eq!(ramp, expected);
    drop((socket, controls, file));
    stream.set_nonblocking(false).unwrap();
    send(&mut stream, gamma, 1, vec![]);
    let (id, opcode, _) = receive(&mut stream).unwrap();
    assert_eq!((id, opcode), (gamma, 1));
    steps.send(()).unwrap();
    resume.recv().unwrap();
    send(&mut stream, registry, 1, words(&[20]));
    send(&mut stream, registry, 1, words(&[10]));
    let (id, opcode, _) = receive(&mut stream).unwrap();
    assert_eq!((id, opcode), (seat, 3));
    let (id, opcode, _) = receive(&mut stream).unwrap();
    assert_eq!((id, opcode), (output, 0));
    steps.send(()).unwrap();
    resume.recv().unwrap();
    send(&mut stream, top, 6, vec![]);
    let (id, opcode, _) = receive(&mut stream).unwrap();
    assert_eq!((id, opcode), (top, 7));
    steps.send(()).unwrap();
    resume.recv().unwrap();
    while receive(&mut stream).is_some() {}
}
fn until(context: &glib::MainContext, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "Wayland fixture timed out");
        context.block_on(glib::timeout_future(Duration::from_millis(2)));
    }
}
#[test]
fn proxy_versions_snapshots_removal_filters_actions_and_cleanup() {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let directory = std::env::temp_dir()
                .join(format!("way-shell-wl-test-{}", glib::uuid_string_random()));
            std::fs::create_dir(&directory).unwrap();
            let path = directory.join("wayland-test");
            let listener = UnixListener::bind(&path).unwrap();
            // This integration binary has one test; configure before spawning.
            unsafe {
                std::env::remove_var("WAYLAND_SOCKET");
                std::env::set_var("WAYLAND_DISPLAY", &path);
            }
            let (steps_tx, steps_rx) = mpsc::channel();
            let (resume_tx, resume_rx) = mpsc::channel();
            let server = std::thread::spawn(move || {
                server(listener.accept().unwrap().0, steps_tx, resume_rx)
            });
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
            let service = WaylandService::with_settings(settings.clone()).unwrap();
            until(&context, || service.is_ready());
            assert!(service.error().is_none());
            assert!(
                service.has_foreign_toplevel()
                    && service.has_gamma()
                    && service.has_shortcut_inhibition()
            );
            let outputs = service.outputs();
            assert_eq!(outputs.len(), 1);
            assert_eq!(outputs[0].name.as_deref(), Some("TEST-1"));
            assert_eq!(
                (outputs[0].scale, outputs[0].width, outputs[0].height),
                (2, 1920, 1080)
            );
            assert_eq!(service.seats()[0].name.as_deref(), Some("seat0"));
            let tops = service.toplevels();
            assert_eq!(tops.len(), 1);
            assert!(tops[0].active);
            assert_eq!(tops[0].outputs.iter().copied().collect::<Vec<_>>(), [10]);
            let id = tops[0].id;
            for _ in 0..BURST {
                service.action(id, ToplevelAction::Activate).unwrap();
            }
            // The compositor deliberately does not read until this timer has
            // run. A spinning flush would hang here instead of yielding.
            context.block_on(glib::timeout_future(Duration::from_millis(10)));
            resume_tx.send(()).unwrap();
            until(&context, || {
                service
                    .toplevels()
                    .first()
                    .is_some_and(|top| top.title.as_deref() == Some("Changed title"))
            });
            steps_rx.recv().unwrap();
            let top = &service.toplevels()[0];
            assert!(top.maximized && top.minimized && top.fullscreen && !top.active);
            settings
                .set_string("ignored-toplevels-app-ids", "org.example.Fixture")
                .unwrap();
            assert!(service.toplevels().is_empty());
            settings
                .set_string("ignored-toplevels-app-ids", "")
                .unwrap();
            assert_eq!(service.toplevels()[0].id, id);
            resume_tx.send(()).unwrap();
            service.set_temperature(4500).unwrap();
            assert!(service.gamma_available());
            assert!(service.gamma_enabled());
            until(&context, || !service.gamma_enabled());
            assert!(!service.gamma_available());
            steps_rx.recv().unwrap();
            resume_tx.send(()).unwrap();
            until(&context, || {
                service.seats().is_empty() && service.outputs().is_empty()
            });
            assert!(service.action(id, ToplevelAction::Activate).is_err());
            assert!(service.toplevels()[0].outputs.is_empty());
            steps_rx.recv().unwrap();
            resume_tx.send(()).unwrap();
            until(&context, || service.toplevels().is_empty());
            assert!(service.action(id, ToplevelAction::Close).is_err());
            steps_rx.recv().unwrap();
            resume_tx.send(()).unwrap();
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
            server.join().unwrap();
            for _ in 0..10 {
                std::fs::remove_file(&path).unwrap();
                let listener = UnixListener::bind(&path).unwrap();
                let server = std::thread::spawn(move || {
                    let mut stream = listener.accept().unwrap().0;
                    assert!(receive(&mut stream).is_some());
                    assert!(receive(&mut stream).is_some());
                    // Compositor dies before the registry handshake finishes.
                });
                let service = WaylandService::with_settings(settings.clone()).unwrap();
                until(&context, || service.error().is_some());
                assert!(!service.is_ready());
                assert!(!service.has_foreign_toplevel() && !service.has_gamma());
                assert!(service.action(1, ToplevelAction::Close).is_err());
                let weak = service.downgrade();
                drop(service);
                assert!(weak.upgrade().is_none());
                server.join().unwrap();
            }
            std::fs::remove_dir_all(directory).unwrap();
        })
        .unwrap();
}
