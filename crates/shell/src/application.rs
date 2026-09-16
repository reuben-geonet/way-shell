//! Application lifetime, shared services and desktop surfaces on one GLib loop.
use crate::{
    APPLICATION_ID, resources,
    services::{
        apps::AppCatalog,
        audio::AudioService,
        bluetooth::BluetoothService,
        brightness::{BrightnessService, ControlKind},
        clock::ClockService,
        ipc::{Command, DispatchFuture, IpcService},
        logind::LogindService,
        media::MediaService,
        network::NetworkService,
        notifications::NotificationsService,
        power::PowerService,
        power_profiles::PowerProfilesService,
        settings,
        shortcuts::ShortcutsService,
        theme::{Theme, ThemeService},
        tray::TrayService,
        wayland::WaylandService,
        wm::{Backend, WindowManager},
    },
    ui::{
        activities::{Activities, ActivitiesEvent},
        app_switcher::AppSwitcher,
        dialog::Dialog,
        message_tray::{MessageTray, MessageTrayServices, window::MessageTrayEvent},
        osd::LevelOsd,
        output_switcher::OutputSwitcher,
        panel::{PanelAction, PanelServices, Panels, status::StatusServices},
        panel_popup::PopupEvent,
        quick_settings::{
            QuickSettingsEvent,
            controller::{QuickSettings, QuickSettingsServices},
            controls::SystemServices,
        },
        rename_switcher::RenameSwitcher,
        shortcuts::ShortcutsSheet,
        workspace_switcher::{WorkspaceMode, WorkspaceSwitcher},
    },
};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    future::poll_fn,
    os::unix::fs::DirBuilderExt,
    rc::{Rc, Weak},
    task::{Poll, Waker},
    time::{Duration, Instant},
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(2);
// The supported native platforms are Linux (Fedora and NixOS).
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;

/// GApplication owns process registration; this owner keeps the Rust runtime
/// alive even while all outputs are absent. Signal closures only hold weak refs.
pub struct ShellApplication {
    application: adw::Application,
    runtime: RefCell<Option<Rc<Runtime>>>,
    starting: RefCell<Option<glib::JoinHandle<()>>>,
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
    handlers: RefCell<Vec<glib::SignalHandlerId>>,
    signals: RefCell<Vec<glib::SourceId>>,
    activated: Cell<bool>,
    stopped: Cell<bool>,
    failed: Cell<bool>,
}
impl ShellApplication {
    pub fn new() -> Rc<Self> {
        let application =
            adw::Application::new(Some(APPLICATION_ID), gio::ApplicationFlags::FLAGS_NONE);
        let this = Rc::new(Self {
            application,
            runtime: RefCell::new(None),
            starting: RefCell::new(None),
            hold: RefCell::new(None),
            handlers: RefCell::new(Vec::new()),
            signals: RefCell::new(Vec::new()),
            activated: Cell::new(false),
            stopped: Cell::new(false),
            failed: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        let handler = this.application.connect_activate(move |_| {
            if let Some(this) = weak.upgrade() {
                this.activate();
            }
        });
        this.handlers.borrow_mut().push(handler);
        let weak = Rc::downgrade(&this);
        let handler = this.application.connect_shutdown(move |_| {
            if let Some(this) = weak.upgrade() {
                this.shutdown();
            }
        });
        this.handlers.borrow_mut().push(handler);
        this
    }
    pub fn application(&self) -> &adw::Application {
        &self.application
    }
    pub fn run(&self) -> glib::ExitCode {
        // Let GApplication parse --help before GTK needs a display. Respect the
        // caller's G_MESSAGES_DEBUG instead of overwriting the environment.
        let result = self.application.run();
        self.shutdown();
        if self.failed.get() {
            glib::ExitCode::FAILURE
        } else {
            result
        }
    }
    fn activate(self: &Rc<Self>) {
        if self.stopped.get() || self.activated.replace(true) {
            return;
        }
        self.hold.replace(Some(self.application.hold()));
        for signal in [SIGINT, SIGTERM] {
            let weak = Rc::downgrade(self);
            let source = glib::unix_signal_add_local(signal, move || {
                if let Some(this) = weak.upgrade() {
                    this.application.quit();
                }
                glib::ControlFlow::Continue
            });
            self.signals.borrow_mut().push(source);
        }
        let weak = Rc::downgrade(self);
        let failed = weak.clone();
        let task = glib::MainContext::ref_thread_default().spawn_local(async move {
            let result = Runtime::start(move |error| {
                if let Some(this) = failed.upgrade() {
                    this.fail(&error);
                }
            })
            .await;
            let Some(this) = weak.upgrade().filter(|this| !this.stopped.get()) else {
                return;
            };
            this.starting.borrow_mut().take();
            match result {
                Ok(runtime) => {
                    this.runtime.replace(Some(runtime));
                }
                Err(error) => this.fail(&error),
            }
        });
        self.starting.replace(Some(task));
    }
    fn fail(&self, error: &str) {
        if !self.stopped.get() && !self.failed.replace(true) {
            glib::g_message!("way-shell", "Could not run Way Shell: {error}");
            self.application.quit();
        }
    }
    pub fn shutdown(&self) {
        if self.stopped.replace(true) {
            return;
        }
        let task = self.starting.borrow_mut().take();
        if let Some(task) = task {
            task.abort();
        }
        for source in self.signals.take() {
            source.remove();
        }
        let runtime = self.runtime.borrow_mut().take();
        if let Some(runtime) = runtime {
            runtime.shutdown();
        }
        self.hold.borrow_mut().take();
    }
}
impl Drop for ShellApplication {
    fn drop(&mut self) {
        self.shutdown();
        for handler in self.handlers.get_mut().drain(..) {
            self.application.disconnect(handler);
        }
    }
}

struct Configuration {
    system: gio::Settings,
    panel: gio::Settings,
    notifications: gio::Settings,
}
impl Configuration {
    fn load() -> Result<Self, String> {
        Ok(Self {
            system: settings::open("org.ldelossa.way-shell.system")
                .map_err(|error| error.to_string())?,
            panel: settings::open("org.ldelossa.way-shell.panel")
                .map_err(|error| error.to_string())?,
            notifications: settings::open("org.ldelossa.way-shell.notifications")
                .map_err(|error| error.to_string())?,
        })
    }
}

struct Services {
    wayland: WaylandService,
    manager: WindowManager,
    theme: ThemeService,
    clock: ClockService,
    audio: AudioService,
    network: NetworkService,
    bluetooth: BluetoothService,
    bluetooth_network_handler: Option<glib::SignalHandlerId>,
    power: PowerService,
    profiles: PowerProfilesService,
    logind: LogindService,
    brightness: BrightnessService,
    brightness_handler: Option<glib::SignalHandlerId>,
    media: MediaService,
    notifications: NotificationsService,
    tray: Option<TrayService>,
    apps: AppCatalog,
    shortcuts: Rc<ShortcutsService>,
}
impl Services {
    fn new(
        wayland: WaylandService,
        manager: WindowManager,
        configuration: &Configuration,
        display: &gtk::gdk::Display,
    ) -> Result<Self, String> {
        let theme = ThemeService::with_settings(
            configuration.system.clone(),
            glib::user_config_dir().join("way-shell"),
        );
        theme.attach_display(display);
        let logind = LogindService::new().map_err(|error| error.to_string())?;
        let controller = logind.clone();
        let brightness = BrightnessService::new(Rc::new(move |kind, name, value, done| {
            controller.set_brightness(kind.subsystem(), name, value, done)
        }))
        .map_err(|error| error.to_string())?;
        brightness.set_backend_available(logind.state().session.is_some());
        let weak = brightness.downgrade();
        let brightness_handler = logind.connect_local("changed", false, move |values| {
            if let Some(brightness) = weak.upgrade() {
                let logind = values[0].get::<LogindService>().unwrap();
                brightness.set_backend_available(logind.state().session.is_some());
            }
            None
        });
        let network = NetworkService::new();
        let bluetooth = BluetoothService::new();
        let weak = bluetooth.downgrade();
        let bluetooth_network_handler = network.connect_local("changed", false, move |values| {
            let state = values[0].get::<NetworkService>().unwrap().state();
            if state.available
                && let Some(bluetooth) = weak.upgrade()
            {
                bluetooth.set_airplane_mode(!state.networking_enabled);
            }
            None
        });
        let state = network.state();
        if state.available {
            bluetooth.set_airplane_mode(!state.networking_enabled);
        }
        let shortcuts = ShortcutsService::new(
            manager.clone(),
            settings::open("org.ldelossa.way-shell.window-manager")
                .map_err(|error| error.to_string())?,
        );
        Ok(Self {
            shortcuts,
            wayland,
            manager,
            theme,
            clock: ClockService::new(),
            audio: AudioService::new(),
            network,
            bluetooth,
            bluetooth_network_handler: Some(bluetooth_network_handler),
            power: PowerService::new(),
            profiles: PowerProfilesService::new(),
            logind,
            brightness,
            brightness_handler: Some(brightness_handler),
            media: MediaService::new(),
            notifications: NotificationsService::new(),
            tray: configuration
                .panel
                .boolean("enable-tray-icons")
                .then(TrayService::new),
            apps: AppCatalog::new(),
        })
    }
}
impl Drop for Services {
    fn drop(&mut self) {
        if let Some(handler) = self.brightness_handler.take() {
            self.logind.disconnect(handler);
        }
        self.shortcuts.stop();
        self.apps.stop();
        self.clock.set_enabled(false);
        self.brightness.set_backend_available(false);
        self.logind.stop();
        self.audio.stop();
        if let Some(handler) = self.bluetooth_network_handler.take() {
            self.network.disconnect(handler);
        }
        self.bluetooth.stop();
        self.network.stop();
        self.media.stop();
        self.notifications.stop();
        if let Some(tray) = &self.tray {
            tray.stop();
        }
        self.manager.stop();
        self.wayland.stop();
        // Remaining native clients/watchers dispose when their last owned
        // handles drop. GIO launches and GLib spawn helpers reap their own
        // children; a process-wide waitpid handler would steal those results.
    }
}

pub struct Runtime {
    configuration: Configuration,
    display: gtk::gdk::Display,
    services: RefCell<Option<Rc<Services>>>,
    ui: RefCell<Option<Rc<Desktop>>>,
    ipc: RefCell<Option<Rc<IpcService>>>,
    handlers: RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>,
    output_update: RefCell<Option<glib::SourceId>>,
    rebuild: Cell<bool>,
    output_revision: Cell<u64>,
    running: Cell<bool>,
    failed: Cell<bool>,
    fatal: Box<dyn Fn(String)>,
}
impl Runtime {
    pub async fn start(fatal: impl Fn(String) + 'static) -> Result<Rc<Self>, String> {
        resources::get();
        let configuration = Configuration::load()?;
        let wm_settings = settings::open("org.ldelossa.way-shell.window-manager")
            .map_err(|error| error.to_string())?;
        let stored = wm_settings.user_value("backend");
        let backend = select_backend(
            stored.as_ref().and_then(|value| value.str()),
            &wm_settings.string("backend"),
            std::env::var_os("NIRI_SOCKET").is_some_and(|path| !path.is_empty()),
            std::env::var_os("SWAYSOCK").is_some_and(|path| !path.is_empty()),
        )?;
        let manager = match backend {
            Backend::Sway => WindowManager::sway(),
            Backend::Niri => WindowManager::niri(),
        }
        .map_err(|error| format!("{backend:?} compositor initialization failed: {error}"))?;
        let wayland = WaylandService::with_settings(wm_settings)
            .map_err(|error| format!("Wayland initialization failed: {error}"))?;
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        while !wayland.is_ready() || !manager.is_connected() {
            if let Some(error) = wayland.error() {
                return Err(format!("Wayland initialization failed: {error}"));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "{backend:?}/Wayland initialization did not complete within two seconds"
                ));
            }
            glib::timeout_future(Duration::from_millis(5)).await;
        }
        let display = gtk::gdk::Display::default().ok_or("No GTK Wayland display is available")?;
        let services = Rc::new(Services::new(wayland, manager, &configuration, &display)?);
        let this = Rc::new(Self {
            configuration,
            display,
            services: RefCell::new(Some(services.clone())),
            ui: RefCell::new(None),
            ipc: RefCell::new(None),
            handlers: RefCell::new(Vec::new()),
            output_update: RefCell::new(None),
            rebuild: Cell::new(false),
            output_revision: Cell::new(0),
            running: Cell::new(true),
            failed: Cell::new(false),
            fatal: Box::new(fatal),
        });
        let weak = Rc::downgrade(&this);
        let handler = services
            .wayland
            .connect_local("failed", false, move |values| {
                if let Some(this) = weak.upgrade() {
                    this.fail(format!(
                        "Wayland connection failed: {}",
                        values[1].get::<String>().unwrap()
                    ));
                }
                None
            });
        this.handlers
            .borrow_mut()
            .push((services.wayland.clone().upcast(), handler));
        let weak = Rc::downgrade(&this);
        let handler = this.display.connect_closed(move |_, error| {
            if let Some(this) = weak.upgrade() {
                this.fail(format!(
                    "GTK display closed{}",
                    if error {
                        " after a connection error"
                    } else {
                        ""
                    }
                ));
            }
        });
        this.handlers
            .borrow_mut()
            .push((this.display.clone().upcast(), handler));
        let monitors = this.display.monitors();
        let weak = Rc::downgrade(&this);
        let handler = monitors.connect_items_changed(move |_, _, removed, _| {
            if let Some(this) = weak.upgrade() {
                this.outputs_changed(removed != 0);
            }
        });
        this.handlers
            .borrow_mut()
            .push((monitors.upcast(), handler));
        this.reconcile_outputs()?;
        let weak = Rc::downgrade(&this);
        let ipc = IpcService::new(move |command| {
            weak.upgrade().map_or_else(
                || ready(Err("Way Shell has stopped".into())),
                |this| this.dispatch(command),
            )
        })
        .map_err(|error| format!("Could not start command socket: {error}"))?;
        this.ipc.replace(Some(ipc));
        Ok(this)
    }
    pub fn is_running(&self) -> bool {
        self.running.get()
    }
    pub fn shutdown(&self) {
        if !self.running.replace(false) {
            return;
        }
        let ipc = self.ipc.borrow_mut().take();
        if let Some(ipc) = ipc {
            ipc.stop();
        }
        let pending = self.output_update.borrow_mut().take();
        if let Some(pending) = pending {
            pending.remove();
        }
        for (object, handler) in self.handlers.take() {
            object.disconnect(handler);
        }
        let ui = self.ui.borrow_mut().take();
        if let Some(ui) = ui {
            ui.close();
        }
        let services = self.services.borrow_mut().take();
        drop(services);
    }
    fn fail(&self, error: String) {
        if self.running.get() && !self.failed.replace(true) {
            (self.fatal)(error);
        }
    }
    fn outputs_changed(self: &Rc<Self>, removed: bool) {
        if !self.running.get() {
            return;
        }
        self.output_revision
            .set(self.output_revision.get().wrapping_add(1));
        self.rebuild.set(self.rebuild.get() || removed);
        if self.output_update.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let source = glib::idle_add_local_once(move || {
            if let Some(this) = weak.upgrade() {
                this.output_update.borrow_mut().take();
                if let Err(error) = this.reconcile_outputs() {
                    this.fail(error);
                }
            }
        });
        self.output_update.replace(Some(source));
    }
    fn reconcile_outputs(self: &Rc<Self>) -> Result<(), String> {
        if !self.running.get() {
            return Ok(());
        }
        let monitors = self.display.monitors();
        if self.rebuild.replace(false) || monitors.n_items() == 0 {
            let previous = self.ui.borrow_mut().take();
            if let Some(previous) = previous {
                previous.close();
            }
        }
        if !self.running.get() || monitors.n_items() == 0 || self.ui.borrow().is_some() {
            return Ok(());
        }
        let revision = self.output_revision.get();
        let services = self
            .services
            .borrow()
            .clone()
            .ok_or("Services have stopped")?;
        let weak = Rc::downgrade(self);
        let desktop = Desktop::new(
            &services,
            &self.configuration,
            &self.display,
            move |action| {
                if let Some(this) = weak.upgrade().filter(|this| this.running.get()) {
                    let desktop = this.ui.borrow().clone();
                    if let Some(desktop) = desktop {
                        match action {
                            PanelAction::ToggleMessageTray => {
                                desktop.message_tray.window().toggle()
                            }
                            PanelAction::ToggleShortcuts(monitor) => {
                                if !desktop.dialog.window().is_visible()
                                    && let Err(error) = desktop.shortcuts.toggle_on(&monitor)
                                {
                                    glib::g_message!(
                                        "way-shell",
                                        "Could not show shortcuts: {error}"
                                    );
                                }
                            }
                            PanelAction::ToggleQuickSettings => {
                                desktop.quick_settings.window().toggle()
                            }
                        }
                    }
                }
            },
        )?;
        if self.running.get() && self.output_revision.get() == revision && monitors.n_items() != 0 {
            self.ui.replace(Some(desktop));
        } else {
            desktop.close();
            self.outputs_changed(true);
        }
        Ok(())
    }
    /// Create the future without retaining Runtime across an asynchronous
    /// operation. IPC cancellation can then drop it without an ownership cycle.
    pub fn dispatch(&self, command: Command) -> DispatchFuture {
        if !self.running.get() {
            return ready(Err("Way Shell has stopped".into()));
        }
        let services = self.services.borrow().clone();
        let Some(services) = services else {
            return ready(Err("Services have stopped".into()));
        };
        match command {
            Command::BrightnessUp
            | Command::BrightnessDown
            | Command::KeyboardBrightnessUp
            | Command::KeyboardBrightnessDown => {
                let kind = if matches!(command, Command::BrightnessUp | Command::BrightnessDown) {
                    ControlKind::Backlight
                } else {
                    ControlKind::Keyboard
                };
                let up = matches!(
                    command,
                    Command::BrightnessUp | Command::KeyboardBrightnessUp
                );
                brightness_action(services.brightness.clone(), kind, up)
            }
            Command::DumpDarkTheme | Command::DumpLightTheme => {
                dump_theme(if command == Command::DumpDarkTheme {
                    Theme::Dark
                } else {
                    Theme::Light
                })
            }
            Command::ThemeDark | Command::ThemeLight => ready(
                services
                    .theme
                    .set_theme(if command == Command::ThemeDark {
                        Theme::Dark
                    } else {
                        Theme::Light
                    })
                    .map_err(|error| error.to_string()),
            ),
            Command::VolumeUp
            | Command::VolumeDown
            | Command::VolumeSet(_)
            | Command::VolumeMute => ready(audio_action(&services.audio, command)),
            Command::BlueLightFilterEnable => ready(services.wayland.set_temperature(3100)),
            Command::BlueLightFilterDisable => {
                services.wayland.disable_gamma();
                ready(Ok(()))
            }
            _ => {
                let ui = self.ui.borrow().clone();
                ready(
                    ui.ok_or_else(|| "No display outputs are available".to_owned())
                        .and_then(|ui| ui.dispatch(command)),
                )
            }
        }
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Clone, Copy)]
enum DesktopEvent {
    Activities(ActivitiesEvent),
    QuickSettings(QuickSettingsEvent),
    MessageTray(MessageTrayEvent),
    Shortcuts(PopupEvent),
}
struct Desktop {
    activities: Activities,
    app_switcher: Rc<AppSwitcher>,
    workspace: Rc<WorkspaceSwitcher>,
    output: Rc<OutputSwitcher>,
    rename: Rc<RenameSwitcher>,
    dialog: Rc<Dialog>,
    quick_settings: Rc<QuickSettings>,
    message_tray: Rc<MessageTray>,
    osd: Rc<LevelOsd>,
    panels: Panels,
    shortcuts: Rc<ShortcutsSheet>,
    popup_handlers: RefCell<Vec<(adw::Window, glib::SignalHandlerId)>>,
    events: RefCell<VecDeque<DesktopEvent>>,
    publishing: Cell<bool>,
    stopped: Cell<bool>,
}
impl Desktop {
    fn new(
        services: &Services,
        configuration: &Configuration,
        display: &gtk::gdk::Display,
        action: impl Fn(PanelAction) + 'static,
    ) -> Result<Rc<Self>, String> {
        let dialog = Dialog::new()?;
        let weak = Rc::downgrade(&dialog);
        let quick_settings = QuickSettings::new(
            QuickSettingsServices {
                system: SystemServices {
                    theme: services.theme.clone(),
                    logind: services.logind.clone(),
                    profiles: services.profiles.clone(),
                    brightness: services.brightness.clone(),
                    wayland: services.wayland.clone(),
                },
                audio: services.audio.clone(),
                network: services.network.clone(),
                bluetooth: services.bluetooth.clone(),
                power: services.power.clone(),
                notifications: services.notifications.clone(),
            },
            configuration.system.clone(),
            move |confirmation| {
                if let Some(dialog) = weak.upgrade() {
                    dialog.present(
                        confirmation.action.title(),
                        confirmation.action.body(),
                        confirmation.respond,
                    );
                } else {
                    (confirmation.respond)(false);
                }
            },
        )?;
        let weak = Rc::downgrade(&quick_settings);
        let osd = LevelOsd::new(
            services.audio.clone(),
            services.brightness.clone(),
            move || {
                weak.upgrade()
                    .is_some_and(|settings| settings.window().is_visible())
            },
        )?;
        let message_tray = MessageTray::new(
            MessageTrayServices {
                clock: services.clock.clone(),
                media: services.media.clone(),
                notifications: services.notifications.clone(),
            },
            configuration.notifications.clone(),
        )?;
        let activities = Activities::new(services.apps.clone())?;
        let app_switcher = AppSwitcher::new(services.wayland.clone())?;
        let workspace = WorkspaceSwitcher::new(services.manager.clone())?;
        let output = OutputSwitcher::new(services.manager.clone())?;
        let rename = RenameSwitcher::new(services.manager.clone())?;
        let shortcuts = ShortcutsSheet::new(services.shortcuts.clone(), services.manager.clone())?;
        let panels = Panels::new(
            display,
            PanelServices {
                clock: services.clock.clone(),
                notifications: services.notifications.clone(),
                manager: services.manager.clone(),
                status: StatusServices {
                    audio: services.audio.clone(),
                    network: services.network.clone(),
                    power: services.power.clone(),
                    logind: services.logind.clone(),
                    wayland: services.wayland.clone(),
                },
                tray: services.tray.clone(),
            },
            configuration.panel.clone(),
            configuration.notifications.clone(),
            action,
        )?;
        let this = Rc::new(Self {
            shortcuts,
            popup_handlers: RefCell::new(Vec::new()),
            activities,
            app_switcher,
            workspace,
            output,
            rename,
            dialog,
            quick_settings,
            message_tray,
            osd,
            panels,
            events: RefCell::new(VecDeque::new()),
            publishing: Cell::new(false),
            stopped: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        this.activities.on_event(move |event| {
            if let Some(this) = weak.upgrade() {
                this.publish(DesktopEvent::Activities(event));
            }
        });
        let weak = Rc::downgrade(&this);
        this.quick_settings.window().on_event(move |event| {
            if let Some(this) = weak.upgrade() {
                this.publish(DesktopEvent::QuickSettings(event));
            }
        });
        let weak = Rc::downgrade(&this);
        this.message_tray.window().on_event(move |event| {
            if let Some(this) = weak.upgrade() {
                this.publish(DesktopEvent::MessageTray(event));
            }
        });
        let weak = Rc::downgrade(&this);
        this.shortcuts.popup().on_event(move |event| {
            if let Some(this) = weak.upgrade() {
                this.publish(DesktopEvent::Shortcuts(event));
            }
        });
        for window in [
            this.workspace.switcher().window(),
            this.output.switcher().window(),
            this.rename.switcher().window(),
            this.app_switcher.window(),
            this.dialog.window(),
        ] {
            let weak = Rc::downgrade(&this);
            let handler = window.connect_visible_notify(move |window| {
                if window.is_visible()
                    && let Some(this) = weak.upgrade()
                {
                    this.shortcuts.hide();
                }
            });
            this.popup_handlers
                .borrow_mut()
                .push((window.clone(), handler));
        }
        Ok(this)
    }
    fn publish(&self, event: DesktopEvent) {
        if self.stopped.get() {
            return;
        }
        self.events.borrow_mut().push_back(event);
        if self.publishing.replace(true) {
            return;
        }
        loop {
            let event = self.events.borrow_mut().pop_front();
            let Some(event) = event else {
                break;
            };
            if self.stopped.get() {
                break;
            }
            match event {
                DesktopEvent::Shortcuts(PopupEvent::Visible) => {
                    if self.dialog.window().is_visible() {
                        self.shortcuts.hide();
                        continue;
                    }
                    self.activities.hide();
                    self.quick_settings.window().hide();
                    self.message_tray.window().hide();
                    self.hide_selectors();
                    self.rename.hide();
                    self.app_switcher.hide();
                }
                DesktopEvent::Activities(ActivitiesEvent::WillShow) => {
                    self.shortcuts.hide();
                    self.panels.set_activities_visible(true);
                    self.quick_settings.window().hide();
                    self.message_tray.window().hide();
                    self.hide_selectors();
                }
                DesktopEvent::Activities(ActivitiesEvent::WillHide | ActivitiesEvent::Hidden) => {
                    self.panels.set_activities_visible(false)
                }
                DesktopEvent::QuickSettings(QuickSettingsEvent::Visible) => {
                    self.shortcuts.hide();
                    self.panels.set_quick_settings_visible(true);
                    self.activities.hide();
                    self.message_tray.window().hide();
                    self.hide_selectors();
                }
                DesktopEvent::QuickSettings(QuickSettingsEvent::WillShow) => self.shortcuts.hide(),
                DesktopEvent::QuickSettings(QuickSettingsEvent::Hidden) => {
                    self.panels.set_quick_settings_visible(false)
                }
                DesktopEvent::MessageTray(MessageTrayEvent::Visible) => {
                    self.shortcuts.hide();
                    self.panels.set_message_tray_visible(true);
                    self.activities.hide();
                    self.quick_settings.window().hide();
                    self.hide_selectors();
                }
                DesktopEvent::MessageTray(MessageTrayEvent::Hidden) => {
                    self.panels.set_message_tray_visible(false)
                }
                _ => {}
            }
        }
        self.publishing.set(false);
    }
    fn hide_selectors(&self) {
        self.workspace.hide();
        self.output.hide();
        self.osd.hide();
    }
    fn dispatch(&self, command: Command) -> Result<(), String> {
        if self.stopped.get() {
            return Err("Desktop surfaces have closed".into());
        }
        match command {
            Command::ShortcutsShow => {
                if !self.dialog.window().is_visible() {
                    self.shortcuts.show()?;
                }
            }
            Command::ShortcutsHide => self.shortcuts.hide(),
            Command::ShortcutsToggle => {
                if !self.dialog.window().is_visible() {
                    self.shortcuts.toggle()?;
                }
            }
            Command::MessageTrayOpen => {
                self.message_tray.window().show();
            }
            Command::ActivitiesShow => {
                self.activities.show();
            }
            Command::ActivitiesHide => {
                self.activities.hide();
            }
            Command::ActivitiesToggle => self.activities.toggle(),
            Command::AppSwitcherShow => self.app_switcher.show(),
            Command::AppSwitcherHide => self.app_switcher.hide(),
            Command::AppSwitcherToggle => self.app_switcher.toggle(),
            Command::WorkspaceSwitcherShow => self.workspace.show(WorkspaceMode::FocusWorkspace),
            Command::WorkspaceSwitcherHide | Command::WorkspaceAppSwitcherHide => {
                self.workspace.hide()
            }
            Command::WorkspaceSwitcherToggle => {
                self.workspace.toggle(WorkspaceMode::FocusWorkspace)
            }
            Command::WorkspaceAppSwitcherShow => self.workspace.show(WorkspaceMode::MoveWindow),
            Command::WorkspaceAppSwitcherToggle => self.workspace.toggle(WorkspaceMode::MoveWindow),
            Command::OutputSwitcherShow => self.output.show(),
            Command::OutputSwitcherHide => self.output.hide(),
            Command::OutputSwitcherToggle => self.output.toggle(),
            Command::RenameSwitcherShow => self.rename.show(),
            Command::RenameSwitcherHide => self.rename.hide(),
            Command::RenameSwitcherToggle => self.rename.toggle(),
            _ => return Err("Command does not target a desktop surface".into()),
        }
        Ok(())
    }
    fn close(&self) {
        if self.stopped.replace(true) {
            return;
        }
        self.events.borrow_mut().clear();
        for (window, handler) in self.popup_handlers.take() {
            window.disconnect(handler);
        }
        self.shortcuts.close();
        self.panels.stop();
        self.osd.close();
        self.activities.close();
        self.app_switcher.close();
        self.workspace.switcher().close();
        self.output.switcher().close();
        self.rename.switcher().close();
        self.quick_settings.close();
        self.message_tray.close();
        self.dialog.close();
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        self.close();
    }
}

fn select_backend(
    stored: Option<&str>,
    default: &str,
    niri: bool,
    sway: bool,
) -> Result<Backend, String> {
    let backend = stored.unwrap_or(if niri {
        "niri"
    } else if sway {
        "sway"
    } else {
        default
    });
    match backend {
        "sway" => Ok(Backend::Sway),
        "niri" => Ok(Backend::Niri),
        other => Err(format!(
            "Unsupported window-manager backend {other:?}; choose sway or niri"
        )),
    }
}
fn ready(result: Result<(), String>) -> DispatchFuture {
    Box::pin(std::future::ready(result))
}
fn audio_action(audio: &AudioService, command: Command) -> Result<(), String> {
    let state = audio.state();
    let id = state
        .default_sink
        .ok_or("No default audio output is available")?;
    let result = match command {
        Command::VolumeUp => audio.change_volume(id, 0.05),
        Command::VolumeDown => audio.change_volume(id, -0.05),
        Command::VolumeSet(value) => audio.set_volume(id, f64::from(value)),
        Command::VolumeMute => {
            let volume = state
                .node(id)
                .and_then(|node| node.volume.as_ref())
                .ok_or("The default audio output has no volume control")?;
            audio.set_muted(id, !volume.mute)
        }
        _ => return Err("Command does not control audio".into()),
    };
    result.map_err(|error| error.to_string())
}
#[derive(Default)]
struct Completion {
    result: Option<Result<(), String>>,
    waker: Option<Waker>,
}
// A timed-out IPC reply drops this receiver. The service has no per-operation
// cancel handle, so an already submitted hardware action may still complete;
// shutdown disables its backend to cancel all queued and in-flight operations.
fn brightness_action(service: BrightnessService, kind: ControlKind, up: bool) -> DispatchFuture {
    Box::pin(async move {
        let state = Rc::new(RefCell::new(Completion::default()));
        let weak: Weak<RefCell<Completion>> = Rc::downgrade(&state);
        service.adjust(kind, up, move |result| {
            if let Some(state) = weak.upgrade() {
                let waker = {
                    let mut state = state.borrow_mut();
                    state.result = Some(result.map_err(|error| error.to_string()));
                    state.waker.take()
                };
                if let Some(waker) = waker {
                    waker.wake();
                }
            }
        });
        poll_fn(move |context| {
            let mut state = state.borrow_mut();
            if let Some(result) = state.result.take() {
                Poll::Ready(result)
            } else {
                state.waker = Some(context.waker().clone());
                Poll::Pending
            }
        })
        .await
    })
}
fn dump_theme(theme: Theme) -> DispatchFuture {
    let bytes = resources::get()
        .lookup_data(theme.resource_path(), gio::ResourceLookupFlags::NONE)
        .map(|bytes| bytes.as_ref().to_vec())
        .map_err(|error| error.to_string());
    let directory = glib::user_config_dir().join("way-shell");
    Box::pin(async move {
        let bytes = bytes?;
        gio::spawn_blocking(move || {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&directory)
                .map_err(|error| error.to_string())?;
            glib::file_set_contents(directory.join(theme.file_name()), &bytes)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|_| "Theme export worker stopped".to_owned())?
    })
}

#[cfg(test)]
#[allow(dead_code)]
#[path = "../tests/common/audio.rs"]
mod audio_fixture;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicitly_saved_backends_override_session_detection() {
        assert_eq!(
            select_backend(Some("sway"), "sway", true, true),
            Ok(Backend::Sway)
        );
        assert_eq!(
            select_backend(Some("niri"), "sway", false, true),
            Ok(Backend::Niri)
        );
        assert!(
            select_backend(Some("unknown"), "sway", true, true)
                .unwrap_err()
                .contains("Unsupported")
        );
    }
    #[test]
    fn unsaved_backend_uses_inner_session_then_existing_default() {
        assert_eq!(select_backend(None, "sway", true, true), Ok(Backend::Niri));
        assert_eq!(select_backend(None, "niri", false, true), Ok(Backend::Sway));
        assert_eq!(
            select_backend(None, "sway", false, false),
            Ok(Backend::Sway)
        );
        assert_eq!(
            select_backend(None, "niri", false, false),
            Ok(Backend::Niri)
        );
    }

    #[test]
    fn volume_set_acknowledges_the_current_default_sink_operation() {
        use wireplumber::{
            Core, InitFlags, core::ObjectFeatures, local::ImplMetadata, prelude::*, pw::Properties,
        };

        Core::init_with_flags(InitFlags::PIPEWIRE);
        let daemon = audio_fixture::Daemon::new();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let audio = AudioService::with_remote(&daemon.remote());
                audio_fixture::wait(&context, || audio.state().available);
                assert!(audio_action(&audio, Command::VolumeSet(0.5)).is_err());

                let properties = Properties::new();
                properties.insert("remote.name", daemon.remote());
                let core = Core::new(Some(&context), None, Some(properties));
                context.block_on(core.connect_future()).unwrap();
                let capture =
                    audio_fixture::node(&context, &core, "dispatch-source", "Audio/Source");
                let playback = audio_fixture::node(&context, &core, "dispatch-sink", "Audio/Sink");
                let _link = audio_fixture::connect_nodes(&context, &core, &capture, &playback);
                let metadata = ImplMetadata::with_properties(&core, Some("default"), None);
                context
                    .block_on(metadata.activate_future(ObjectFeatures::ALL))
                    .unwrap();
                metadata.set(
                    0,
                    Some("default.audio.sink"),
                    Some("Spa:String:JSON"),
                    Some(r#"{"name":"dispatch-sink"}"#),
                );
                audio_fixture::wait(&context, || {
                    let state = audio.state();
                    state.default_sink == Some(playback.bound_id())
                        && state
                            .node(playback.bound_id())
                            .is_some_and(|node| node.volume.is_some())
                });
                assert!(
                    (audio
                        .state()
                        .node(playback.bound_id())
                        .unwrap()
                        .volume
                        .as_ref()
                        .unwrap()
                        .volume
                        - 0.5)
                        .abs()
                        > 0.01,
                    "Fixture must start away from the requested volume"
                );
                assert_eq!(audio_action(&audio, Command::VolumeSet(0.5)), Ok(()));
                audio_fixture::wait(&context, || {
                    audio
                        .state()
                        .node(playback.bound_id())
                        .and_then(|node| node.volume.as_ref())
                        .is_some_and(|volume| (volume.volume - 0.5).abs() < 0.0001)
                });
                audio.stop();
                core.disconnect();
            })
            .unwrap();
    }

    #[test]
    #[ignore = "requires the private Wayland, D-Bus, PipeWire and XDG fixture"]
    fn gtk_runtime_commands_output_recovery_and_reentrant_shutdown() {
        use way_shell_core::notifications::NotificationRequest;
        assert_eq!(
            std::env::var("WAY_SHELL_PRIVATE_RUNTIME_TEST").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("GSETTINGS_BACKEND").as_deref(), Ok("memory"));
        assert_eq!(
            std::env::var("DBUS_SYSTEM_BUS_ADDRESS"),
            std::env::var("DBUS_SESSION_BUS_ADDRESS")
        );
        adw::init().unwrap();
        let context = glib::MainContext::default();
        context
            .with_thread_default(|| {
                // Never inspect a laptop's sysfs device from this application test.
                let settings = settings::open("org.ldelossa.way-shell.system").unwrap();
                settings.set_string("backlight-directory", "").unwrap();
                settings
                    .set_string("keyboard-backlight-directory", "")
                    .unwrap();
                let failures = Rc::new(RefCell::new(Vec::new()));
                let reported = failures.clone();
                let runtime = context
                    .block_on(Runtime::start(move |error| {
                        reported.borrow_mut().push(error)
                    }))
                    .unwrap();
                let wait = |condition: &dyn Fn() -> bool| {
                    let deadline = Instant::now() + Duration::from_secs(8);
                    while !condition() {
                        assert!(
                            Instant::now() < deadline,
                            "Runtime condition timed out; failures: {:?}",
                            failures.borrow()
                        );
                        context.block_on(glib::timeout_future(Duration::from_millis(5)));
                    }
                };
                let desktop = runtime.ui.borrow().clone().unwrap();
                wait(&|| {
                    desktop
                        .panels
                        .windows()
                        .iter()
                        .all(|window| window.is_mapped())
                });
                assert!(!desktop.panels.windows().is_empty());
                for _ in 0..2 {
                    context
                        .block_on(runtime.dispatch(Command::ShortcutsShow))
                        .unwrap();
                    assert!(desktop.shortcuts.popup().is_visible());
                }
                desktop.workspace.show(WorkspaceMode::FocusWorkspace);
                assert!(!desktop.shortcuts.popup().is_visible());
                context
                    .block_on(runtime.dispatch(Command::ShortcutsShow))
                    .unwrap();
                assert!(!desktop.workspace.is_visible());
                desktop
                    .dialog
                    .present("Confirm", "Confirmation retains focus", |_| {});
                assert!(!desktop.shortcuts.popup().is_visible());
                context
                    .block_on(runtime.dispatch(Command::ShortcutsShow))
                    .unwrap();
                assert!(!desktop.shortcuts.popup().is_visible());
                desktop.dialog.cancel();
                context
                    .block_on(runtime.dispatch(Command::ShortcutsToggle))
                    .unwrap();
                assert!(desktop.shortcuts.popup().is_visible());
                context
                    .block_on(runtime.dispatch(Command::ShortcutsToggle))
                    .unwrap();
                assert!(!desktop.shortcuts.popup().is_visible());
                for _ in 0..2 {
                    context
                        .block_on(runtime.dispatch(Command::ShortcutsHide))
                        .unwrap();
                }
                let old_panel = desktop.panels.windows().remove(0);
                let socket = runtime
                    .ipc
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .socket_path()
                    .to_path_buf();
                assert!(socket.exists());
                let windows_before = gtk::Window::toplevels().n_items();
                let duplicate = context.block_on(Runtime::start(|error| {
                    panic!("Failed startup left a live observer: {error}")
                }));
                let duplicate_error = match duplicate {
                    Ok(_) => panic!("A second runtime acquired the occupied command socket"),
                    Err(error) => error,
                };
                assert!(duplicate_error.contains("Could not start command socket"));
                assert_eq!(gtk::Window::toplevels().n_items(), windows_before);
                assert!(runtime.is_running());
                assert!(socket.exists());
                context
                    .block_on(runtime.dispatch(Command::ActivitiesShow))
                    .unwrap();
                assert!(desktop.activities.window().is_visible());
                context
                    .block_on(runtime.dispatch(Command::WorkspaceSwitcherShow))
                    .unwrap();
                assert!(desktop.workspace.is_visible());
                context
                    .block_on(runtime.dispatch(Command::MessageTrayOpen))
                    .unwrap();
                wait(&|| {
                    desktop.message_tray.window().window().is_visible()
                        && !desktop.activities.window().is_visible()
                        && !desktop.workspace.is_visible()
                });
                desktop.quick_settings.window().show();
                wait(&|| {
                    desktop.quick_settings.window().is_visible()
                        && !desktop.message_tray.window().window().is_visible()
                });
                context
                    .block_on(runtime.dispatch(Command::ShortcutsShow))
                    .unwrap();
                wait(&|| !desktop.quick_settings.window().is_visible());
                desktop.message_tray.window().show();
                wait(&|| !desktop.shortcuts.popup().is_visible());
                context
                    .block_on(runtime.dispatch(Command::ThemeLight))
                    .unwrap();
                assert!(settings.boolean("light-theme"));
                context
                    .block_on(runtime.dispatch(Command::ThemeDark))
                    .unwrap();
                assert!(!settings.boolean("light-theme"));
                context
                    .block_on(runtime.dispatch(Command::DumpDarkTheme))
                    .unwrap();
                let exported =
                    std::fs::read(glib::user_config_dir().join("way-shell/way-shell-dark.css"))
                        .unwrap();
                let expected = resources::get()
                    .lookup_data(Theme::Dark.resource_path(), gio::ResourceLookupFlags::NONE)
                    .unwrap();
                assert_eq!(exported.as_slice(), expected.as_ref());
                assert!(
                    context
                        .block_on(runtime.dispatch(Command::BrightnessUp))
                        .is_err()
                );
                let notifications = runtime
                    .services
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .notifications
                    .clone();
                let id = notifications
                    .send_internal(NotificationRequest {
                        summary: "Survives output loss".into(),
                        ..Default::default()
                    })
                    .unwrap();
                desktop.quick_settings.window().hide();
                drop(desktop);

                if std::env::var_os("NIRI_SOCKET").is_none() {
                    // Sway's private headless backend supports physical output
                    // removal/re-addition. Niri's nested winit output does not.
                    let monitors = runtime.display.monitors();
                    let connector = monitors
                        .item(0)
                        .unwrap()
                        .downcast::<gtk::gdk::Monitor>()
                        .unwrap()
                        .connector()
                        .unwrap();
                    let output = std::process::Command::new("swaymsg")
                        .args(["output", connector.as_str(), "disable"])
                        .output()
                        .unwrap();
                    assert!(
                        output.status.success(),
                        "{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    wait(&|| monitors.n_items() == 0 && runtime.ui.borrow().is_none());
                    assert!(runtime.is_running());
                    assert!(socket.exists());
                    assert!(!old_panel.is_visible());
                    assert!(
                        context
                            .block_on(runtime.dispatch(Command::ActivitiesShow))
                            .is_err()
                    );
                    let output = std::process::Command::new("swaymsg")
                        .args(["output", connector.as_str(), "enable"])
                        .output()
                        .unwrap();
                    assert!(
                        output.status.success(),
                        "{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    wait(&|| {
                        runtime.ui.borrow().as_ref().is_some_and(|ui| {
                            !ui.panels.windows().is_empty()
                                && ui.panels.windows().iter().all(|window| window.is_mapped())
                        })
                    });
                    assert_ne!(
                        runtime.ui.borrow().as_ref().unwrap().panels.windows()[0],
                        old_panel
                    );
                    assert!(
                        notifications
                            .state()
                            .notifications
                            .iter()
                            .any(|notification| notification.id == id)
                    );
                    context
                        .block_on(runtime.dispatch(Command::ActivitiesShow))
                        .unwrap();
                    assert!(
                        runtime
                            .ui
                            .borrow()
                            .as_ref()
                            .unwrap()
                            .activities
                            .window()
                            .is_visible()
                    );
                }
                let desktop = runtime.ui.borrow().clone().unwrap();
                let (manager, wayland) = {
                    let services = runtime.services.borrow();
                    let services = services.as_ref().unwrap();
                    (services.manager.clone(), services.wayland.clone())
                };
                assert!(manager.is_connected());
                assert!(wayland.is_ready());
                let retained = desktop.activities.window().clone();
                let weak = Rc::downgrade(&runtime);
                let handler = retained.connect_visible_notify(move |window| {
                    if !window.is_visible()
                        && let Some(runtime) = weak.upgrade()
                    {
                        runtime.shutdown();
                    }
                });
                desktop.activities.show();
                desktop.activities.hide();
                wait(&|| !runtime.is_running());
                assert!(!socket.exists());
                assert!(runtime.ui.borrow().is_none());
                assert!(runtime.services.borrow().is_none());
                assert!(!retained.is_visible());
                assert!(
                    !manager.is_connected(),
                    "Retained UI handles must not keep compositor IPC active after shutdown"
                );
                assert!(manager.workspaces().is_empty());
                assert!(manager.outputs().is_empty());
                assert!(
                    manager
                        .perform(&way_shell_core::wm::Action::RenameWorkspace(
                            "stopped runtime must reject actions".into()
                        ))
                        .is_err()
                );
                assert!(!wayland.is_ready());
                assert!(wayland.outputs().is_empty());
                assert!(wayland.seats().is_empty());
                assert!(wayland.toplevels().is_empty());
                assert!(!wayland.has_foreign_toplevel());
                assert!(wayland.set_temperature(4500).is_err());
                // Stop another live connection from inside its own dispatch:
                // the source's temporary Driver reference must not keep it active.
                let ready_seen = Rc::new(Cell::new(false));
                let observed = ready_seen.clone();
                let stopping = WaylandService::connect().unwrap();
                let ready_handler = stopping.connect_local("ready", false, move |values| {
                    values[0].get::<WaylandService>().unwrap().stop();
                    observed.set(true);
                    None
                });
                wait(&|| ready_seen.get());
                assert!(!stopping.is_ready());
                assert!(stopping.outputs().is_empty());
                assert!(stopping.set_temperature(4500).is_err());
                stopping.disconnect(ready_handler);
                retained.disconnect(handler);
                assert!(
                    context
                        .block_on(runtime.dispatch(Command::ThemeDark))
                        .is_err()
                );
                drop(desktop);
                drop(notifications);
                let weak = Rc::downgrade(&runtime);
                drop(runtime);
                assert!(weak.upgrade().is_none());
                assert!(failures.borrow().is_empty(), "{:?}", failures.borrow());
            })
            .unwrap();
    }
}
