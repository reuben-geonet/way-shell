//! One owned panel for each live GDK monitor.
pub mod clock;
pub mod status;
pub mod tray;
pub mod workspaces;

use self::{
    clock::PanelClock,
    status::{StatusAction, StatusBar, StatusServices},
    tray::TrayBar,
    workspaces::WorkspacesBar,
};
use crate::{
    services::{
        clock::ClockService, notifications::NotificationsService, tray::TrayService,
        wm::WindowManager,
    },
    ui::window::{LayerWindow, WindowRole},
};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

#[derive(Clone)]
pub struct PanelServices {
    pub clock: ClockService,
    pub notifications: NotificationsService,
    pub manager: WindowManager,
    pub status: StatusServices,
    /// None when tray icons were disabled at application startup.
    pub tray: Option<TrayService>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelAction {
    ToggleMessageTray,
    ToggleQuickSettings,
    ToggleShortcuts(gtk::gdk::Monitor),
}

struct Panel {
    window: LayerWindow,
    container: gtk::CenterBox,
    clock: PanelClock,
    status: StatusBar,
    help: gtk::Button,
    help_handler: Option<glib::SignalHandlerId>,
    _workspaces: WorkspacesBar,
    _tray: Option<TrayBar>,
}
impl Panel {
    fn new(monitor: &gtk::gdk::Monitor, owner: &Inner) -> Result<Self, String> {
        let window = LayerWindow::new(WindowRole::Panel, Some(monitor))?;
        let container = gtk::CenterBox::new();
        container.set_widget_name("panel");
        let left = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let center = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let right = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        center.set_halign(gtk::Align::Center);
        container.set_start_widget(Some(&left));
        container.set_center_widget(Some(&center));
        container.set_end_widget(Some(&right));
        let workspaces = WorkspacesBar::new(owner.services.manager.clone(), monitor);
        left.append(workspaces.widget());
        let action = owner.action.clone();
        let clock = PanelClock::new(
            owner.services.clock.clone(),
            owner.services.notifications.clone(),
            &owner.panel_settings,
            owner.notification_settings.clone(),
            move || action(PanelAction::ToggleMessageTray),
        )
        .map_err(|error| error.to_string())?;
        center.append(clock.widget());
        let action = owner.action.clone();
        let status = StatusBar::new(
            owner.services.status.clone(),
            move |request| match request {
                StatusAction::ToggleQuickSettings => action(PanelAction::ToggleQuickSettings),
            },
        );
        let help = gtk::Button::from_icon_name("help-about-symbolic");
        help.add_css_class("panel-shortcuts");
        help.set_tooltip_text(Some("Keyboard shortcuts"));
        help.update_property(&[gtk::accessible::Property::Label("Keyboard shortcuts")]);
        let action = owner.action.clone();
        let output = monitor.clone();
        let stopped = owner.stopped.clone();
        let help_handler = help.connect_clicked(move |_| {
            if !stopped.get() && output.is_valid() {
                action(PanelAction::ToggleShortcuts(output.clone()));
            }
        });
        right.append(&help);
        right.append(status.widget());
        let tray = owner.services.tray.clone().map(TrayBar::new);
        if let Some(tray) = &tray {
            right.prepend(tray.widget());
        }
        window.window().set_content(Some(&container));
        let panel = Self {
            window,
            container,
            clock,
            status,
            help,
            help_handler: Some(help_handler),
            _workspaces: workspaces,
            _tray: tray,
        };
        panel.clock.set_toggled(owner.message_tray_visible.get());
        panel.status.set_toggled(owner.quick_settings_visible.get());
        panel.set_activities_visible(owner.activities_visible.get());
        Ok(panel)
    }
    fn set_activities_visible(&self, visible: bool) {
        if visible {
            self.container.add_css_class("activities-visible");
        } else {
            self.container.remove_css_class("activities-visible");
        }
    }
}
impl Drop for Panel {
    fn drop(&mut self) {
        // Destroy the surface while its content controllers and subscriptions
        // still exist. Their final drops then make any retained widgets inert.
        if let Some(handler) = self.help_handler.take() {
            self.help.disconnect(handler);
        }
        self.window.close();
    }
}

struct Inner {
    monitors: gio::ListModel,
    monitor_handler: RefCell<Option<glib::SignalHandlerId>>,
    panels: RefCell<HashMap<gtk::gdk::Monitor, Rc<Panel>>>,
    services: PanelServices,
    panel_settings: gio::Settings,
    notification_settings: gio::Settings,
    action: Rc<dyn Fn(PanelAction)>,
    reconciling: Cell<bool>,
    pending: Cell<bool>,
    stopped: Rc<Cell<bool>>,
    message_tray_visible: Cell<bool>,
    quick_settings_visible: Cell<bool>,
    activities_visible: Cell<bool>,
}
impl Inner {
    fn reconcile(&self) -> Result<(), String> {
        if self.stopped.get() {
            return Ok(());
        }
        self.pending.set(true);
        if self.reconciling.replace(true) {
            return Ok(());
        }
        let result = (|| {
            while self.pending.replace(false) && !self.stopped.get() {
                // Reconcile the entire model, including batched removals and
                // additions at the same position. GDK monitor identity is stable.
                let monitors = (0..self.monitors.n_items())
                    .filter_map(|index| self.monitors.item(index))
                    .filter_map(|object| object.downcast::<gtk::gdk::Monitor>().ok())
                    .filter(|monitor| monitor.is_valid())
                    .collect::<Vec<_>>();
                let wanted = monitors.iter().collect::<HashSet<_>>();
                let removed = self
                    .panels
                    .borrow()
                    .keys()
                    .filter(|monitor| !wanted.contains(monitor))
                    .cloned()
                    .collect::<Vec<_>>();
                for monitor in removed {
                    let panel = self.panels.borrow_mut().remove(&monitor);
                    drop(panel);
                }
                for monitor in monitors {
                    if self.stopped.get() {
                        break;
                    }
                    if self.panels.borrow().contains_key(&monitor) {
                        continue;
                    }
                    let panel = Rc::new(Panel::new(&monitor, self)?);
                    if self.stopped.get() {
                        break;
                    }
                    self.panels.borrow_mut().insert(monitor, panel.clone());
                    panel.window.present();
                }
            }
            Ok(())
        })();
        self.reconciling.set(false);
        result
    }
    fn snapshot(&self) -> Vec<Rc<Panel>> {
        self.panels.borrow().values().cloned().collect()
    }
    fn stop(&self) {
        if self.stopped.replace(true) {
            return;
        }
        if let Some(handler) = self.monitor_handler.borrow_mut().take() {
            self.monitors.disconnect(handler);
        }
        let panels = self.panels.take();
        drop(panels);
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone)]
pub struct Panels(Rc<Inner>);
impl Panels {
    pub fn new(
        display: &gtk::gdk::Display,
        services: PanelServices,
        panel_settings: gio::Settings,
        notification_settings: gio::Settings,
        action: impl Fn(PanelAction) + 'static,
    ) -> Result<Self, String> {
        let inner = Rc::new(Inner {
            monitors: display.monitors(),
            monitor_handler: RefCell::new(None),
            panels: RefCell::new(HashMap::new()),
            services,
            panel_settings,
            notification_settings,
            action: Rc::new(action),
            reconciling: Cell::new(false),
            pending: Cell::new(false),
            stopped: Rc::new(Cell::new(false)),
            message_tray_visible: Cell::new(false),
            quick_settings_visible: Cell::new(false),
            activities_visible: Cell::new(false),
        });
        let weak = Rc::downgrade(&inner);
        let handler = inner.monitors.connect_items_changed(move |_, _, _, _| {
            if let Some(inner) = weak.upgrade()
                && let Err(error) = inner.reconcile()
            {
                glib::g_warning!("way-shell", "Could not update panels: {error}");
            }
        });
        inner.monitor_handler.replace(Some(handler));
        inner.reconcile()?;
        Ok(Self(inner))
    }
    pub fn set_message_tray_visible(&self, visible: bool) {
        self.0.message_tray_visible.set(visible);
        for panel in self.0.snapshot() {
            if self.0.stopped.get() {
                break;
            }
            panel.clock.set_toggled(self.0.message_tray_visible.get());
        }
    }
    pub fn set_quick_settings_visible(&self, visible: bool) {
        self.0.quick_settings_visible.set(visible);
        for panel in self.0.snapshot() {
            if self.0.stopped.get() {
                break;
            }
            panel
                .status
                .set_toggled(self.0.quick_settings_visible.get());
        }
    }
    pub fn set_activities_visible(&self, visible: bool) {
        self.0.activities_visible.set(visible);
        for panel in self.0.snapshot() {
            if self.0.stopped.get() {
                break;
            }
            panel.set_activities_visible(self.0.activities_visible.get());
        }
    }
    pub fn windows(&self) -> Vec<adw::Window> {
        self.0
            .snapshot()
            .iter()
            .map(|panel| panel.window.window().clone())
            .collect()
    }
    pub fn stop(&self) {
        self.0.stop();
    }
}
