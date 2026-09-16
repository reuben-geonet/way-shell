//! Reusable panel-anchored surface; features own their services and content.
use super::window::{LayerWindow, UnderlaySet, VisibilityController, WindowRole};
use adw::prelude::*;
use gtk4_layer_shell::{KeyboardMode, LayerShell};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupEvent {
    Visible,
    Hidden,
    Dismissed,
}
type Callback = Rc<dyn Fn(PopupEvent)>;
pub struct PanelPopup {
    visibility: VisibilityController,
    content: gtk::Box,
    close_button: gtk::Button,
    width: i32,
    height: i32,
    monitor: RefCell<Option<(gtk::gdk::Monitor, glib::SignalHandlerId)>>,
    callbacks: RefCell<Vec<Callback>>,
    visible: Cell<bool>,
    stopped: Cell<bool>,
}
impl PanelPopup {
    pub fn new(
        icon: &str,
        title: &str,
        content: &impl IsA<gtk::Widget>,
        width: i32,
        height: i32,
    ) -> Result<Rc<Self>, String> {
        let surface = LayerWindow::new(WindowRole::PanelPopup, None)?;
        surface.window().set_title(Some(title));
        surface
            .window()
            .update_property(&[gtk::accessible::Property::Label(title)]);
        surface.window().add_css_class("panel-popup");
        let underlays = UnderlaySet::new(WindowRole::PanelPopupUnderlay)?;
        let visibility = VisibilityController::new(surface, Some(underlays));
        let container = gtk::Box::new(gtk::Orientation::Vertical, 12);
        container.add_css_class("panel-popup-content");
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.append(&gtk::Image::from_icon_name(icon));
        let heading = gtk::Label::new(Some(title));
        heading.add_css_class("title-3");
        heading.set_xalign(0.0);
        heading.set_hexpand(true);
        header.append(&heading);
        let close_button = gtk::Button::from_icon_name("window-close-symbolic");
        close_button.set_tooltip_text(Some("Close"));
        close_button.update_property(&[gtk::accessible::Property::Label("Close")]);
        header.append(&close_button);
        container.append(&header);
        container.append(content);
        visibility.main().window().set_content(Some(&container));
        let this = Rc::new(Self {
            visibility,
            content: container,
            close_button,
            width,
            height,
            monitor: RefCell::new(None),
            callbacks: RefCell::new(Vec::new()),
            visible: Cell::new(false),
            stopped: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        this.close_button.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.dismiss();
            }
        });
        let weak = Rc::downgrade(&this);
        this.visibility.underlays().unwrap().on_dismiss(move || {
            if let Some(this) = weak.upgrade() {
                this.dismiss();
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&this);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape
                && let Some(this) = weak.upgrade().filter(|this| this.is_visible())
            {
                this.dismiss();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        this.window().add_controller(keys);
        let weak = Rc::downgrade(&this);
        this.visibility.main().on_closed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.shutdown();
            }
        });
        Ok(this)
    }
    pub fn window(&self) -> &adw::Window {
        self.visibility.main().window()
    }
    pub fn is_visible(&self) -> bool {
        self.visible.get()
    }
    pub fn monitor(&self) -> Option<gtk::gdk::Monitor> {
        self.monitor
            .borrow()
            .as_ref()
            .map(|(monitor, _)| monitor.clone())
    }
    pub fn underlays(&self) -> &UnderlaySet {
        self.visibility.underlays().unwrap()
    }
    pub fn on_event(&self, callback: impl Fn(PopupEvent) + 'static) {
        self.callbacks.borrow_mut().push(Rc::new(callback));
    }
    fn emit(&self, event: PopupEvent) {
        let callbacks = self.callbacks.borrow().clone();
        for callback in callbacks {
            callback(event);
        }
    }
    pub fn show(self: &Rc<Self>, monitor: &gtk::gdk::Monitor) -> Result<(), String> {
        if self.stopped.get() || self.visibility.main().is_closed() || !monitor.is_valid() {
            return Err("Shortcut surface or monitor is unavailable".into());
        }
        if self.is_visible() && self.monitor().as_ref() == Some(monitor) {
            return Ok(());
        }
        self.hide();
        if let Some((monitor, handler)) = self.monitor.borrow_mut().take() {
            monitor.disconnect(handler);
        }
        self.window().set_monitor(Some(monitor));
        let weak = Rc::downgrade(self);
        let handler = monitor.connect_invalidate(move |_| {
            if let Some(this) = weak.upgrade() {
                this.hide();
            }
        });
        self.monitor.replace(Some((monitor.clone(), handler)));
        let geometry = monitor.geometry();
        self.window().set_default_size(
            self.width.min((geometry.width() - 40).max(160)),
            self.height.min((geometry.height() - 80).max(160)),
        );
        self.window().set_keyboard_mode(KeyboardMode::Exclusive);
        self.visible.set(true);
        self.emit(PopupEvent::Visible);
        if self.stopped.get() || !self.visible.get() {
            return Ok(());
        }
        if let Some(transition) = self.visibility.begin_show() {
            self.visibility.finish_transition(&transition);
        }
        if self.visibility.main().is_closed() {
            self.hide();
            return Err("Could not create the shortcut surface".into());
        }
        self.close_button.grab_focus();
        Ok(())
    }
    pub fn hide(&self) {
        if !self.visible.replace(false) {
            return;
        }
        self.window().set_keyboard_mode(KeyboardMode::None);
        self.visibility.hide_now();
        self.emit(PopupEvent::Hidden);
    }
    pub fn toggle(self: &Rc<Self>, monitor: &gtk::gdk::Monitor) -> Result<(), String> {
        if self.is_visible() && self.monitor().as_ref() == Some(monitor) {
            self.hide();
            Ok(())
        } else {
            self.show(monitor)
        }
    }
    pub fn dismiss(&self) {
        if self.is_visible() {
            self.hide();
            self.emit(PopupEvent::Dismissed);
        }
    }
    pub fn shutdown(&self) {
        if self.stopped.replace(true) {
            return;
        }
        self.hide();
        if let Some((monitor, handler)) = self.monitor.borrow_mut().take() {
            monitor.disconnect(handler);
        }
        self.visibility.close();
        // Retaining a GTK widget must not retain live feature callbacks.
        self.callbacks.borrow_mut().clear();
        self.window().set_content(None::<&gtk::Widget>);
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
    }
}
impl Drop for PanelPopup {
    fn drop(&mut self) {
        self.shutdown();
    }
}
