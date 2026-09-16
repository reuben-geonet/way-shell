//! Monitor-bound click-away surfaces sharing one popup's visibility lifetime.
use super::{CloseReason, LayerWindow, WindowRole};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

type Callback = Rc<dyn Fn()>;

#[derive(Clone)]
struct Underlay {
    window: LayerWindow,
    button: gtk::Button,
}

struct Inner {
    role: WindowRole,
    monitors: gio::ListModel,
    monitor_handler: RefCell<Option<glib::SignalHandlerId>>,
    surfaces: RefCell<HashMap<gtk::gdk::Monitor, Underlay>>,
    dismiss: RefCell<Vec<Callback>>,
    on_closed: RefCell<Vec<Callback>>,
    visible: Cell<bool>,
    generation: Cell<u64>,
    reconciling: Cell<bool>,
    pending: Cell<bool>,
    closed: Cell<bool>,
}

impl Inner {
    fn snapshot(&self) -> Vec<Underlay> {
        // Follow GDK's ordering, but never hold the collection across GTK calls.
        (0..self.monitors.n_items())
            .filter_map(|i| self.monitors.item(i).and_downcast::<gtk::gdk::Monitor>())
            .filter_map(|monitor| self.surfaces.borrow().get(&monitor).cloned())
            .collect()
    }

    fn dismiss(&self) {
        if self.closed.get() || !self.visible.get() {
            return;
        }
        let callbacks = self.dismiss.borrow().clone();
        for callback in callbacks {
            if self.closed.get() || !self.visible.get() {
                break;
            }
            callback();
        }
    }

    fn reconcile(self: &Rc<Self>) -> Result<(), String> {
        if self.closed.get() {
            return Ok(());
        }
        self.pending.set(true);
        if self.reconciling.replace(true) {
            return Ok(());
        }
        let result = (|| {
            while self.pending.replace(false) && !self.closed.get() {
                let monitors = (0..self.monitors.n_items())
                    .filter_map(|i| self.monitors.item(i).and_downcast::<gtk::gdk::Monitor>())
                    .filter(|monitor| monitor.is_valid())
                    .collect::<Vec<_>>();
                let removed = self
                    .surfaces
                    .borrow()
                    .keys()
                    .filter(|monitor| !monitors.contains(monitor))
                    .cloned()
                    .collect::<Vec<_>>();
                if !removed.is_empty() {
                    self.dismiss();
                }
                for monitor in removed {
                    let surface = self.surfaces.borrow_mut().remove(&monitor);
                    if let Some(surface) = surface {
                        surface.window.close();
                    }
                }
                for monitor in monitors {
                    if self.closed.get() {
                        break;
                    }
                    if self.surfaces.borrow().contains_key(&monitor) {
                        continue;
                    }
                    let window = LayerWindow::new(self.role, Some(&monitor))?;
                    let button = gtk::Button::new();
                    button.set_hexpand(true);
                    button.set_vexpand(true);
                    window.window().set_content(Some(&button));
                    let weak = Rc::downgrade(self);
                    let output = monitor.clone();
                    button.connect_clicked(move |button| {
                        if let Some(inner) = weak.upgrade() {
                            let current =
                                inner.surfaces.borrow().get(&output).is_some_and(|surface| {
                                    surface.button == *button && !surface.window.is_closed()
                                });
                            if current {
                                inner.dismiss();
                            }
                        }
                    });
                    let weak = Rc::downgrade(self);
                    let output = monitor.clone();
                    window.on_closed(move |reason| {
                        if let Some(inner) = weak.upgrade().filter(|inner| !inner.closed.get()) {
                            if reason == CloseReason::OutputRemoved {
                                let removed = inner.surfaces.borrow_mut().remove(&output);
                                if removed.is_some() {
                                    inner.dismiss();
                                }
                            } else if inner.surfaces.borrow().contains_key(&output) {
                                // Losing a live blocker must not leave a keyboard grab
                                // with incomplete click-away coverage.
                                inner.close();
                            }
                        }
                    });
                    if self.closed.get() {
                        window.close();
                        break;
                    }
                    self.surfaces.borrow_mut().insert(
                        monitor,
                        Underlay {
                            window: window.clone(),
                            button,
                        },
                    );
                    if self.visible.get() {
                        window.present();
                    }
                }
            }
            Ok(())
        })();
        self.reconciling.set(false);
        result
    }

    fn set_visible(&self, visible: bool) {
        if self.closed.get() {
            return;
        }
        self.visible.set(visible);
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        for surface in self.snapshot() {
            // A GTK visibility observer can synchronously reopen or close the
            // popup while an earlier transition is walking its underlays.
            if self.closed.get() || self.generation.get() != generation {
                break;
            }
            if visible {
                surface.window.present();
            } else {
                surface.window.hide();
            }
        }
    }

    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.visible.set(false);
        if let Some(handler) = self.monitor_handler.borrow_mut().take() {
            self.monitors.disconnect(handler);
        }
        let surfaces = self.surfaces.take();
        for surface in surfaces.into_values() {
            surface.window.close();
        }
        self.dismiss.borrow_mut().clear();
        let callbacks = self.on_closed.take();
        for callback in callbacks {
            callback();
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.on_closed.get_mut().clear();
        self.close();
    }
}

/// One click-catching surface per live monitor. Retained widgets become inert
/// when removed or closed; the owning VisibilityController closes the whole set.
#[derive(Clone)]
pub struct UnderlaySet(Rc<Inner>);

impl UnderlaySet {
    pub fn new(role: WindowRole) -> Result<Self, String> {
        let display = gtk::gdk::Display::default().ok_or("No GTK display is available")?;
        let inner = Rc::new(Inner {
            role,
            monitors: display.monitors(),
            monitor_handler: RefCell::new(None),
            surfaces: RefCell::new(HashMap::new()),
            dismiss: RefCell::new(Vec::new()),
            on_closed: RefCell::new(Vec::new()),
            visible: Cell::new(false),
            generation: Cell::new(0),
            reconciling: Cell::new(false),
            pending: Cell::new(false),
            closed: Cell::new(false),
        });
        let weak = Rc::downgrade(&inner);
        let handler = inner
            .monitors
            .connect_items_changed(move |_, _, removed, _| {
                if let Some(inner) = weak.upgrade() {
                    if removed != 0 {
                        inner.dismiss();
                    }
                    if let Err(error) = inner.reconcile() {
                        inner.close();
                        glib::g_warning!("way-shell", "Could not update popup underlays: {error}");
                    }
                }
            });
        inner.monitor_handler.replace(Some(handler));
        inner.reconcile()?;
        Ok(Self(inner))
    }

    pub fn windows(&self) -> Vec<LayerWindow> {
        self.0
            .snapshot()
            .into_iter()
            .map(|surface| surface.window)
            .collect()
    }
    pub fn buttons(&self) -> Vec<gtk::Button> {
        self.0
            .snapshot()
            .into_iter()
            .map(|surface| surface.button)
            .collect()
    }
    /// Invoked for an outside click or output removal while mapped.
    pub fn on_dismiss(&self, callback: impl Fn() + 'static) {
        if !self.0.closed.get() {
            self.0.dismiss.borrow_mut().push(Rc::new(callback));
        }
    }
    pub(super) fn on_closed(&self, callback: impl Fn() + 'static) {
        if self.0.closed.get() {
            callback();
        } else {
            self.0.on_closed.borrow_mut().push(Rc::new(callback));
        }
    }
    pub(super) fn present(&self) {
        self.0.set_visible(true);
    }
    pub(super) fn hide(&self) {
        self.0.set_visible(false);
    }
    pub(super) fn close(&self) {
        self.0.close();
    }
}
