//! Integrate wayland-client's prepared reads with the existing GLib context.
use super::{
    ToplevelAction,
    model::{Output, Seat, Toplevel},
    protocol::{Change, State},
};
use gio::prelude::*;
use std::{
    cell::{Cell, RefCell},
    os::fd::AsFd,
    rc::Rc,
};
use wayland_client::{
    Connection, EventQueue,
    backend::{ReadEventsGuard, WaylandError},
};

struct Protocol {
    connection: Connection,
    queue: EventQueue<State>,
    state: State,
}
pub(super) struct Driver {
    protocol: RefCell<Protocol>,
    poll_socket: gio::Socket,
    context: glib::MainContext,
    guard: RefCell<Option<ReadEventsGuard>>,
    reader: RefCell<Option<glib::Source>>,
    writer: RefCell<Option<glib::Source>>,
    stopped: Cell<bool>,
    notify: Box<dyn Fn(Vec<Change>)>,
    failed: Box<dyn Fn(String)>,
}
fn would_block(error: &WaylandError) -> bool {
    matches!(error, WaylandError::Io(error) if error.kind() == std::io::ErrorKind::WouldBlock)
}
impl Driver {
    pub fn connect(
        apps: &str,
        titles: &str,
        notify: impl Fn(Vec<Change>) + 'static,
        failed: impl Fn(String) + 'static,
    ) -> Result<Rc<Self>, String> {
        let connection = Connection::connect_to_env()
            .map_err(|e| format!("Could not connect to Wayland display: {e}"))?;
        let queue = connection.new_event_queue();
        let qh = queue.handle();
        connection.display().get_registry(&qh, ());
        connection.display().sync(&qh, false);
        let socket = gio::Socket::from_fd(
            connection
                .as_fd()
                .try_clone_to_owned()
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        socket.set_blocking(false);
        socket.set_timeout(0);
        let mut state = State::default();
        state.ignored(apps, titles);
        let driver = Rc::new(Self {
            protocol: RefCell::new(Protocol {
                connection,
                queue,
                state,
            }),
            poll_socket: socket,
            context: glib::MainContext::ref_thread_default(),
            guard: RefCell::new(None),
            reader: RefCell::new(None),
            writer: RefCell::new(None),
            stopped: Cell::new(false),
            notify: Box::new(notify),
            failed: Box::new(failed),
        });
        driver.dispatch()?;
        driver.flush();
        let weak = Rc::downgrade(&driver);
        let source = gio::prelude::SocketExtManual::create_source(
            &driver.poll_socket,
            glib::IOCondition::IN | glib::IOCondition::ERR | glib::IOCondition::HUP,
            gio::Cancellable::NONE,
            Some("way-shell-wayland-read"),
            glib::Priority::DEFAULT,
            move |_, condition| {
                let Some(driver) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let result = driver
                    .guard
                    .borrow_mut()
                    .take()
                    .map_or(Ok(0), ReadEventsGuard::read);
                if let Err(error) = result
                    && !would_block(&error)
                {
                    driver.fail(format!("Wayland read failed: {error}"));
                    return glib::ControlFlow::Break;
                }
                if let Err(error) = driver.dispatch() {
                    driver.fail(error);
                    return glib::ControlFlow::Break;
                }
                driver.flush();
                if condition.intersects(glib::IOCondition::ERR | glib::IOCondition::HUP) {
                    driver.fail("Wayland compositor disconnected".into());
                    return glib::ControlFlow::Break;
                }
                if driver.stopped.get() {
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            },
        );
        source.attach(Some(&driver.context));
        driver.reader.replace(Some(source));
        Ok(driver)
    }
    // Prepare before polling. Callbacks run only after the RefCell borrow ends,
    // so signal handlers can safely send actions or drop their service owner.
    fn dispatch(&self) -> Result<(), String> {
        let changes = {
            let mut protocol = self.protocol.borrow_mut();
            let Protocol {
                connection,
                queue,
                state,
            } = &mut *protocol;
            loop {
                queue
                    .dispatch_pending(state)
                    .map_err(|e| format!("Wayland dispatch failed: {e}"))?;
                if let Some(guard) = connection.prepare_read() {
                    self.guard.replace(Some(guard));
                    break;
                }
            }
            std::mem::take(&mut state.changes)
        };
        (self.notify)(changes);
        Ok(())
    }
    fn changes(&self) {
        let changes = std::mem::take(&mut self.protocol.borrow_mut().state.changes);
        (self.notify)(changes);
    }
    fn fail(&self, error: String) {
        if self.stopped.replace(true) {
            return;
        }
        self.stop_sources();
        (self.failed)(error);
    }
    fn stop_sources(&self) {
        self.guard.borrow_mut().take();
        for cell in [&self.reader, &self.writer] {
            if let Some(source) = cell.borrow_mut().take() {
                source.destroy();
            }
        }
    }
    fn flush(self: &Rc<Self>) {
        if self.stopped.get() {
            return;
        }
        let result = self.protocol.borrow().connection.flush();
        match result {
            Ok(()) => {
                if let Some(source) = self.writer.borrow_mut().take() {
                    source.destroy();
                }
            }
            Err(error) if would_block(&error) => {
                if self.writer.borrow().is_some() {
                    return;
                }
                let weak = Rc::downgrade(self);
                let source = gio::prelude::SocketExtManual::create_source(
                    &self.poll_socket,
                    glib::IOCondition::OUT | glib::IOCondition::ERR | glib::IOCondition::HUP,
                    gio::Cancellable::NONE,
                    Some("way-shell-wayland-write"),
                    glib::Priority::DEFAULT,
                    move |_, condition| {
                        let Some(driver) = weak.upgrade() else {
                            return glib::ControlFlow::Break;
                        };
                        if condition.intersects(glib::IOCondition::ERR | glib::IOCondition::HUP) {
                            driver.fail("Wayland compositor disconnected while writing".into());
                            return glib::ControlFlow::Break;
                        }
                        driver.flush();
                        if driver.writer.borrow().is_some() {
                            glib::ControlFlow::Continue
                        } else {
                            glib::ControlFlow::Break
                        }
                    },
                );
                source.attach(Some(&self.context));
                self.writer.replace(Some(source));
            }
            Err(error) => self.fail(format!("Wayland flush failed: {error}")),
        }
    }
    pub fn outputs(&self) -> Vec<Output> {
        self.protocol.borrow().state.outputs()
    }
    pub fn seats(&self) -> Vec<Seat> {
        self.protocol.borrow().state.seats()
    }
    pub fn toplevels(&self) -> Vec<Toplevel> {
        self.protocol.borrow().state.toplevels()
    }
    pub fn has_foreign_toplevel(&self) -> bool {
        !self.stopped.get() && self.protocol.borrow().state.has_foreign_toplevel()
    }
    pub fn has_gamma(&self) -> bool {
        !self.stopped.get() && self.protocol.borrow().state.has_gamma()
    }
    pub fn gamma_available(&self) -> bool {
        !self.stopped.get() && self.protocol.borrow().state.gamma_available()
    }
    pub fn has_shortcut_inhibition(&self) -> bool {
        !self.stopped.get() && self.protocol.borrow().state.has_shortcut_inhibition()
    }
    pub fn gamma_enabled(&self) -> bool {
        !self.stopped.get() && self.protocol.borrow().state.gamma_enabled()
    }
    pub fn ignored(&self, apps: &str, titles: &str) {
        self.protocol.borrow_mut().state.ignored(apps, titles);
        self.changes();
    }
    pub fn action(self: &Rc<Self>, id: u64, action: ToplevelAction) -> Result<(), String> {
        if self.stopped.get() {
            return Err("Wayland connection is closed".into());
        }
        self.protocol.borrow().state.action(id, action)?;
        self.flush();
        Ok(())
    }
    pub fn set_temperature(self: &Rc<Self>, temperature: u32) -> Result<(), String> {
        if self.stopped.get() {
            return Err("Wayland connection is closed".into());
        }
        {
            let mut protocol = self.protocol.borrow_mut();
            let qh = protocol.queue.handle();
            protocol.state.set_temperature(temperature, &qh)?;
        }
        self.changes();
        self.flush();
        Ok(())
    }
    pub fn disable_gamma(self: &Rc<Self>) {
        self.protocol.borrow_mut().state.disable_gamma();
        self.changes();
        self.flush();
    }
}
impl Drop for Driver {
    fn drop(&mut self) {
        self.stopped.set(true);
        self.stop_sources();
        let protocol = self.protocol.get_mut();
        protocol.state.close();
        let _ = protocol.connection.flush();
        let _ = self.poll_socket.close();
    }
}
