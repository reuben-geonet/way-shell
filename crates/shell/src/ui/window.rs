//! Owned layer surfaces and popup transitions on the GTK main thread.
//!
//! Components keep their animations and compatibility signals. In particular,
//! message-tray `will-hide` still belongs at the end of its fade, while activities
//! emits it when its revealer starts closing. This module does not change that.
mod underlay;
pub use underlay::UnderlaySet;

use adw::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};

const EDGES: [Edge; 4] = [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom];

/// Surface geometry from the existing shell. Content-specific sizing stays in
/// the component (for example the message tray's 700×600 content container).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowRole {
    Panel,
    QuickSettings,
    QuickSettingsUnderlay,
    MessageTray,
    MessageTrayUnderlay,
    Activities,
    Switcher,
    AppSwitcher,
    AppInstances,
    LevelOsd,
    NotificationOsd,
    Dialog,
    PanelPopup,
    PanelPopupUnderlay,
}

impl WindowRole {
    fn configure(self, window: &adw::Window) {
        let (namespace, layer, anchors, keyboard) = match self {
            Self::Panel => (
                "way-shell-panel",
                Layer::Top,
                &[Edge::Left, Edge::Right, Edge::Top][..],
                KeyboardMode::None,
            ),
            Self::PanelPopup => (
                "way-shell-panel-popup",
                Layer::Overlay,
                &[Edge::Top, Edge::Right][..],
                KeyboardMode::Exclusive,
            ),
            Self::PanelPopupUnderlay => (
                "way-shell-panel-popup-underlay",
                Layer::Top,
                &EDGES[..],
                KeyboardMode::None,
            ),
            Self::QuickSettings => (
                "way-shell-quick-settings",
                Layer::Overlay,
                &[Edge::Top, Edge::Right][..],
                KeyboardMode::Exclusive,
            ),
            Self::QuickSettingsUnderlay => (
                "way-shell-quick-settings-underlay",
                Layer::Top,
                &EDGES[..],
                KeyboardMode::None,
            ),
            Self::MessageTray => (
                "way-shell-message-tray",
                Layer::Overlay,
                &[Edge::Top][..],
                KeyboardMode::None,
            ),
            Self::MessageTrayUnderlay => (
                "way-shell-message-tray-underlay",
                Layer::Top,
                &EDGES[..],
                KeyboardMode::None,
            ),
            Self::Activities => (
                "way-shell-activities",
                Layer::Top,
                &EDGES[..],
                KeyboardMode::Exclusive,
            ),
            Self::Switcher | Self::AppSwitcher => (
                "way-shell-switcher",
                Layer::Overlay,
                &[Edge::Top][..],
                KeyboardMode::Exclusive,
            ),
            Self::AppInstances => (
                "way-shell-switcher",
                Layer::Overlay,
                &[Edge::Top][..],
                KeyboardMode::None,
            ),
            Self::LevelOsd => (
                "way-shell-osd",
                Layer::Overlay,
                &[Edge::Bottom][..],
                KeyboardMode::None,
            ),
            Self::NotificationOsd => (
                "way-shell-notifications-osd",
                Layer::Top,
                &[Edge::Top][..],
                KeyboardMode::None,
            ),
            Self::Dialog => (
                "way-shell-dialog",
                Layer::Overlay,
                &EDGES[..],
                KeyboardMode::Exclusive,
            ),
        };
        window.set_namespace(Some(namespace));
        window.set_layer(layer);
        window.set_keyboard_mode(keyboard);
        // Zero respects panels' reserved space. -1 would change popup placement.
        window.set_exclusive_zone(0);
        for edge in EDGES {
            window.set_anchor(edge, anchors.contains(&edge));
        }
        match self {
            Self::PanelPopup => {
                window.set_margin(Edge::Top, 8);
                window.set_margin(Edge::Right, 20);
            }
            Self::PanelPopupUnderlay => window.add_css_class("underlay"),
            Self::Panel => {
                window.auto_exclusive_zone_enable();
                window.set_size_request(-1, 30);
            }
            Self::QuickSettings => {
                window.set_margin(Edge::Top, 8);
                window.set_margin(Edge::Right, 20);
                window.set_size_request(400, 280);
            }
            Self::QuickSettingsUnderlay | Self::MessageTrayUnderlay => {
                window.add_css_class("underlay")
            }
            Self::MessageTray => window.set_margin(Edge::Top, 8),
            Self::Switcher => {
                window.set_margin(Edge::Top, 400);
                window.set_default_size(600, 0);
            }
            Self::AppSwitcher => {
                window.set_margin(Edge::Top, 500);
                window.set_default_size(0, 0);
            }
            Self::AppInstances => {
                window.set_margin(Edge::Top, 660);
                window.set_default_size(0, 0);
            }
            Self::LevelOsd => {
                window.set_margin(Edge::Bottom, 150);
                window.set_size_request(340, 64);
            }
            Self::NotificationOsd => {
                window.set_margin(Edge::Top, 10);
                window.set_size_request(440, 100);
            }
            Self::Activities | Self::Dialog => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    Requested,
    Compositor,
    OutputRemoved,
}

type CloseCallback = Box<dyn Fn(CloseReason)>;

struct LayerWindowInner {
    window: adw::Window,
    closed: Cell<Option<CloseReason>>,
    close_callbacks: RefCell<Vec<CloseCallback>>,
    window_handlers: RefCell<Vec<glib::SignalHandlerId>>,
    monitor: Option<gtk::gdk::Monitor>,
    monitor_handler: RefCell<Option<glib::SignalHandlerId>>,
}

impl LayerWindowInner {
    fn close(&self, reason: CloseReason) {
        if self.closed.get().is_some() {
            return;
        }
        self.closed.set(Some(reason));
        self.window.destroy();
        // No RefCell borrow is held while application callbacks execute.
        for callback in self.close_callbacks.take() {
            callback(reason);
        }
    }
}

impl Drop for LayerWindowInner {
    fn drop(&mut self) {
        for handler in self.window_handlers.get_mut().drain(..) {
            self.window.disconnect(handler);
        }
        if let (Some(monitor), Some(handler)) =
            (&self.monitor, self.monitor_handler.get_mut().take())
        {
            monitor.disconnect(handler);
        }
        // GTK holds a reference to top-level windows until they are destroyed.
        // Dropping the final Rust owner must therefore explicitly destroy it.
        self.window.destroy();
    }
}

/// A GTK layer surface with explicit terminal destruction. Clones share ownership.
/// Use these lifecycle methods rather than destroying the borrowed GTK window.
#[derive(Clone)]
pub struct LayerWindow(Rc<LayerWindowInner>);

impl LayerWindow {
    pub fn new(role: WindowRole, monitor: Option<&gtk::gdk::Monitor>) -> Result<Self, String> {
        if !gtk::is_initialized_main_thread() {
            return Err("Layer windows must be created on the initialized GTK main thread".into());
        }
        if !gtk4_layer_shell::is_supported() {
            return Err("The compositor does not support layer-shell".into());
        }
        if monitor.is_some_and(|monitor| !monitor.is_valid()) {
            return Err("Cannot create a layer window on a removed monitor".into());
        }
        let window = adw::Window::new();
        window.init_layer_shell();
        role.configure(&window);
        window.set_monitor(monitor);
        let inner = Rc::new(LayerWindowInner {
            window,
            closed: Cell::new(None),
            close_callbacks: RefCell::new(Vec::new()),
            window_handlers: RefCell::new(Vec::new()),
            monitor: monitor.cloned(),
            monitor_handler: RefCell::new(None),
        });
        let weak = Rc::downgrade(&inner);
        let handler = inner.window.connect_close_request(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.close(CloseReason::Compositor);
            }
            glib::Propagation::Stop
        });
        inner.window_handlers.borrow_mut().push(handler);
        if let Some(monitor) = &inner.monitor {
            let weak = Rc::downgrade(&inner);
            let handler = monitor.connect_invalidate(move |_| {
                if let Some(inner) = weak.upgrade() {
                    inner.close(CloseReason::OutputRemoved);
                }
            });
            inner.monitor_handler.replace(Some(handler));
        }
        Ok(Self(inner))
    }

    pub fn window(&self) -> &adw::Window {
        &self.0.window
    }

    pub fn close_reason(&self) -> Option<CloseReason> {
        self.0.closed.get()
    }

    pub fn is_closed(&self) -> bool {
        self.close_reason().is_some()
    }

    pub fn present(&self) -> bool {
        if self.is_closed() {
            return false;
        }
        self.0.window.present();
        true
    }

    pub fn hide(&self) {
        if !self.is_closed() {
            self.0.window.set_visible(false);
        }
    }

    pub fn close(&self) {
        self.0.close(CloseReason::Requested);
    }

    /// Called once on explicit or native close. Capture application owners weakly.
    /// Dropping the final LayerWindow destroys it without invoking these callbacks.
    pub fn on_closed(&self, callback: impl Fn(CloseReason) + 'static) {
        if let Some(reason) = self.close_reason() {
            callback(reason);
        } else {
            self.0.close_callbacks.borrow_mut().push(Box::new(callback));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Hidden,
    Showing,
    Visible,
    Hiding,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ticket {
    generation: u64,
    showing: bool,
}

struct TransitionState {
    visibility: Visibility,
    generation: u64,
}

impl Default for TransitionState {
    fn default() -> Self {
        Self {
            visibility: Visibility::Hidden,
            generation: 0,
        }
    }
}

impl TransitionState {
    fn begin(&mut self, showing: bool) -> Option<Ticket> {
        use Visibility::*;
        match (self.visibility, showing) {
            (Closed, _) | (Showing | Visible, true) | (Hiding | Hidden, false) => return None,
            _ => {}
        }
        self.generation = self.generation.wrapping_add(1);
        self.visibility = if showing { Showing } else { Hiding };
        Some(Ticket {
            generation: self.generation,
            showing,
        })
    }

    fn finish(&mut self, ticket: Ticket) -> bool {
        let expected = if ticket.showing {
            Visibility::Showing
        } else {
            Visibility::Hiding
        };
        if self.generation != ticket.generation || self.visibility != expected {
            return false;
        }
        self.visibility = if ticket.showing {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        true
    }

    fn hide_now(&mut self) {
        if self.visibility != Visibility::Closed {
            self.generation = self.generation.wrapping_add(1);
            self.visibility = Visibility::Hidden;
        }
    }

    fn close(&mut self) {
        self.visibility = Visibility::Closed;
    }
}

struct VisibilityInner {
    main: LayerWindow,
    underlays: Option<UnderlaySet>,
    state: RefCell<TransitionState>,
}

impl VisibilityInner {
    fn close(&self) {
        self.state.borrow_mut().close();
        self.main.close();
        if let Some(underlays) = &self.underlays {
            underlays.close();
        }
    }

    fn hide_surfaces(&self, generation: u64) {
        self.main.hide();
        // Hiding a GTK widget can synchronously run an observer which opens it
        // again. Do not let that obsolete hide unmap the new underlay.
        let still_hidden = {
            let state = self.state.borrow();
            state.visibility == Visibility::Hidden && state.generation == generation
        };
        if still_hidden && let Some(underlays) = &self.underlays {
            underlays.hide();
        }
    }
}

impl Drop for VisibilityInner {
    fn drop(&mut self) {
        // A retained borrowed GTK widget or LayerWindow cannot keep a popup open
        // after the component owning its visibility has gone away.
        self.main.close();
        if let Some(underlays) = &self.underlays {
            underlays.close();
        }
    }
}

/// An animation completion token. It neither owns the popup nor applies to any
/// other controller; stale callbacks can safely attempt to finish it.
#[derive(Clone)]
pub struct Transition {
    owner: Weak<VisibilityInner>,
    ticket: Ticket,
}

#[derive(Clone)]
pub struct VisibilityController(Rc<VisibilityInner>);

impl VisibilityController {
    pub fn new(main: LayerWindow, underlays: Option<UnderlaySet>) -> Self {
        main.hide();
        if let Some(underlays) = &underlays {
            underlays.hide();
        }
        let inner = Rc::new(VisibilityInner {
            main,
            underlays,
            state: RefCell::new(TransitionState::default()),
        });
        let weak = Rc::downgrade(&inner);
        inner.main.on_closed(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.close();
            }
        });
        if let Some(underlays) = &inner.underlays {
            let weak = Rc::downgrade(&inner);
            underlays.on_closed(move || {
                if let Some(inner) = weak.upgrade() {
                    inner.close();
                }
            });
        }
        Self(inner)
    }

    pub fn main(&self) -> &LayerWindow {
        &self.0.main
    }
    pub fn underlays(&self) -> Option<&UnderlaySet> {
        self.0.underlays.as_ref()
    }
    pub fn visibility(&self) -> Visibility {
        self.0.state.borrow().visibility
    }

    pub fn begin_show(&self) -> Option<Transition> {
        let ticket = self.0.state.borrow_mut().begin(true)?;
        if let Some(underlays) = &self.0.underlays {
            underlays.present();
        }
        let still_showing = {
            let state = self.0.state.borrow();
            state.generation == ticket.generation && state.visibility == Visibility::Showing
        };
        if still_showing {
            self.0.main.present();
        }
        Some(Transition {
            owner: Rc::downgrade(&self.0),
            ticket,
        })
    }

    /// The surface and click-away underlay stay mapped until this transition
    /// completes. The component decides when its own will-hide signal fires.
    pub fn begin_hide(&self) -> Option<Transition> {
        let ticket = self.0.state.borrow_mut().begin(false)?;
        Some(Transition {
            owner: Rc::downgrade(&self.0),
            ticket,
        })
    }

    pub fn finish_transition(&self, transition: &Transition) -> bool {
        if !transition.owner.ptr_eq(&Rc::downgrade(&self.0))
            || !self.0.state.borrow_mut().finish(transition.ticket)
        {
            return false;
        }
        if !transition.ticket.showing {
            self.0.hide_surfaces(transition.ticket.generation);
        }
        true
    }

    pub fn hide_now(&self) {
        self.0.state.borrow_mut().hide_now();
        let generation = self.0.state.borrow().generation;
        self.0.hide_surfaces(generation);
    }

    pub fn close(&self) {
        self.0.close();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PopupRole {
    Activities,
    QuickSettings,
    MessageTray,
    Workspace,
    Output,
    LevelOsd,
    AppSwitcher,
    Rename,
    Dialog,
}

/// These events deliberately preserve the old mediator's asymmetric timing.
#[derive(Debug, Clone, Copy)]
pub enum PopupEvent {
    ActivitiesWillShow,
    QuickSettingsVisible,
    MessageTrayVisible,
}

impl PopupEvent {
    fn peers(self) -> &'static [PopupRole] {
        use PopupRole::*;
        match self {
            Self::ActivitiesWillShow => &[QuickSettings, MessageTray, Workspace, Output, LevelOsd],
            Self::QuickSettingsVisible => &[MessageTray, Activities, Workspace, Output, LevelOsd],
            Self::MessageTrayVisible => &[QuickSettings, Activities, Workspace, Output, LevelOsd],
        }
    }
}

pub struct HideRequest {
    pub role: PopupRole,
    pub controller: VisibilityController,
    pub transition: Transition,
}

/// Coordinates only the peers in the existing mediator. Components animate the
/// returned hide requests; there is no global Escape binding or implicit focus.
#[derive(Default)]
pub struct PopupCoordinator {
    popups: RefCell<BTreeMap<PopupRole, Weak<VisibilityInner>>>,
}

impl PopupCoordinator {
    pub fn register(&self, role: PopupRole, controller: &VisibilityController) {
        self.popups
            .borrow_mut()
            .insert(role, Rc::downgrade(&controller.0));
    }

    pub fn request_peer_hides(&self, event: PopupEvent) -> Vec<HideRequest> {
        let mut popups = self.popups.borrow_mut();
        popups.retain(|_, popup| popup.strong_count() > 0);
        event
            .peers()
            .iter()
            .filter_map(|&role| {
                let controller = VisibilityController(popups.get(&role)?.upgrade()?);
                let transition = controller.begin_hide()?;
                Some(HideRequest {
                    role,
                    controller,
                    transition,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopening_rejects_the_old_hide_completion() {
        let mut state = TransitionState::default();
        let first_show = state.begin(true).unwrap();
        assert!(state.finish(first_show));
        let old_hide = state.begin(false).unwrap();
        let show = state.begin(true).unwrap();
        assert!(!state.finish(old_hide));
        assert_eq!(state.visibility, Visibility::Showing);
        assert!(state.finish(show));
        assert!(!state.finish(first_show));
        assert_eq!(state.visibility, Visibility::Visible);
    }

    #[test]
    fn hiding_during_show_rejects_stale_and_duplicate_completions() {
        let mut state = TransitionState::default();
        let show = state.begin(true).unwrap();
        assert!(state.begin(true).is_none());
        let hide = state.begin(false).unwrap();
        assert!(state.begin(false).is_none());
        assert!(!state.finish(show));
        assert!(state.finish(hide));
        assert!(!state.finish(hide));
        assert_eq!(state.visibility, Visibility::Hidden);
    }

    #[test]
    fn immediate_hide_and_terminal_close_invalidate_pending_work() {
        let mut state = TransitionState::default();
        let show = state.begin(true).unwrap();
        state.hide_now();
        assert!(!state.finish(show));
        let next = state.begin(true).unwrap();
        state.close();
        state.hide_now();
        assert!(!state.finish(next));
        assert!(state.begin(true).is_none());
        assert!(state.begin(false).is_none());
        assert_eq!(state.visibility, Visibility::Closed);
    }

    #[test]
    fn popup_policy_preserves_the_existing_exclusions_and_trigger_timing() {
        use PopupRole::*;
        assert_eq!(
            PopupEvent::ActivitiesWillShow.peers(),
            &[QuickSettings, MessageTray, Workspace, Output, LevelOsd]
        );
        assert_eq!(
            PopupEvent::QuickSettingsVisible.peers(),
            &[MessageTray, Activities, Workspace, Output, LevelOsd]
        );
        assert_eq!(
            PopupEvent::MessageTrayVisible.peers(),
            &[QuickSettings, Activities, Workspace, Output, LevelOsd]
        );
        for event in [
            PopupEvent::ActivitiesWillShow,
            PopupEvent::QuickSettingsVisible,
            PopupEvent::MessageTrayVisible,
        ] {
            for excluded in [AppSwitcher, Rename, Dialog] {
                assert!(!event.peers().contains(&excluded));
            }
        }
    }
}
