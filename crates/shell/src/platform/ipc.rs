//! Unix stream I/O on the application's GLib context, with bounded backpressure.
use gio::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    os::unix::ffi::OsStrExt,
    path::Path,
    rc::Rc,
};

const MAX_QUEUED: usize = 4 * 1024 * 1024;
type Receiver = Box<dyn FnMut(&[u8]) -> Result<(), String>>;
type Closed = Box<dyn FnOnce(String)>;

#[derive(Clone)]
pub struct Connection(Rc<Inner>);

struct Inner {
    context: glib::MainContext,
    cancellable: gio::Cancellable,
    connection: RefCell<Option<gio::SocketConnection>>,
    reader: RefCell<Option<glib::Source>>,
    writer: RefCell<Option<glib::Source>>,
    pending: RefCell<VecDeque<Vec<u8>>>,
    offset: Cell<usize>,
    queued: Cell<usize>,
    closed: Cell<bool>,
    receive: RefCell<Receiver>,
    on_closed: RefCell<Option<Closed>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Connection {
    pub fn connect(
        path: &Path,
        receive: impl FnMut(&[u8]) -> Result<(), String> + 'static,
        on_closed: impl FnOnce(String) + 'static,
    ) -> Result<Self, String> {
        let bytes = path.as_os_str().as_bytes();
        if bytes.is_empty() || bytes.len() > 107 || bytes.contains(&0) {
            return Err("Invalid Unix socket path (must fit in 107 bytes with no NUL)".into());
        }
        let inner = Rc::new(Inner {
            context: glib::MainContext::ref_thread_default(),
            cancellable: gio::Cancellable::new(),
            connection: RefCell::new(None),
            reader: RefCell::new(None),
            writer: RefCell::new(None),
            pending: RefCell::new(VecDeque::new()),
            offset: Cell::new(0),
            queued: Cell::new(0),
            closed: Cell::new(false),
            receive: RefCell::new(Box::new(receive)),
            on_closed: RefCell::new(Some(Box::new(on_closed))),
        });
        let client = gio::SocketClient::new();
        client.set_timeout(2);
        let weak = Rc::downgrade(&inner);
        client.connect_async(
            &gio::UnixSocketAddress::new(path),
            Some(&inner.cancellable),
            move |result| {
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                if inner.closed.get() {
                    return;
                }
                match result {
                    Ok(connection) => inner.connected(connection),
                    Err(error) => {
                        inner.fail(format!("Could not connect to compositor socket: {error}"))
                    }
                }
            },
        );
        Ok(Self(inner))
    }

    pub fn send(&self, bytes: Vec<u8>) -> Result<(), String> {
        if self.0.closed.get() {
            return Err("Compositor connection is closed".into());
        }
        if bytes.is_empty() {
            return Ok(());
        }
        if bytes.len() > MAX_QUEUED - self.0.queued.get() {
            return Err("Compositor command queue is full".into());
        }
        self.0.queued.set(self.0.queued.get() + bytes.len());
        self.0.pending.borrow_mut().push_back(bytes);
        self.0.flush();
        if self.0.closed.get() {
            Err("Could not write to compositor socket".into())
        } else {
            Ok(())
        }
    }

    pub fn close(&self) {
        self.0.stop();
    }
}

impl Inner {
    fn stop(&self) {
        self.closed.set(true);
        self.cancellable.cancel();
        for source in [&self.reader, &self.writer] {
            if let Some(source) = source.borrow_mut().take() {
                source.destroy();
            }
        }
        if let Some(connection) = self.connection.borrow_mut().take() {
            let _ = connection.socket().close();
        }
        self.pending.borrow_mut().clear();
        self.queued.set(0);
        self.offset.set(0);
    }

    fn fail(&self, error: String) {
        if self.closed.replace(true) {
            return;
        }
        self.stop();
        let callback = self.on_closed.borrow_mut().take();
        if let Some(callback) = callback {
            callback(error);
        }
    }

    fn connected(self: &Rc<Self>, connection: gio::SocketConnection) {
        let socket = connection.socket();
        // SocketClient's deadline is only for establishing the connection.
        // An established compositor stream may legitimately remain idle.
        socket.set_timeout(0);
        socket.set_blocking(false);
        self.connection.replace(Some(connection));
        let weak = Rc::downgrade(self);
        let source = gio::prelude::SocketExtManual::create_source(
            &socket,
            glib::IOCondition::IN | glib::IOCondition::HUP | glib::IOCondition::ERR,
            gio::Cancellable::NONE,
            Some("way-shell-compositor-read"),
            glib::Priority::DEFAULT,
            move |socket, condition| {
                let Some(inner) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let mut bytes = [0u8; 8192];
                // Yield to GTK even when a compositor supplies a large event burst.
                for _ in 0..16 {
                    match socket.receive(&mut bytes, gio::Cancellable::NONE) {
                        Ok(0) => {
                            inner.fail("Compositor closed its IPC connection".into());
                            return glib::ControlFlow::Break;
                        }
                        Ok(length) => {
                            let result = (inner.receive.borrow_mut())(&bytes[..length]);
                            if let Err(error) = result {
                                inner.fail(error);
                                return glib::ControlFlow::Break;
                            }
                            if inner.closed.get() {
                                return glib::ControlFlow::Break;
                            }
                        }
                        Err(error) if error.matches(gio::IOErrorEnum::WouldBlock) => {
                            if condition.intersects(glib::IOCondition::HUP | glib::IOCondition::ERR)
                            {
                                inner.fail("Compositor IPC connection was lost".into());
                                return glib::ControlFlow::Break;
                            }
                            break;
                        }
                        Err(error) => {
                            inner.fail(format!("Could not read compositor IPC: {error}"));
                            return glib::ControlFlow::Break;
                        }
                    }
                }
                glib::ControlFlow::Continue
            },
        );
        source.attach(Some(&self.context));
        self.reader.replace(Some(source));
        self.flush();
    }

    fn flush(self: &Rc<Self>) {
        let Some(socket) = self
            .connection
            .borrow()
            .as_ref()
            .map(|connection| connection.socket())
        else {
            return;
        };
        let mut budget: usize = 128 * 1024;
        while self.queued.get() > 0 && budget > 0 {
            let result = {
                let pending = self.pending.borrow();
                socket.send(
                    &pending.front().unwrap()[self.offset.get()..],
                    gio::Cancellable::NONE,
                )
            };
            match result {
                Ok(0) => {
                    self.fail("Compositor IPC write made no progress".into());
                    return;
                }
                Ok(length) => {
                    self.offset.set(self.offset.get() + length);
                    self.queued.set(self.queued.get() - length);
                    budget = budget.saturating_sub(length);
                    let finished =
                        self.offset.get() == self.pending.borrow().front().unwrap().len();
                    if finished {
                        self.pending.borrow_mut().pop_front();
                        self.offset.set(0);
                    }
                }
                Err(error) if error.matches(gio::IOErrorEnum::WouldBlock) => break,
                Err(error) => {
                    self.fail(format!("Could not write compositor IPC: {error}"));
                    return;
                }
            }
        }
        if self.queued.get() == 0 {
            if let Some(source) = self.writer.borrow_mut().take() {
                source.destroy();
            }
        } else if self.writer.borrow().is_none() {
            let weak = Rc::downgrade(self);
            let source = gio::prelude::SocketExtManual::create_source(
                &socket,
                glib::IOCondition::OUT | glib::IOCondition::HUP | glib::IOCondition::ERR,
                gio::Cancellable::NONE,
                Some("way-shell-compositor-write"),
                glib::Priority::DEFAULT,
                move |_, condition| {
                    let Some(inner) = weak.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    if condition.intersects(glib::IOCondition::HUP | glib::IOCondition::ERR) {
                        inner.fail("Compositor IPC connection was lost while writing".into());
                        return glib::ControlFlow::Break;
                    }
                    inner.flush();
                    if inner.closed.get() || inner.queued.get() == 0 {
                        glib::ControlFlow::Break
                    } else {
                        glib::ControlFlow::Continue
                    }
                },
            );
            source.attach(Some(&self.context));
            self.writer.replace(Some(source));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        os::unix::net::UnixListener,
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, Instant},
    };

    #[test]
    fn backpressure_fragmented_reads_eof_and_cancellation() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "way-shell-transport.{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("socket");
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let expected: Vec<_> = (0..1_000_000).map(|i| (i % 251) as u8).collect();
        let sent = expected.clone();
        let (continue_send, continue_receive) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut peer = loop {
                match listener.accept() {
                    Ok((peer, _)) => break peer,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "client did not connect");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            std::thread::sleep(Duration::from_millis(20));
            let mut received = vec![0; expected.len()];
            peer.read_exact(&mut received).unwrap();
            assert_eq!(received, expected);
            continue_receive
                .recv_timeout(Duration::from_secs(6))
                .unwrap();
            for part in [b"ab", b"cd"] {
                peer.write_all(part).unwrap();
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let received = Rc::new(RefCell::new(Vec::new()));
                let sink = received.clone();
                let closed = Rc::new(Cell::new(0));
                let counter = closed.clone();
                let connection = Connection::connect(
                    &path,
                    move |bytes| {
                        sink.borrow_mut().extend_from_slice(bytes);
                        Ok(())
                    },
                    move |_| counter.set(counter.get() + 1),
                )
                .unwrap();
                connection.send(sent).unwrap();
                assert!(connection.send(vec![0; MAX_QUEUED + 1]).is_err());
                context.block_on(glib::timeout_future(Duration::from_millis(2100)));
                assert_eq!(
                    closed.get(),
                    0,
                    "an idle established connection must not inherit the connect deadline"
                );
                continue_send.send(()).unwrap();
                let deadline = Instant::now() + Duration::from_secs(4);
                while closed.get() == 0 {
                    assert!(Instant::now() < deadline, "connection did not finish");
                    context.block_on(glib::timeout_future(Duration::from_millis(10)));
                }
                assert_eq!(*received.borrow(), b"abcd");
                assert_eq!(closed.get(), 1);
                assert!(connection.send(vec![1]).is_err());
                let weak = Rc::downgrade(&connection.0);
                drop(connection);
                assert!(weak.upgrade().is_none());
                for _ in 0..20 {
                    let connection = Connection::connect(
                        &path,
                        |_| Ok(()),
                        |_| panic!("canceled owner was called"),
                    )
                    .unwrap();
                    let weak = Rc::downgrade(&connection.0);
                    drop(connection);
                    assert!(weak.upgrade().is_none());
                }
                context.block_on(glib::timeout_future(Duration::from_millis(20)));
            })
            .unwrap();
        server.join().unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
