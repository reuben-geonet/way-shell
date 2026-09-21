//! The stable `way-sh` datagram protocol, dispatched asynchronously on GLib.
use gio::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fs::{self, File, OpenOptions, TryLockError},
    future::Future,
    io,
    os::{
        linux::net::SocketAddrExt,
        unix::{
            ffi::OsStrExt,
            fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
            io::AsFd,
            net::{SocketAddr, UnixDatagram},
        },
    },
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
    time::Duration,
};
use way_shell_core::{
    IPC_SOCKET_NAME,
    ipc::{Request, encode_response},
};

const MAX_IN_FLIGHT: usize = 64;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
/// An application action completes only once its asynchronous operation has replied.
pub type DispatchFuture = Pin<Box<dyn Future<Output = Result<(), String>> + 'static>>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    MessageTrayOpen,
    VolumeUp,
    VolumeDown,
    VolumeSet(f32),
    VolumeMute,
    BrightnessUp,
    BrightnessDown,
    ThemeDark,
    ThemeLight,
    DumpDarkTheme,
    DumpLightTheme,
    ActivitiesShow,
    ActivitiesHide,
    ActivitiesToggle,
    AppSwitcherShow,
    AppSwitcherHide,
    AppSwitcherToggle,
    WorkspaceSwitcherShow,
    WorkspaceSwitcherHide,
    WorkspaceSwitcherToggle,
    OutputSwitcherShow,
    OutputSwitcherHide,
    OutputSwitcherToggle,
    WorkspaceAppSwitcherShow,
    WorkspaceAppSwitcherHide,
    WorkspaceAppSwitcherToggle,
    BlueLightFilterEnable,
    BlueLightFilterDisable,
    KeyboardBrightnessUp,
    KeyboardBrightnessDown,
    RenameSwitcherShow,
    RenameSwitcherHide,
    RenameSwitcherToggle,
}
impl Command {
    fn decode(bytes: &[u8]) -> Option<Self> {
        let request = Request::decode(bytes).ok()?;
        Some(match request.opcode() {
            1 => Self::MessageTrayOpen,
            2 => Self::VolumeUp,
            3 => Self::VolumeDown,
            4 => Self::VolumeSet(request.volume()?),
            5 => Self::VolumeMute,
            6 => Self::BrightnessUp,
            7 => Self::BrightnessDown,
            8 => Self::ThemeDark,
            9 => Self::ThemeLight,
            10 => Self::DumpDarkTheme,
            11 => Self::DumpLightTheme,
            12 => Self::ActivitiesShow,
            13 => Self::ActivitiesHide,
            14 => Self::ActivitiesToggle,
            15 => Self::AppSwitcherShow,
            16 => Self::AppSwitcherHide,
            17 => Self::AppSwitcherToggle,
            18 => Self::WorkspaceSwitcherShow,
            19 => Self::WorkspaceSwitcherHide,
            20 => Self::WorkspaceSwitcherToggle,
            21 => Self::OutputSwitcherShow,
            22 => Self::OutputSwitcherHide,
            23 => Self::OutputSwitcherToggle,
            24 => Self::WorkspaceAppSwitcherShow,
            25 => Self::WorkspaceAppSwitcherHide,
            26 => Self::WorkspaceAppSwitcherToggle,
            27 => Self::BlueLightFilterEnable,
            28 => Self::BlueLightFilterDisable,
            29 => Self::KeyboardBrightnessUp,
            30 => Self::KeyboardBrightnessDown,
            31 => Self::RenameSwitcherShow,
            32 => Self::RenameSwitcherHide,
            33 => Self::RenameSwitcherToggle,
            // Opcode zero historically decoded but had no successful operation.
            _ => return None,
        })
    }
}

struct Binding {
    socket: UnixDatagram,
    readiness: Option<gio::Socket>,
    path: PathBuf,
    identity: (u64, u64),
    // Release the persistent advisory lock only after the socket descriptors.
    _lock: File,
}
impl Binding {
    fn bind(runtime: &Path) -> io::Result<Self> {
        let path = runtime.join(IPC_SOCKET_NAME);
        let bytes = path.as_os_str().as_bytes();
        if !runtime.is_absolute() || bytes.len() >= 108 || bytes.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "XDG_RUNTIME_DIR must yield an absolute socket path of at most 107 bytes with no NUL",
            ));
        }
        // Never unlink this file: its stable inode serializes concurrent binders.
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(runtime.join(format!(".{IPC_SOCKET_NAME}.lock")))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "Another Way Shell IPC server owns this runtime directory",
                ));
            }
            Err(TryLockError::Error(error)) => return Err(error),
        }
        let socket = match UnixDatagram::bind(&path) {
            Ok(socket) => socket,
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                let previous = fs::symlink_metadata(&path)?;
                if !previous.file_type().is_socket() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "Way Shell IPC path exists and is not a socket",
                    ));
                }
                let probe = UnixDatagram::unbound()?;
                probe.set_nonblocking(true)?;
                match probe.connect(&path) {
                    Ok(()) => {
                        return Err(io::Error::new(
                            io::ErrorKind::AddrInUse,
                            "A live server already owns the Way Shell IPC socket",
                        ));
                    }
                    Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {}
                    Err(error) => return Err(error),
                }
                let current = fs::symlink_metadata(&path)?;
                if identity(&previous) != identity(&current) || !current.file_type().is_socket() {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "Way Shell IPC socket changed while checking its owner",
                    ));
                }
                fs::remove_file(&path)?;
                UnixDatagram::bind(&path)?
            }
            Err(error) => return Err(error),
        };
        let metadata = fs::symlink_metadata(&path)?;
        let mut binding = Self {
            socket,
            readiness: None,
            path,
            identity: identity(&metadata),
            _lock: lock,
        };
        binding.socket.set_nonblocking(true)?;
        let readiness = gio::Socket::from_fd(binding.socket.as_fd().try_clone_to_owned()?)
            .map_err(io::Error::other)?;
        readiness.set_blocking(false);
        binding.readiness = Some(readiness);
        Ok(binding)
    }
}
impl Drop for Binding {
    fn drop(&mut self) {
        if let Some(socket) = self.readiness.take() {
            let _ = socket.close();
        }
        if fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.file_type().is_socket() && identity(&metadata) == self.identity
        }) {
            let _ = fs::remove_file(&self.path);
        }
    }
}
fn identity(metadata: &fs::Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

pub struct IpcService {
    context: glib::MainContext,
    path: PathBuf,
    binding: RefCell<Option<Binding>>,
    source: RefCell<Option<glib::Source>>,
    dispatch: Box<dyn Fn(Command) -> DispatchFuture>,
    tasks: RefCell<HashMap<u64, glib::JoinHandle<()>>>,
    next: Cell<u64>,
    running: Cell<bool>,
}
impl IpcService {
    pub fn new(dispatch: impl Fn(Command) -> DispatchFuture + 'static) -> io::Result<Rc<Self>> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
        Self::bind(Path::new(&runtime), dispatch)
    }
    pub fn bind(
        runtime: &Path,
        dispatch: impl Fn(Command) -> DispatchFuture + 'static,
    ) -> io::Result<Rc<Self>> {
        let binding = Binding::bind(runtime)?;
        let readiness = binding.readiness.as_ref().unwrap().clone();
        let this = Rc::new(Self {
            context: glib::MainContext::ref_thread_default(),
            path: binding.path.clone(),
            binding: RefCell::new(Some(binding)),
            source: RefCell::new(None),
            dispatch: Box::new(dispatch),
            tasks: RefCell::new(HashMap::new()),
            next: Cell::new(0),
            running: Cell::new(true),
        });
        let weak = Rc::downgrade(&this);
        let source = gio::prelude::SocketExtManual::create_source(
            &readiness,
            glib::IOCondition::IN | glib::IOCondition::HUP | glib::IOCondition::ERR,
            gio::Cancellable::NONE,
            Some("way-shell-command-server"),
            glib::Priority::DEFAULT,
            move |_, condition| {
                let Some(this) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if condition.intersects(glib::IOCondition::HUP | glib::IOCondition::ERR) {
                    glib::g_message!("way-shell", "Way Shell IPC socket became unavailable");
                    this.stop();
                } else {
                    this.receive();
                }
                if this.is_running() {
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                }
            },
        );
        source.attach(Some(&this.context));
        this.source.replace(Some(source));
        Ok(this)
    }
    pub fn socket_path(&self) -> &Path {
        &self.path
    }
    pub fn is_running(&self) -> bool {
        self.running.get()
    }
    pub fn stop(&self) {
        if !self.running.replace(false) {
            return;
        }
        let source = self.source.take();
        if let Some(source) = source {
            source.destroy();
        }
        let tasks = self.tasks.take();
        for (_, task) in tasks {
            task.abort();
        }
        self.binding.take();
    }
    fn receive(self: &Rc<Self>) {
        // One extra byte rejects truncated oversized datagrams without native recvmsg.
        let mut bytes = [0; 9];
        for _ in 0..32 {
            let received = {
                let binding = self.binding.borrow();
                let Some(binding) = binding.as_ref() else {
                    break;
                };
                binding.socket.recv_from(&mut bytes)
            };
            match received {
                Ok((length, address)) => {
                    // An unnamed sender cannot receive an acknowledgment.
                    if address.as_pathname().is_none() && address.as_abstract_name().is_none() {
                        continue;
                    }
                    if let Some(command) = Command::decode(&bytes[..length]) {
                        if self.tasks.borrow().len() < MAX_IN_FLIGHT {
                            self.start(command, address);
                        } else {
                            self.respond(&address, false);
                        }
                    } else {
                        self.respond(&address, false);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    glib::g_message!("way-shell", "Could not read Way Shell IPC: {error}");
                    self.stop();
                    break;
                }
            }
            if !self.running.get() {
                break;
            }
        }
    }
    fn start(self: &Rc<Self>, command: Command, address: SocketAddr) {
        let mut id = self.next.get();
        while self.tasks.borrow().contains_key(&id) {
            id = id.wrapping_add(1);
        }
        self.next.set(id.wrapping_add(1));
        let weak = Rc::downgrade(self);
        let task = self.context.spawn_local(async move {
            let future = {
                let Some(this) = weak.upgrade().filter(|this| this.running.get()) else {
                    return;
                };
                let future = (this.dispatch)(command);
                if !this.running.get() {
                    return;
                }
                future
            };
            let result = glib::future_with_timeout(COMMAND_TIMEOUT, future).await;
            if let Some(this) = weak.upgrade().filter(|this| this.running.get()) {
                this.tasks.borrow_mut().remove(&id);
                let success = match result {
                    Ok(Ok(())) => true,
                    Ok(Err(error)) => {
                        glib::g_message!("way-shell", "IPC {command:?} failed: {error}");
                        false
                    }
                    Err(_) => {
                        glib::g_message!(
                            "way-shell",
                            "IPC {command:?} exceeded its response deadline"
                        );
                        false
                    }
                };
                this.respond(&address, success);
            }
        });
        self.tasks.borrow_mut().insert(id, task);
    }
    fn respond(&self, address: &SocketAddr, success: bool) {
        let binding = self.binding.borrow();
        if let Some(binding) = binding.as_ref()
            && let Err(error) = binding
                .socket
                .send_to_addr(&encode_response(success), address)
        {
            glib::g_debug!("way-shell", "Could not deliver IPC acknowledgment: {error}");
        }
    }
}
impl Drop for IpcService {
    fn drop(&mut self) {
        self.stop();
    }
}
