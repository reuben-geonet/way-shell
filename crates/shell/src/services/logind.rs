//! Asynchronous login1 actions, current-session lookup and owned inhibitor FDs.
use crate::platform::bus_watch::Watcher;
use gio::prelude::*;
use glib::{
    subclass::prelude::*,
    variant::{Handle, ObjectPath},
};
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::HashMap,
    os::fd::OwnedFd,
    sync::OnceLock,
    time::Duration,
};
const NAME: &str = "org.freedesktop.login1";
const PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION: &str = "org.freedesktop.login1.Session";
const TIMEOUT: i32 = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum PowerAction {
    Reboot,
    PowerOff,
    Suspend,
    Hibernate,
    HybridSleep,
    SuspendThenHibernate,
}
impl PowerAction {
    pub const ALL: [Self; 6] = [
        Self::Reboot,
        Self::PowerOff,
        Self::Suspend,
        Self::Hibernate,
        Self::HybridSleep,
        Self::SuspendThenHibernate,
    ];
    fn method(self) -> &'static str {
        match self {
            Self::Reboot => "Reboot",
            Self::PowerOff => "PowerOff",
            Self::Suspend => "Suspend",
            Self::Hibernate => "Hibernate",
            Self::HybridSleep => "HybridSleep",
            Self::SuspendThenHibernate => "SuspendThenHibernate",
        }
    }
}
#[derive(Clone, Debug)]
pub struct SessionIdentity {
    pub uid: u32,
    pub pid: u32,
    pub session_id: Option<String>,
}
impl SessionIdentity {
    pub fn current() -> Result<Self, glib::Error> {
        Ok(Self {
            uid: gio::Credentials::new().unix_user()?,
            pid: std::process::id(),
            session_id: std::env::var("XDG_SESSION_ID")
                .ok()
                .filter(|id| !id.is_empty()),
        })
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoginState {
    pub available: bool,
    pub session: Option<String>,
    pub capabilities: [bool; 6],
    pub inhibited: bool,
}
mod imp {
    use super::*;
    #[derive(Default)]
    pub struct LogindService {
        pub state: RefCell<LoginState>,
        pub identity: OnceCell<SessionIdentity>,
        pub settings: RefCell<Option<(gio::Settings, glib::SignalHandlerId)>>,
        pub desired_inhibit: Cell<bool>,
        pub inhibitor: RefCell<Option<OwnedFd>>,
        pub inhibit_request: RefCell<Option<gio::Cancellable>>,
        pub inhibit_generation: Cell<u64>,
        pub owner: RefCell<Option<(gio::DBusConnection, String)>>,
        pub generation: Cell<u64>,
        pub(super) watcher: RefCell<Option<Watcher>>,
        pub subscription: RefCell<Option<gio::SignalSubscription>>,
        pub tasks: RefCell<Vec<glib::JoinHandle<()>>>,
        pub session_task: RefCell<Option<glib::JoinHandle<()>>>,
        pub retry: RefCell<Option<glib::JoinHandle<()>>>,
        pub session_path: RefCell<Option<String>>,
        pub operations: RefCell<HashMap<u64, gio::Cancellable>>,
        pub next_operation: Cell<u64>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for LogindService {
        const NAME: &'static str = "WayShellLogindService";
        type Type = super::LogindService;
    }
    impl ObjectImpl for LogindService {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("changed").build(),
                    glib::subclass::Signal::builder("idle-inhibitor-changed")
                        .param_types([bool::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("availability-changed")
                        .param_types([bool::static_type()])
                        .build(),
                ]
            })
        }
        fn dispose(&self) {
            self.obj().stop();
        }
    }
}
glib::wrapper! { pub struct LogindService(ObjectSubclass<imp::LogindService>); }
impl LogindService {
    pub fn new() -> Result<Self, glib::BoolError> {
        let settings = super::settings::open("org.ldelossa.way-shell.system")?;
        let identity = SessionIdentity::current()
            .map_err(|error| glib::bool_error!("Could not identify current user: {error}"))?;
        let service = Self::configured(settings, identity);
        let appeared = service.downgrade();
        let vanished = service.downgrade();
        let watcher = Watcher::on_bus(
            gio::BusType::System,
            NAME,
            move |connection, owner| {
                if let Some(service) = appeared.upgrade() {
                    service.appeared(connection, owner);
                }
            },
            move || {
                if let Some(service) = vanished.upgrade() {
                    service.reset();
                    glib::g_message!(
                        "way-shell",
                        "login1 unavailable; session controls wait for the service"
                    );
                }
            },
        );
        service.imp().watcher.replace(Some(watcher));
        Ok(service)
    }
    pub fn on_connection(
        connection: &gio::DBusConnection,
        settings: gio::Settings,
        identity: SessionIdentity,
    ) -> Self {
        let service = Self::configured(settings, identity);
        let appeared = service.downgrade();
        let vanished = service.downgrade();
        let watcher = Watcher::on_connection(
            connection,
            NAME,
            move |connection, owner| {
                if let Some(service) = appeared.upgrade() {
                    service.appeared(connection, owner);
                }
            },
            move || {
                if let Some(service) = vanished.upgrade() {
                    service.reset();
                }
            },
        );
        service.imp().watcher.replace(Some(watcher));
        service
    }
    fn configured(settings: gio::Settings, identity: SessionIdentity) -> Self {
        let service: Self = glib::Object::new();
        service.imp().identity.set(identity).unwrap();
        service
            .imp()
            .desired_inhibit
            .set(settings.boolean("idle-inhibitor"));
        let weak = service.downgrade();
        let handler = settings.connect_changed(Some("idle-inhibitor"), move |settings, _| {
            if let Some(service) = weak.upgrade() {
                service.set_idle_inhibit(settings.boolean("idle-inhibitor"));
            }
        });
        service.imp().settings.replace(Some((settings, handler)));
        service
    }
    pub fn stop(&self) {
        self.imp().watcher.borrow_mut().take();
        if let Some((settings, handler)) = self.imp().settings.borrow_mut().take() {
            settings.disconnect(handler);
        }
        self.reset();
    }
    pub fn state(&self) -> LoginState {
        self.imp().state.borrow().clone()
    }
    pub fn can(&self, action: PowerAction) -> bool {
        self.imp().state.borrow().capabilities[action as usize]
    }
    pub fn perform(
        &self,
        action: PowerAction,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) -> gio::Cancellable {
        if !self.can(action) {
            let cancel = gio::Cancellable::new();
            done(Err(error(
                gio::IOErrorEnum::PermissionDenied,
                "login1 does not permit this power action",
            )));
            return cancel;
        }
        self.operation(
            PATH,
            MANAGER,
            action.method(),
            Some(&(false,).to_variant()),
            done,
        )
    }
    pub fn terminate_session(
        &self,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) -> gio::Cancellable {
        let path = self.imp().session_path.borrow().clone();
        if let Some(path) = path {
            self.operation(&path, SESSION, "Terminate", None, done)
        } else {
            let cancel = gio::Cancellable::new();
            done(Err(error(
                gio::IOErrorEnum::NotConnected,
                "No current login1 session is available",
            )));
            cancel
        }
    }
    pub fn set_brightness(
        &self,
        subsystem: &str,
        name: &str,
        brightness: u32,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) -> gio::Cancellable {
        if !matches!(subsystem, "backlight" | "leds")
            || name.is_empty()
            || name.contains(['/', '\0'])
            || name == "."
            || name == ".."
        {
            let cancel = gio::Cancellable::new();
            done(Err(error(
                gio::IOErrorEnum::InvalidArgument,
                "Invalid brightness device",
            )));
            return cancel;
        }
        let path = self.imp().session_path.borrow().clone();
        if let Some(path) = path {
            self.operation(
                &path,
                SESSION,
                "SetBrightness",
                Some(&(subsystem, name, brightness).to_variant()),
                done,
            )
        } else {
            let cancel = gio::Cancellable::new();
            done(Err(error(
                gio::IOErrorEnum::NotConnected,
                "No current login1 session is available",
            )));
            cancel
        }
    }
    fn operation(
        &self,
        path: &str,
        interface: &str,
        method: &str,
        parameters: Option<&glib::Variant>,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) -> gio::Cancellable {
        let cancel = gio::Cancellable::new();
        let Some((connection, owner)) = self.imp().owner.borrow().clone() else {
            done(Err(error(
                gio::IOErrorEnum::NotConnected,
                "login1 unavailable",
            )));
            return cancel;
        };
        let generation = self.imp().generation.get();
        let operation = self.imp().next_operation.get();
        self.imp().next_operation.set(operation.wrapping_add(1));
        self.imp()
            .operations
            .borrow_mut()
            .insert(operation, cancel.clone());
        let weak = self.downgrade();
        connection.call(
            Some(&owner),
            path,
            interface,
            method,
            parameters,
            Some(glib::VariantTy::UNIT),
            gio::DBusCallFlags::NONE,
            TIMEOUT,
            Some(&cancel),
            move |result| {
                let current = weak.upgrade().is_some_and(|service| {
                    service.imp().operations.borrow_mut().remove(&operation);
                    service.imp().generation.get() == generation
                });
                if current {
                    done(result.map(|_| ()));
                } else {
                    done(Err(error(
                        gio::IOErrorEnum::Cancelled,
                        "login1 owner changed during the request",
                    )));
                }
            },
        );
        cancel
    }
    pub fn set_idle_inhibit(&self, enabled: bool) {
        self.imp().desired_inhibit.set(enabled);
        if !enabled {
            self.imp()
                .inhibit_generation
                .set(self.imp().inhibit_generation.get().wrapping_add(1));
            if let Some(cancel) = self.imp().inhibit_request.borrow_mut().take() {
                cancel.cancel();
            }
            self.imp().inhibitor.borrow_mut().take();
            self.update(|state| state.inhibited = false);
            return;
        }
        if self.imp().inhibitor.borrow().is_some() || self.imp().inhibit_request.borrow().is_some()
        {
            return;
        }
        let Some((connection, owner)) = self.imp().owner.borrow().clone() else {
            return;
        };
        let generation = self.imp().generation.get();
        let request = self.imp().inhibit_generation.get();
        let cancel = gio::Cancellable::new();
        self.imp().inhibit_request.replace(Some(cancel.clone()));
        let weak = self.downgrade();
        connection.call_with_unix_fd_list(
            Some(&owner),
            PATH,
            MANAGER,
            "Inhibit",
            Some(&("idle", "way-shell", "User initiated idle block", "block").to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            TIMEOUT,
            gio::UnixFDList::NONE,
            Some(&cancel),
            move |result| {
                let Some(service) = weak.upgrade().filter(|s| {
                    s.imp().generation.get() == generation
                        && s.imp().inhibit_generation.get() == request
                }) else {
                    return;
                };
                service.imp().inhibit_request.borrow_mut().take();
                let fd = result.and_then(|(reply, fds)| {
                    let (Handle(index),) = reply.get::<(Handle,)>().ok_or_else(|| {
                        error(gio::IOErrorEnum::InvalidData, "Invalid inhibitor reply")
                    })?;
                    let fds = fds.ok_or_else(|| {
                        error(
                            gio::IOErrorEnum::InvalidData,
                            "Missing inhibitor descriptor",
                        )
                    })?;
                    if !(0..fds.length()).contains(&index) {
                        return Err(error(
                            gio::IOErrorEnum::InvalidData,
                            "Invalid inhibitor descriptor index",
                        ));
                    }
                    fds.get(index)
                });
                match fd {
                    Ok(fd) if service.imp().desired_inhibit.get() => {
                        service.imp().inhibitor.replace(Some(fd));
                        service.update(|state| state.inhibited = true);
                    }
                    Ok(_) => (),
                    Err(error) => {
                        glib::g_message!("way-shell", "Could not acquire idle inhibitor: {error}")
                    }
                }
            },
        );
    }
    fn appeared(&self, connection: gio::DBusConnection, owner: &str) {
        self.reset();
        self.imp()
            .owner
            .replace(Some((connection.clone(), owner.into())));
        self.update(|state| state.available = true);
        let weak = self.downgrade();
        self.imp()
            .subscription
            .replace(Some(connection.subscribe_to_signal(
                Some(owner),
                Some(MANAGER),
                None,
                Some(PATH),
                None,
                gio::DBusSignalFlags::NONE,
                move |signal| {
                    if let Some(service) = weak.upgrade() {
                        match signal.signal_name {
                            "SessionNew" | "SessionRemoved" => service.refresh_session(),
                            "PrepareForSleep" | "PrepareForShutdown"
                                if signal.parameters.get::<(bool,)>() == Some((false,)) =>
                            {
                                service.refresh_capabilities()
                            }
                            _ => (),
                        }
                    }
                },
            )));
        self.refresh_session();
        self.refresh_capabilities();
        self.set_idle_inhibit(self.imp().desired_inhibit.get());
    }
    fn refresh_capabilities(&self) {
        for task in self.imp().tasks.borrow_mut().drain(..) {
            task.abort();
        }
        let Some((connection, owner)) = self.imp().owner.borrow().clone() else {
            return;
        };
        let generation = self.imp().generation.get();
        for action in PowerAction::ALL {
            let weak = self.downgrade();
            let connection = connection.clone();
            let owner = owner.clone();
            let task = glib::MainContext::ref_thread_default().spawn_local(async move {
                let reply = connection
                    .call_future(
                        Some(&owner),
                        PATH,
                        MANAGER,
                        &format!("Can{}", action.method()),
                        None,
                        None,
                        gio::DBusCallFlags::NONE,
                        TIMEOUT,
                    )
                    .await;
                if let Some(service) = weak
                    .upgrade()
                    .filter(|s| s.imp().generation.get() == generation)
                {
                    let allowed = reply
                        .ok()
                        .and_then(|reply| reply.get::<(String,)>())
                        .is_some_and(|(value,)| value == "yes");
                    service.update(|state| state.capabilities[action as usize] = allowed);
                }
            });
            self.imp().tasks.borrow_mut().push(task);
        }
    }
    fn refresh_session(&self) {
        if let Some(task) = self.imp().session_task.borrow_mut().take() {
            task.abort();
        }
        if let Some(retry) = self.imp().retry.borrow_mut().take() {
            retry.abort();
        }
        self.imp().session_path.borrow_mut().take();
        self.update(|state| state.session = None);
        let Some((connection, owner)) = self.imp().owner.borrow().clone() else {
            return;
        };
        let identity = self.imp().identity.get().unwrap().clone();
        let generation = self.imp().generation.get();
        let weak = self.downgrade();
        let task = glib::MainContext::ref_thread_default().spawn_local(async move {
            let result = resolve_session(&connection, &owner, &identity).await;
            let Some(service) = weak
                .upgrade()
                .filter(|s| s.imp().generation.get() == generation)
            else {
                return;
            };
            service.imp().session_task.borrow_mut().take();
            match result {
                Ok((path, id)) => {
                    service.imp().session_path.replace(Some(path));
                    service.update(|state| state.session = Some(id));
                }
                Err(error) => {
                    glib::g_message!(
                        "way-shell",
                        "Current login1 session unavailable: {error}; retrying"
                    );
                    let weak = service.downgrade();
                    let retry = glib::MainContext::ref_thread_default().spawn_local(async move {
                        glib::timeout_future(Duration::from_secs(1)).await;
                        if let Some(service) = weak.upgrade() {
                            service.imp().retry.borrow_mut().take();
                            service.refresh_session();
                        }
                    });
                    service.imp().retry.replace(Some(retry));
                }
            }
        });
        self.imp().session_task.replace(Some(task));
    }
    fn update(&self, mutate: impl FnOnce(&mut LoginState)) {
        let previous = self.state();
        mutate(&mut self.imp().state.borrow_mut());
        let state = self.state();
        if previous != state {
            self.emit_by_name::<()>("changed", &[]);
        }
        if previous.inhibited != state.inhibited {
            self.emit_by_name::<()>("idle-inhibitor-changed", &[&state.inhibited]);
        }
        if previous.available != state.available {
            self.emit_by_name::<()>("availability-changed", &[&state.available]);
        }
    }
    fn reset(&self) {
        self.imp()
            .generation
            .set(self.imp().generation.get().wrapping_add(1));
        self.imp().owner.borrow_mut().take();
        self.imp().subscription.borrow_mut().take();
        for task in self.imp().tasks.borrow_mut().drain(..) {
            task.abort();
        }
        if let Some(task) = self.imp().session_task.borrow_mut().take() {
            task.abort();
        }
        if let Some(retry) = self.imp().retry.borrow_mut().take() {
            retry.abort();
        }
        for (_, cancel) in self.imp().operations.borrow_mut().drain() {
            cancel.cancel();
        }
        if let Some(cancel) = self.imp().inhibit_request.borrow_mut().take() {
            cancel.cancel();
        }
        self.imp().inhibitor.borrow_mut().take();
        self.imp().session_path.borrow_mut().take();
        self.update(|state| *state = LoginState::default());
    }
}
fn error(kind: gio::IOErrorEnum, message: &str) -> glib::Error {
    glib::Error::new(kind, message)
}
async fn session_properties(
    connection: &gio::DBusConnection,
    owner: &str,
    path: &str,
    uid: u32,
) -> Result<(String, bool), glib::Error> {
    let reply = connection
        .call_future(
            Some(owner),
            path,
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some(&(SESSION,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            TIMEOUT,
        )
        .await?;
    let (properties,) = reply
        .get::<(HashMap<String, glib::Variant>,)>()
        .ok_or_else(|| error(gio::IOErrorEnum::InvalidData, "Invalid session properties"))?;
    let user = properties
        .get("User")
        .and_then(|value| value.get::<(u32, ObjectPath)>());
    if !user.is_some_and(|(owner, _)| owner == uid) {
        return Err(error(
            gio::IOErrorEnum::PermissionDenied,
            "login1 session belongs to a different user",
        ));
    }
    let id = properties
        .get("Id")
        .and_then(|value| value.get::<String>())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| error(gio::IOErrorEnum::InvalidData, "Missing session identifier"))?;
    let active = properties
        .get("State")
        .and_then(|value| value.get::<String>())
        .is_some_and(|state| state == "active");
    Ok((id, active))
}
async fn resolve_session(
    connection: &gio::DBusConnection,
    owner: &str,
    identity: &SessionIdentity,
) -> Result<(String, String), glib::Error> {
    let mut queries = vec![("GetSessionByPID", (identity.pid,).to_variant())];
    if let Some(id) = &identity.session_id {
        queries.push(("GetSession", (id,).to_variant()));
    }
    for (method, parameters) in queries {
        if let Ok(reply) = connection
            .call_future(
                Some(owner),
                PATH,
                MANAGER,
                method,
                Some(&parameters),
                None,
                gio::DBusCallFlags::NONE,
                TIMEOUT,
            )
            .await
            && let Some((path,)) = reply.get::<(ObjectPath,)>()
            && let Ok((id, _)) =
                session_properties(connection, owner, path.as_str(), identity.uid).await
        {
            return Ok((path.to_string(), id));
        }
    }
    let reply = connection
        .call_future(
            Some(owner),
            PATH,
            MANAGER,
            "ListSessions",
            None,
            None,
            gio::DBusCallFlags::NONE,
            TIMEOUT,
        )
        .await?;
    type SessionRow = (String, u32, String, String, ObjectPath);
    let (sessions,) = reply
        .get::<(Vec<SessionRow>,)>()
        .ok_or_else(|| error(gio::IOErrorEnum::InvalidData, "Invalid session inventory"))?;
    let mut own = None;
    for (_, uid, _, seat, path) in sessions {
        if uid != identity.uid || seat.is_empty() {
            continue;
        }
        if let Ok((id, true)) = session_properties(connection, owner, path.as_str(), uid).await {
            if own.is_some() {
                return Err(error(
                    gio::IOErrorEnum::Failed,
                    "Several active sessions match this user; set XDG_SESSION_ID",
                ));
            }
            own = Some((path.to_string(), id));
        }
    }
    own.ok_or_else(|| error(gio::IOErrorEnum::NotFound, "No current session found"))
}
