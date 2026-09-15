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
    niri, sway,
    wm::{Action, Output, Workspace},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    #[default]
    Sway,
    Niri,
}

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct WindowManager {
        pub backend: Cell<Backend>,
        pub event_connection: RefCell<Option<Connection>>,
        pub event_decoder: RefCell<niri::Decoder>,
        pub command_decoder: RefCell<niri::Decoder>,
        pub path: OnceCell<PathBuf>,
        pub config: OnceCell<PathBuf>,
        pub settings: RefCell<Option<(gio::Settings, glib::SignalHandlerId)>>,
        pub connection: RefCell<Option<Connection>>,
        pub retry: RefCell<Option<glib::JoinHandle<()>>>,
        pub decoder: RefCell<sway::Decoder>,
        pub workspaces: RefCell<Vec<Workspace>>,
        pub outputs: RefCell<Vec<Output>>,
        pub ready: Cell<bool>,
        pub stopped: Cell<bool>,
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
            self.obj().stop_inner(false);
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
        Self::with_settings(Backend::Sway, settings, path, config)
    }

    pub fn niri() -> Result<Self, String> {
        Self::niri_with_settings(
            super::settings::open("org.ldelossa.way-shell.window-manager")
                .map_err(|e| e.to_string())?,
            std::env::var_os("NIRI_SOCKET")
                .ok_or("NIRI_SOCKET is not set")?
                .into(),
            glib::user_config_dir().join("way-shell"),
        )
    }
    pub fn niri_with_settings(
        settings: gio::Settings,
        path: PathBuf,
        config: PathBuf,
    ) -> Result<Self, String> {
        Self::with_settings(Backend::Niri, settings, path, config)
    }
    fn with_settings(
        backend: Backend,
        settings: gio::Settings,
        path: PathBuf,
        config: PathBuf,
    ) -> Result<Self, String> {
        let service: Self = glib::Object::new();
        service.imp().backend.set(backend);
        service.imp().path.set(path).unwrap();
        service.imp().config.set(config).unwrap();
        let weak = service.downgrade();
        let handler =
            settings.connect_changed(Some("sort-workspaces-alphabetical"), move |_, _| {
                if let Some(service) = weak.upgrade()
                    && service.imp().backend.get() == Backend::Sway
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

    /// Permanently disconnect this owner. Unlike a transport failure, an
    /// explicit stop must not retry while a closed widget retains its handle.
    pub fn stop(&self) {
        self.stop_inner(true);
    }

    fn stop_inner(&self, notify: bool) {
        let imp = self.imp();
        if imp.stopped.replace(true) {
            return;
        }
        let settings = imp.settings.borrow_mut().take();
        if let Some((settings, handler)) = settings {
            settings.disconnect(handler);
        }
        let retry = imp.retry.borrow_mut().take();
        if let Some(retry) = retry {
            retry.abort();
        }
        for connection in [&imp.event_connection, &imp.connection] {
            let connection = connection.borrow_mut().take();
            if let Some(connection) = connection {
                connection.close();
            }
        }
        let connected = imp.ready.replace(false);
        let workspaces = !imp.workspaces.borrow().is_empty();
        let outputs = !imp.outputs.borrow().is_empty();
        imp.workspaces.borrow_mut().clear();
        imp.outputs.borrow_mut().clear();
        imp.decoder.replace(sway::Decoder::default());
        imp.command_decoder.replace(niri::Decoder::default());
        imp.event_decoder.replace(niri::Decoder::default());
        if notify {
            if connected {
                self.emit_by_name::<()>("connection-changed", &[&false]);
            }
            if workspaces {
                self.emit_by_name::<()>("workspaces-changed", &[]);
            }
            if outputs {
                self.emit_by_name::<()>("outputs-changed", &[]);
            }
        }
    }

    pub fn perform(&self, action: &Action) -> Result<(), String> {
        if self.imp().stopped.get() {
            return Err("Compositor service has stopped".into());
        }
        match self.imp().backend.get() {
            Backend::Sway => self.request(sway::COMMAND, sway::command(action)?.as_bytes()),
            Backend::Niri => self.niri_request(&niri::command(action)?),
        }
    }

    fn request(&self, kind: u32, payload: &[u8]) -> Result<(), String> {
        if self.imp().stopped.get() {
            return Err("Compositor service has stopped".into());
        }
        let connection = self
            .imp()
            .connection
            .borrow()
            .clone()
            .ok_or("Compositor is disconnected")?;
        connection.send(sway::encode(kind, payload)?)
    }

    fn connect(&self) -> Result<(), String> {
        if self.imp().stopped.get() {
            return Err("Compositor service has stopped".into());
        }
        if self.imp().backend.get() == Backend::Niri {
            return self.connect_niri();
        }
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
        if self.imp().stopped.get() {
            return;
        }
        glib::g_warning!(
            "way-shell",
            "{:?} IPC at {}: {error}; reconnecting",
            self.imp().backend.get(),
            self.imp().path.get().unwrap().display()
        );
        if let Some(connection) = self.imp().event_connection.borrow_mut().take() {
            connection.close();
        }
        if let Some(connection) = self.imp().connection.borrow_mut().take() {
            connection.close();
        }
        if self.imp().ready.replace(false) {
            self.emit_by_name::<()>("connection-changed", &[&false]);
        }
        if self.imp().stopped.get() {
            return;
        }
        self.imp().workspaces.borrow_mut().clear();
        self.imp().outputs.borrow_mut().clear();
        self.emit_by_name::<()>("workspaces-changed", &[]);
        if self.imp().stopped.get() {
            return;
        }
        self.emit_by_name::<()>("outputs-changed", &[]);
        if !self.imp().stopped.get() && self.imp().retry.borrow().is_none() {
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

    fn niri_request(&self, request: &serde_json::Value) -> Result<(), String> {
        if self.imp().stopped.get() {
            return Err("Compositor service has stopped".into());
        }
        let connection = self
            .imp()
            .connection
            .borrow()
            .clone()
            .ok_or("Niri is disconnected")?;
        connection.send(niri::encode(request))
    }
    fn connect_niri(&self) -> Result<(), String> {
        self.imp().command_decoder.replace(niri::Decoder::default());
        self.imp().event_decoder.replace(niri::Decoder::default());
        let weak = self.downgrade();
        let lost = self.downgrade();
        let connection = Connection::connect(
            self.imp().path.get().unwrap(),
            move |bytes| {
                if let Some(service) = weak.upgrade() {
                    let values = service.imp().command_decoder.borrow_mut().push(bytes)?;
                    for value in values {
                        service.niri_response(value)?;
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
        let weak = self.downgrade();
        let lost = self.downgrade();
        let events = Connection::connect(
            self.imp().path.get().unwrap(),
            move |bytes| {
                if let Some(service) = weak.upgrade() {
                    let values = service.imp().event_decoder.borrow_mut().push(bytes)?;
                    for value in values {
                        service.niri_event(value)?;
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
        events.send(niri::encode(&serde_json::json!("EventStream")))?;
        self.imp().event_connection.replace(Some(events));
        Ok(())
    }
    fn niri_response(&self, value: serde_json::Value) -> Result<(), String> {
        if self.imp().stopped.get() {
            return Ok(());
        }
        // A rejected action leaves the connection usable. A malformed reply
        // loses the protocol contract and requires a fresh subscription.
        let reply = serde_json::from_value::<Result<niri::Response, String>>(value)
            .map_err(|error| format!("Invalid Niri reply: {error}"))?;
        match reply {
            Ok(niri::Response::Handled) => (),
            Ok(niri::Response::Workspaces(values)) => {
                self.imp()
                    .workspaces
                    .replace(values.into_iter().map(Into::into).collect());
                self.niri_workspaces_changed();
            }
            Ok(niri::Response::Outputs(outputs)) => {
                self.imp().outputs.replace(outputs.into_values().collect());
                self.niri_outputs_changed();
            }
            Err(error) => {
                glib::g_warning!("way-shell", "Niri request failed: {error}");
            }
        }
        Ok(())
    }
    fn niri_event(&self, value: serde_json::Value) -> Result<(), String> {
        if self.imp().stopped.get() {
            return Ok(());
        }
        if !self.imp().ready.get() {
            if !matches!(niri::response(value)?, niri::Response::Handled) {
                return Err("Niri rejected the event subscription".into());
            }
            self.imp().ready.set(true);
            self.emit_by_name::<()>("connection-changed", &[&true]);
            return Ok(());
        }
        let event = niri::event(value)?;
        if matches!(event, niri::Event::Other) {
            return Ok(());
        }
        let outputs_changed = matches!(event, niri::Event::WorkspacesChanged { .. });
        let updated = niri::update(&mut self.imp().workspaces.borrow_mut(), event);
        if updated {
            self.niri_workspaces_changed();
        } else {
            self.niri_request(&serde_json::json!("Workspaces"))?;
        }
        // Niri has no dedicated output event. Workspace topology changes cover
        // output addition/removal; request the authoritative output inventory.
        if outputs_changed && !self.imp().stopped.get() {
            self.niri_request(&serde_json::json!("Outputs"))?;
        }
        Ok(())
    }
    fn niri_workspaces_changed(&self) {
        if self.imp().stopped.get() {
            return;
        }
        self.imp()
            .workspaces
            .borrow_mut()
            .sort_by_key(|workspace| workspace.num);
        self.emit_by_name::<()>("workspaces-changed", &[]);
        self.niri_outputs_changed();
    }
    fn niri_outputs_changed(&self) {
        if self.imp().stopped.get() {
            return;
        }
        {
            let workspaces = self.imp().workspaces.borrow();
            for output in self.imp().outputs.borrow_mut().iter_mut() {
                output.current_workspace = workspaces
                    .iter()
                    .find(|workspace| {
                        workspace.visible && workspace.output.as_deref() == Some(&output.name)
                    })
                    .map(|workspace| workspace.name.clone());
            }
        }
        self.emit_by_name::<()>("outputs-changed", &[]);
    }

    fn setting(&self, key: &str) -> bool {
        self.imp()
            .settings
            .borrow()
            .as_ref()
            .is_some_and(|(settings, _)| settings.boolean(key))
    }

    fn handle(&self, frame: sway::Frame) -> Result<(), String> {
        if self.imp().stopped.get() {
            return Ok(());
        }
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
        if self.imp().stopped.get() {
            return;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopping_from_subscription_ignores_queued_events_and_retries() {
        for backend in [Backend::Sway, Backend::Niri] {
            let service: WindowManager = glib::Object::new();
            service.imp().backend.set(backend);
            let handler = service.connect_local("connection-changed", false, |values| {
                if values[1].get::<bool>().unwrap() {
                    values[0].get::<WindowManager>().unwrap().stop();
                }
                None
            });
            match backend {
                Backend::Sway => service
                    .handle(sway::Frame {
                        kind: sway::SUBSCRIBE,
                        payload: br#"{"success":true}"#.to_vec(),
                    })
                    .unwrap(),
                Backend::Niri => service
                    .niri_event(serde_json::json!({"Ok":"Handled"}))
                    .unwrap(),
            }
            assert!(!service.is_connected());
            service
                .handle(sway::Frame {
                    kind: sway::WORKSPACES,
                    payload: include_bytes!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../../tests/fixtures/sway-workspaces.json"
                    ))
                    .to_vec(),
                })
                .unwrap();
            let workspaces: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/niri-workspaces.json"
            )))
            .unwrap();
            service.niri_response(workspaces.clone()).unwrap();
            service
                .niri_event(serde_json::json!({
                    "WorkspacesChanged": {"workspaces": workspaces["Ok"]["Workspaces"]}
                }))
                .unwrap();
            // No path/config was initialized: these must return before either
            // accessing a hook or scheduling a new compositor connection.
            service.workspace_hook("stopped");
            service.disconnected("a queued transport callback".into());
            assert!(service.workspaces().is_empty());
            assert!(service.outputs().is_empty());
            assert!(service.imp().retry.borrow().is_none());
            assert!(
                service
                    .perform(&Action::RenameWorkspace("stopped".into()))
                    .is_err()
            );
            service.stop();
            service.disconnect(handler);
        }
    }
}
