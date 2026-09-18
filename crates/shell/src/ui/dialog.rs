//! Full-output confirmation with one owned, exactly-once response callback.
use super::window::{LayerWindow, WindowRole};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

type Response = Box<dyn FnOnce(bool)>;

pub struct Dialog {
    surface: LayerWindow,
    heading: gtk::Label,
    body: gtk::Label,
    confirm: gtk::Button,
    cancel: gtk::Button,
    pending: RefCell<Option<Response>>,
    revision: Cell<u64>,
    closed: Cell<bool>,
}

impl Dialog {
    pub fn new() -> Result<Rc<Self>, String> {
        let surface = LayerWindow::new(WindowRole::Dialog, None)?;
        surface.window().set_widget_name("dialog-overlay");
        let contents = gtk::Box::new(gtk::Orientation::Vertical, 0);
        contents.set_widget_name("dialog-overlay-container");
        contents.set_halign(gtk::Align::Center);
        contents.set_valign(gtk::Align::Center);
        contents.set_size_request(420, 200);
        let heading = gtk::Label::new(None);
        heading.add_css_class("dialog-overlay-heading");
        heading.set_halign(gtk::Align::Center);
        heading.set_valign(gtk::Align::Center);
        heading.set_xalign(0.5);
        let body = gtk::Label::new(None);
        body.add_css_class("dialog-overlay-body");
        body.set_halign(gtk::Align::Center);
        body.set_valign(gtk::Align::Center);
        body.set_xalign(0.5);
        let buttons = gtk::CenterBox::new();
        buttons.set_hexpand(true);
        buttons.set_vexpand(true);
        let confirm = gtk::Button::with_label("Confirm");
        confirm.add_css_class("dialog-overlay-confirm");
        confirm.set_hexpand(true);
        let cancel = gtk::Button::with_label("Cancel");
        cancel.add_css_class("dialog-overlay-cancel");
        cancel.set_hexpand(true);
        buttons.set_start_widget(Some(&cancel));
        buttons.set_end_widget(Some(&confirm));
        contents.append(&heading);
        contents.append(&body);
        contents.append(&buttons);
        surface.window().set_content(Some(&contents));
        let dialog = Rc::new(Self {
            surface,
            heading,
            body,
            confirm,
            cancel,
            pending: RefCell::new(None),
            revision: Cell::new(0),
            closed: Cell::new(false),
        });
        let weak = Rc::downgrade(&dialog);
        dialog.confirm.connect_clicked(move |_| {
            if let Some(dialog) = weak.upgrade() {
                dialog.respond(true);
            }
        });
        let weak = Rc::downgrade(&dialog);
        dialog.cancel.connect_clicked(move |_| {
            if let Some(dialog) = weak.upgrade() {
                dialog.cancel();
            }
        });
        let weak = Rc::downgrade(&dialog);
        dialog.surface.on_closed(move |_| {
            if let Some(dialog) = weak.upgrade() {
                dialog.close();
            }
        });
        Ok(dialog)
    }

    /// Replacement cancels the former request. If that callback presents a
    /// newer request, it wins and this superseded callback also receives false.
    /// Capture the dialog and other application owners weakly in callbacks.
    pub fn present(
        self: &Rc<Self>,
        heading: &str,
        body: &str,
        response: impl FnOnce(bool) + 'static,
    ) {
        if self.closed.get() {
            response(false);
            return;
        }
        let revision = self.next_revision();
        let previous = self.pending.borrow_mut().take();
        if let Some(previous) = previous {
            previous(false);
        }
        if !self.current(revision) {
            response(false);
            return;
        }
        self.pending.replace(Some(Box::new(response)));
        self.heading.set_label(heading);
        if !self.current(revision) {
            return;
        }
        self.body.set_label(body);
        if !self.current(revision) {
            return;
        }
        if !self.surface.present() {
            self.close();
            return;
        }
        if self.current(revision) {
            self.cancel.grab_focus();
        }
    }

    pub fn cancel(&self) {
        self.respond(false);
    }

    pub fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.next_revision();
        let pending = self.pending.borrow_mut().take();
        self.surface.close();
        if let Some(pending) = pending {
            pending(false);
        }
    }

    pub fn window(&self) -> &adw::Window {
        self.surface.window()
    }

    fn next_revision(&self) -> u64 {
        let revision = self.revision.get().wrapping_add(1);
        self.revision.set(revision);
        revision
    }
    fn current(&self, revision: u64) -> bool {
        !self.closed.get() && self.revision.get() == revision
    }
    fn respond(&self, accepted: bool) {
        let pending = self.pending.borrow_mut().take();
        let Some(pending) = pending else {
            return;
        };
        self.next_revision();
        // Hide before invoking application code; a response may open the next
        // confirmation, which must remain visible after this callback returns.
        self.surface.hide();
        pending(accepted);
    }
}
impl Drop for Dialog {
    fn drop(&mut self) {
        self.closed.set(true);
        let pending = self.pending.get_mut().take();
        self.surface.close();
        if let Some(pending) = pending {
            pending(false);
        }
    }
}
