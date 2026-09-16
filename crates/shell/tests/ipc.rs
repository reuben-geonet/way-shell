//! Real Unix datagrams against the permanent, nonblocking GLib IPC server.
use std::{
    cell::{Cell, RefCell},
    fs,
    future::Future,
    io,
    os::{
        linux::net::SocketAddrExt,
        unix::net::{SocketAddr, UnixDatagram},
    },
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll},
    time::{Duration, Instant},
};
use way_shell::services::ipc::{Command, DispatchFuture, IpcService};
use way_shell_core::ipc::{Request, decode_response};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "way-shell-server.{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> PathBuf {
        self.0.join("way-shell.sock")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn client() -> UnixDatagram {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let address = SocketAddr::from_abstract_name(format!(
        "way-shell-server-client-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
    .unwrap();
    let socket = UnixDatagram::bind_addr(&address).unwrap();
    socket.set_nonblocking(true).unwrap();
    socket
}
fn wait(context: &glib::MainContext, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while !done() {
        assert!(Instant::now() < deadline, "IPC fixture timed out");
        context.block_on(glib::timeout_future(Duration::from_millis(2)));
    }
}
fn response(context: &glib::MainContext, client: &UnixDatagram) -> bool {
    let mut bytes = [0; 5];
    let mut received = None;
    wait(context, || match client.recv(&mut bytes) {
        Ok(size) => {
            received = Some(size);
            true
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => false,
        Err(error) => panic!("receive failed: {error}"),
    });
    decode_response(&bytes[..received.unwrap()]).unwrap()
}
fn send(context: &glib::MainContext, peer: &UnixDatagram, bytes: &[u8], path: &Path) {
    wait(context, || match peer.send_to(bytes, path) {
        Ok(length) => {
            assert_eq!(length, bytes.len());
            true
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => false,
        Err(error) => panic!("send failed: {error}"),
    });
}
fn success(_: Command) -> DispatchFuture {
    Box::pin(async { Ok(()) })
}
fn socket_entries(path: &Path) -> usize {
    let target = path.to_string_lossy();
    fs::read_to_string("/proc/net/unix")
        .unwrap()
        .lines()
        .filter(|line| line.split_whitespace().last() == Some(target.as_ref()))
        .count()
}

#[test]
fn valid_wire_requests_async_results_and_concurrent_clients() {
    let directory = Directory::new();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let calls = Rc::new(RefCell::new(Vec::new()));
            let captured = calls.clone();
            let service = IpcService::bind(&directory.0, move |command| {
                captured.borrow_mut().push(command);
                Box::pin(async move {
                    if command == Command::VolumeUp {
                        glib::timeout_future(Duration::from_millis(40)).await;
                    }
                    if command == Command::VolumeDown {
                        Err("device unavailable".into())
                    } else {
                        Ok(())
                    }
                })
            })
            .unwrap();
            let slow = client();
            let fast = client();
            slow.send_to(
                &Request::new(2, None).unwrap().encode(),
                service.socket_path(),
            )
            .unwrap();
            fast.send_to(
                &Request::new(3, None).unwrap().encode(),
                service.socket_path(),
            )
            .unwrap();
            assert!(!response(&context, &fast));
            assert!(response(&context, &slow));
            assert_eq!(*calls.borrow(), [Command::VolumeUp, Command::VolumeDown]);
            let peers: Vec<_> = (1..=36)
                .map(|opcode| {
                    let peer = client();
                    send(
                        &context,
                        &peer,
                        &Request::new(opcode, (opcode == 4).then_some(0.375))
                            .unwrap()
                            .encode(),
                        service.socket_path(),
                    );
                    (opcode, peer)
                })
                .collect();
            for (opcode, peer) in peers {
                assert_eq!(response(&context, &peer), opcode != 3);
            }
            assert_eq!(
                &calls.borrow()[2..],
                &[
                    Command::MessageTrayOpen,
                    Command::VolumeUp,
                    Command::VolumeDown,
                    Command::VolumeSet(0.375),
                    Command::VolumeMute,
                    Command::BrightnessUp,
                    Command::BrightnessDown,
                    Command::ThemeDark,
                    Command::ThemeLight,
                    Command::DumpDarkTheme,
                    Command::DumpLightTheme,
                    Command::ActivitiesShow,
                    Command::ActivitiesHide,
                    Command::ActivitiesToggle,
                    Command::AppSwitcherShow,
                    Command::AppSwitcherHide,
                    Command::AppSwitcherToggle,
                    Command::WorkspaceSwitcherShow,
                    Command::WorkspaceSwitcherHide,
                    Command::WorkspaceSwitcherToggle,
                    Command::OutputSwitcherShow,
                    Command::OutputSwitcherHide,
                    Command::OutputSwitcherToggle,
                    Command::WorkspaceAppSwitcherShow,
                    Command::WorkspaceAppSwitcherHide,
                    Command::WorkspaceAppSwitcherToggle,
                    Command::BlueLightFilterEnable,
                    Command::BlueLightFilterDisable,
                    Command::KeyboardBrightnessUp,
                    Command::KeyboardBrightnessDown,
                    Command::RenameSwitcherShow,
                    Command::RenameSwitcherHide,
                    Command::RenameSwitcherToggle,
                    Command::ShortcutsShow,
                    Command::ShortcutsHide,
                    Command::ShortcutsToggle,
                ]
            );
            assert!(calls.borrow().contains(&Command::VolumeSet(0.375)));
            assert_eq!(calls.borrow().last(), Some(&Command::ShortcutsToggle));
            let pathname = directory.0.join("legacy-client.sock");
            let legacy = UnixDatagram::bind(&pathname).unwrap();
            legacy.set_nonblocking(true).unwrap();
            legacy
                .send_to(
                    &Request::new(33, None).unwrap().encode(),
                    service.socket_path(),
                )
                .unwrap();
            assert!(response(&context, &legacy));
            service.stop();
            assert!(!service.is_running());
            assert!(!directory.path().exists());
        })
        .unwrap();
}

#[test]
fn invalid_lengths_opcodes_floats_and_unreplyable_senders_never_dispatch() {
    let directory = Directory::new();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let calls = Rc::new(Cell::new(0));
            let captured = calls.clone();
            let service = IpcService::bind(&directory.0, move |command| {
                captured.set(captured.get() + 1);
                success(command)
            })
            .unwrap();
            let peer = client();
            let mut malformed = vec![
                Vec::new(),
                vec![2],
                vec![2, 0, 0],
                vec![0; 4],
                vec![37, 0, 0, 0],
                vec![2, 0, 0, 0, 0],
                vec![4, 0, 0, 0],
                vec![0; 9],
                vec![0; 65536],
            ];
            let mut oversized = Request::new(4, Some(0.5)).unwrap().encode();
            oversized.extend([0; 24]);
            malformed.push(oversized);
            for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.1, 1.1] {
                let mut bytes = 4u32.to_le_bytes().to_vec();
                bytes.extend(value.to_le_bytes());
                malformed.push(bytes);
            }
            for bytes in malformed {
                peer.send_to(&bytes, service.socket_path()).unwrap();
                assert!(!response(&context, &peer));
            }
            let unnamed = UnixDatagram::unbound().unwrap();
            unnamed
                .send_to(
                    &Request::new(2, None).unwrap().encode(),
                    service.socket_path(),
                )
                .unwrap();
            context.block_on(glib::timeout_future(Duration::from_millis(20)));
            assert_eq!(calls.get(), 0);
            peer.send_to(
                &Request::new(1, None).unwrap().encode(),
                service.socket_path(),
            )
            .unwrap();
            assert!(response(&context, &peer));
            assert_eq!(calls.get(), 1);
        })
        .unwrap();
}

struct PendingDrop(Rc<Cell<usize>>);
impl Future for PendingDrop {
    type Output = Result<(), String>;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}
impl Drop for PendingDrop {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn deadlines_bounded_dispatch_and_shutdown_cancel_without_another_iteration() {
    let directory = Directory::new();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let drops = Rc::new(Cell::new(0));
            let captured = drops.clone();
            let starts = Rc::new(Cell::new(0));
            let count = starts.clone();
            let service = IpcService::bind(&directory.0, move |_| {
                count.set(count.get() + 1);
                Box::pin(PendingDrop(captured.clone()))
            })
            .unwrap();
            let peer = client();
            let start = Instant::now();
            peer.send_to(
                &Request::new(2, None).unwrap().encode(),
                service.socket_path(),
            )
            .unwrap();
            assert!(!response(&context, &peer));
            assert!(start.elapsed() >= Duration::from_millis(1900));
            assert_eq!(drops.get(), 1);
            let peers: Vec<_> = (0..65).map(|_| client()).collect();
            for peer in &peers {
                send(
                    &context,
                    peer,
                    &Request::new(2, None).unwrap().encode(),
                    service.socket_path(),
                );
                context.iteration(false);
            }
            wait(&context, || starts.get() == 65);
            assert!(!response(&context, &peers[64]));
            assert_eq!(socket_entries(&directory.path()), 1);
            service.stop();
            assert_eq!(
                drops.get(),
                65,
                "stop must cancel every native-independent dispatch immediately"
            );
            assert_eq!(socket_entries(&directory.path()), 0);
            assert!(!directory.path().exists());
            let weak = Rc::downgrade(&service);
            drop(service);
            assert!(weak.upgrade().is_none());
            // No event-loop iteration is needed to rebind after complete shutdown.
            let replacement = IpcService::bind(&directory.0, success).unwrap();
            drop(replacement);
            assert_eq!(socket_entries(&directory.path()), 0);
        })
        .unwrap();
}

#[test]
fn socket_ownership_live_stale_foreign_paths_and_replacements() {
    let directory = Directory::new();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let live = UnixDatagram::bind(directory.path()).unwrap();
            assert!(IpcService::bind(&directory.0, success).is_err());
            assert!(directory.path().exists());
            drop(live); // Crash leaves a stale filesystem address.
            let service = IpcService::bind(&directory.0, success).unwrap();
            assert!(IpcService::bind(&directory.0, success).is_err());
            let peer = client();
            peer.send_to(
                &Request::new(1, None).unwrap().encode(),
                service.socket_path(),
            )
            .unwrap();
            assert!(response(&context, &peer));
            fs::remove_file(directory.path()).unwrap();
            fs::write(directory.path(), b"unrelated replacement").unwrap();
            service.stop();
            assert_eq!(
                fs::read(directory.path()).unwrap(),
                b"unrelated replacement"
            );
            assert!(IpcService::bind(&directory.0, success).is_err());
            assert_eq!(
                fs::read(directory.path()).unwrap(),
                b"unrelated replacement"
            );
            fs::remove_file(directory.path()).unwrap();
            let first = IpcService::bind(&directory.0, success).unwrap();
            fs::remove_file(directory.path()).unwrap();
            let foreign = UnixDatagram::bind(directory.path()).unwrap();
            first.stop();
            assert!(directory.path().exists());
            assert!(IpcService::bind(&directory.0, success).is_err());
            drop(foreign);
            let final_server = IpcService::bind(&directory.0, success).unwrap();
            drop(final_server);
            assert!(!directory.path().exists());
            assert!(IpcService::bind(Path::new("relative"), success).is_err());
            assert!(IpcService::bind(Path::new("/tmp/invalid\0runtime"), success).is_err());
            assert!(IpcService::bind(Path::new(""), success).is_err());
            assert!(
                IpcService::bind(&PathBuf::from(format!("/{}", "x".repeat(108))), success).is_err()
            );
        })
        .unwrap();
}

#[test]
fn shutdown_inside_dispatch_and_direct_drop_cancel_every_owner() {
    let directory = Directory::new();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let slot = Rc::new(RefCell::new(std::rc::Weak::<IpcService>::new()));
            let captured = slot.clone();
            let service = IpcService::bind(&directory.0, move |_| {
                let owner = captured.borrow().clone();
                Box::pin(async move {
                    owner.upgrade().unwrap().stop();
                    Ok(())
                })
            })
            .unwrap();
            slot.replace(Rc::downgrade(&service));
            let peer = client();
            peer.send_to(
                &Request::new(2, None).unwrap().encode(),
                service.socket_path(),
            )
            .unwrap();
            wait(&context, || !service.is_running());
            assert_eq!(socket_entries(&directory.path()), 0);
            let weak = Rc::downgrade(&service);
            drop(service);
            assert!(weak.upgrade().is_none());
            let drops = Rc::new(Cell::new(0));
            let captured = drops.clone();
            let starts = Rc::new(Cell::new(0));
            let count = starts.clone();
            let service = IpcService::bind(&directory.0, move |_| {
                count.set(count.get() + 1);
                Box::pin(PendingDrop(captured.clone()))
            })
            .unwrap();
            peer.send_to(
                &Request::new(2, None).unwrap().encode(),
                service.socket_path(),
            )
            .unwrap();
            wait(&context, || starts.get() == 1);
            let weak = Rc::downgrade(&service);
            drop(service);
            assert!(weak.upgrade().is_none());
            assert_eq!(drops.get(), 1);
            assert_eq!(socket_entries(&directory.path()), 0);
            assert!(!directory.path().exists());
        })
        .unwrap();
}
