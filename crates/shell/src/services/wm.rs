use crate::platform::ipc::Connection;
use gio::prelude::*;
use glib::subclass::prelude::*;
use std::{
    cell::{Cell, OnceCell, RefCell},
    os::unix::fs::FileTypeExt,
    path::PathBuf,
    sync::OnceLock,
    time::Duration,
};
use way_shell_core::{
    sway,
    wm::{Action, Output, Workspace},
};

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct WindowManager {
        pub path: OnceCell<PathBuf>,
        pub config: OnceCell<PathBuf>,
        pub settings: RefCell<Option<(gio::Settings, glib::SignalHandlerId)>>,
        pub connection: RefCell<Option<Connection>>,
        pub retry: RefCell<Option<glib::JoinHandle<()>>>,
        pub decoder: RefCell<sway::Decoder>,
        pub workspaces: RefCell<Vec<Workspace>>,
        pub outputs: RefCell<Vec<Output>>,
        pub ready: Cell<bool>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for WindowManager {
        const NAME: &'static str = "WayShellWindowManager";
        type Type = super::WindowManager;
    }
    impl ObjectImpl for WindowManager {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("workspaces-changed").build(),
                    glib::subclass::Signal::builder("outputs-changed").build(),
                    glib::subclass::Signal::builder("connection-changed")
                        .param_types([bool::static_type()])
                        .build(),
                ]
            })
        }
        fn dispose(&self) {
            if let Some((settings, handler)) = self.settings.borrow_mut().take() {
                settings.disconnect(handler);
            }
            if let Some(retry) = self.retry.borrow_mut().take() {
                retry.abort();
            }
            if let Some(connection) = self.connection.borrow_mut().take() {
                connection.close();
            }
        }
    }
}

glib::wrapper! { pub struct WindowManager(ObjectSubclass<imp::WindowManager>); }

impl WindowManager {
    pub fn sway() -> Result<Self, String> {
        Self::sway_with_settings(
            super::settings::open("org.ldelossa.way-shell.window-manager")
                .map_err(|e| e.to_string())?,
            find_sway_socket()?,
            glib::user_config_dir().join("way-shell"),
        )
    }

    pub fn sway_with_settings(
        settings: gio::Settings,
        path: PathBuf,
        config: PathBuf,
    ) -> Result<Self, String> {
        let service: Self = glib::Object::new();
        service.imp().path.set(path).unwrap();
        service.imp().config.set(config).unwrap();
        let weak = service.downgrade();
        let handler =
            settings.connect_changed(Some("sort-workspaces-alphabetical"), move |_, _| {
                if let Some(service) = weak.upgrade()
                    && let Err(error) = service.request(sway::WORKSPACES, b"")
                {
                    glib::g_warning!("way-shell", "Could not refresh Sway workspaces: {error}");
                }
            });
        service.imp().settings.replace(Some((settings, handler)));
        service.connect()?;
        Ok(service)
    }

    pub fn workspaces(&self) -> Vec<Workspace> {
        self.imp().workspaces.borrow().clone()
    }
    pub fn outputs(&self) -> Vec<Output> {
        self.imp().outputs.borrow().clone()
    }
    pub fn is_connected(&self) -> bool {
        self.imp().ready.get()
    }

    pub fn perform(&self, action: &Action) -> Result<(), String> {
        self.request(sway::COMMAND, sway::command(action)?.as_bytes())
    }

    fn request(&self, kind: u32, payload: &[u8]) -> Result<(), String> {
        let connection = self
            .imp()
            .connection
            .borrow()
            .clone()
            .ok_or("Compositor is disconnected")?;
        connection.send(sway::encode(kind, payload)?)
    }

    fn connect(&self) -> Result<(), String> {
        self.imp().decoder.replace(sway::Decoder::default());
        let weak = self.downgrade();
        let lost = self.downgrade();
        let connection = Connection::connect(
            self.imp().path.get().unwrap(),
            move |bytes| {
                if let Some(service) = weak.upgrade() {
                    let frames = service.imp().decoder.borrow_mut().push(bytes)?;
                    for frame in frames {
                        service.handle(frame)?;
                    }
                }
                Ok(())
            },
            move |error| {
                if let Some(service) = lost.upgrade() {
                    service.disconnected(error);
                }
            },
        )?;
        self.imp().connection.replace(Some(connection));
        self.request(sway::SUBSCRIBE, br#"["workspace","output"]"#)?;
        self.request(sway::WORKSPACES, b"")?;
        self.request(sway::OUTPUTS, b"")
    }

    fn disconnected(&self, error: String) {
        glib::g_warning!(
            "way-shell",
            "Sway IPC at {}: {error}; reconnecting",
            self.imp().path.get().unwrap().display()
        );
        if let Some(connection) = self.imp().connection.borrow_mut().take() {
            connection.close();
        }
        if self.imp().ready.replace(false) {
            self.emit_by_name::<()>("connection-changed", &[&false]);
        }
        self.imp().workspaces.borrow_mut().clear();
        self.imp().outputs.borrow_mut().clear();
        self.emit_by_name::<()>("workspaces-changed", &[]);
        self.emit_by_name::<()>("outputs-changed", &[]);
        if self.imp().retry.borrow().is_none() {
            let weak = self.downgrade();
            let retry = glib::MainContext::ref_thread_default().spawn_local(async move {
                glib::timeout_future(Duration::from_secs(1)).await;
                if let Some(service) = weak.upgrade() {
                    service.imp().retry.borrow_mut().take();
                    if let Err(error) = service.connect() {
                        service.disconnected(error);
                    }
                }
            });
            self.imp().retry.replace(Some(retry));
        }
    }

    fn setting(&self, key: &str) -> bool {
        self.imp()
            .settings
            .borrow()
            .as_ref()
            .is_some_and(|(settings, _)| settings.boolean(key))
    }

    fn handle(&self, frame: sway::Frame) -> Result<(), String> {
        match frame.kind {
            sway::SUBSCRIBE => {
                sway::acknowledged(&frame.payload, false)?;
                if !self.imp().ready.replace(true) {
                    self.emit_by_name::<()>("connection-changed", &[&true]);
                }
            }
            sway::COMMAND => {
                if let Err(error) = sway::acknowledged(&frame.payload, true) {
                    glib::g_warning!("way-shell", "Sway command failed: {error}");
                }
            }
            sway::WORKSPACES => {
                let mut workspaces = sway::workspaces(&frame.payload)
                    .map_err(|e| format!("Invalid Sway workspace response: {e}"))?;
                if self.setting("sort-workspaces-alphabetical") {
                    workspaces.sort_by(|a, b| a.name.cmp(&b.name));
                }
                self.imp().workspaces.replace(workspaces);
                self.emit_by_name::<()>("workspaces-changed", &[]);
            }
            sway::OUTPUTS => {
                let outputs = sway::outputs(&frame.payload)
                    .map_err(|e| format!("Invalid Sway output response: {e}"))?;
                self.imp().outputs.replace(outputs);
                self.emit_by_name::<()>("outputs-changed", &[]);
            }
            sway::WORKSPACE_EVENT => {
                let event: sway::WorkspaceEvent = serde_json::from_slice(&frame.payload)
                    .map_err(|e| format!("Invalid Sway workspace event: {e}"))?;
                if let Some(workspace) = event.current {
                    if event.change == "init" {
                        self.workspace_hook(&workspace.name);
                    }
                    if event.change == "urgent"
                        && workspace.urgent
                        && self.setting("focus-urgent-workspace")
                    {
                        self.perform(&Action::FocusWorkspace((&workspace).into()))?;
                    }
                }
                self.request(sway::WORKSPACES, b"")?;
            }
            sway::OUTPUT_EVENT => self.request(sway::OUTPUTS, b"")?,
            _ => (),
        }
        Ok(())
    }

    fn workspace_hook(&self, name: &str) {
        let script = self.imp().config.get().unwrap().join("on_workspace_new.sh");
        if !script.is_file() {
            return;
        }
        let command = format!(
            "{} {}",
            glib::shell_quote(script.as_os_str()).to_string_lossy(),
            glib::shell_quote(name).to_string_lossy()
        );
        if let Err(error) = glib::spawn_command_line_async(command) {
            glib::g_warning!("way-shell", "Could not run workspace hook: {error}");
        }
    }
}

fn find_sway_socket() -> Result<PathBuf, String> {
    for key in ["SWAYSOCK", "I3SOCK"] {
        if let Some(path) = std::env::var_os(key) {
            return Ok(path.into());
        }
    }
    let directory = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or("Neither SWAYSOCK, I3SOCK nor XDG_RUNTIME_DIR is set")?;
    let mut sockets: Vec<_> = std::fs::read_dir(directory)
        .map_err(|e| format!("Could not search for the Sway socket: {e}"))?
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("sway-ipc.")
                && name.ends_with(".sock")
                && entry.file_type().is_ok_and(|kind| kind.is_socket())
        })
        .map(|entry| entry.path())
        .collect();
    sockets.sort();
    sockets
        .into_iter()
        .next()
        .ok_or_else(|| "No Sway IPC socket was found in XDG_RUNTIME_DIR".into())
}
