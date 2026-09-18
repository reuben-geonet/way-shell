//! One notification popup with owned replacement, animation and timeout state.
use super::notification_card::{CardEvent, CardMode, NotificationCard};
use crate::{
    services::notifications::{Notification, NotificationEvent, NotificationsService},
    ui::window::{LayerWindow, WindowRole},
};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
    time::Duration,
};

enum Update {
    Event(Box<NotificationEvent>),
    Hide,
    Timeout(u64),
}
pub struct NotificationOsd {
    window: LayerWindow,
    content: gtk::Box,
    revealer: gtk::Revealer,
    motion: gtk::EventControllerMotion,
    service: NotificationsService,
    settings: gio::Settings,
    service_handler: RefCell<Option<glib::SignalHandlerId>>,
    card: RefCell<Option<Rc<NotificationCard>>>,
    timeout: RefCell<Option<glib::SourceId>>,
    generation: Cell<u64>,
    updates: RefCell<VecDeque<Update>>,
    updating: Cell<bool>,
    tray_visible: Cell<bool>,
    stopped: Cell<bool>,
}
impl NotificationOsd {
    pub fn new(service: NotificationsService, settings: gio::Settings) -> Result<Rc<Self>, String> {
        let window = LayerWindow::new(WindowRole::NotificationOsd, None)?;
        window.window().set_widget_name("notifications-osd");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.set_widget_name("notifications-osd-container");
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(350)
            .child(&content)
            .build();
        window.window().set_content(Some(&revealer));
        let motion = gtk::EventControllerMotion::new();
        window.window().add_controller(motion.clone());
        let this = Rc::new(Self {
            window,
            content,
            revealer,
            motion,
            service,
            settings,
            service_handler: RefCell::new(None),
            card: RefCell::new(None),
            timeout: RefCell::new(None),
            generation: Cell::new(0),
            updates: RefCell::new(VecDeque::new()),
            updating: Cell::new(false),
            tray_visible: Cell::new(false),
            stopped: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        this.revealer
            .connect_child_revealed_notify(move |revealer| {
                if let Some(this) = weak.upgrade()
                    && !revealer.is_child_revealed()
                    && !revealer.reveals_child()
                {
                    this.window.hide();
                }
            });
        let weak = Rc::downgrade(&this);
        this.window.on_closed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.stop();
            }
        });
        let weak = Rc::downgrade(&this);
        let handler = this.service.connect_local("event", false, move |values| {
            if let Some(this) = weak.upgrade() {
                this.update(Update::Event(Box::new(
                    values[1].get::<NotificationEvent>().unwrap(),
                )));
            }
            None
        });
        this.service_handler.replace(Some(handler));
        Ok(this)
    }
    pub fn window(&self) -> &adw::Window {
        self.window.window()
    }
    pub fn card(&self) -> Option<Rc<NotificationCard>> {
        self.card.borrow().clone()
    }
    pub fn is_visible(&self) -> bool {
        self.window.window().get_visible() && self.revealer.reveals_child()
    }
    pub fn tray_will_show(self: &Rc<Self>) {
        self.hide();
    }
    pub fn set_tray_visible(&self, visible: bool) {
        self.tray_visible.set(visible);
    }
    pub fn hide(self: &Rc<Self>) {
        self.update(Update::Hide);
    }
    pub fn stop(&self) {
        if self.stopped.replace(true) {
            return;
        }
        if let Some(handler) = self.service_handler.borrow_mut().take() {
            self.service.disconnect(handler);
        }
        self.updates.borrow_mut().clear();
        self.cancel_timeout();
        if let Some(card) = self.card() {
            card.stop();
        }
        self.window.close();
    }
    fn update(self: &Rc<Self>, update: Update) {
        if self.stopped.get() {
            return;
        }
        self.updates.borrow_mut().push_back(update);
        if self.updating.replace(true) {
            return;
        }
        loop {
            let update = self.updates.borrow_mut().pop_front();
            let Some(update) = update else {
                break;
            };
            if self.stopped.get() {
                break;
            }
            match update {
                Update::Event(event) => match *event {
                    NotificationEvent::Added { notification, .. } => {
                        if !self.tray_visible.get() && !self.settings.boolean("do-not-disturb") {
                            self.present(notification);
                        }
                    }
                    NotificationEvent::Replaced { notification, .. } => {
                        if let Some(card) = self.card().filter(|card| card.id() == notification.id)
                        {
                            card.set_notification(notification);
                            if self.is_visible()
                                && !self.tray_visible.get()
                                && !self.settings.boolean("do-not-disturb")
                            {
                                self.schedule_timeout();
                            }
                        }
                    }
                    NotificationEvent::Closed { notification, .. } => {
                        if self.card().is_some_and(|card| card.id() == notification.id) {
                            self.hide_now();
                        }
                    }
                },
                Update::Hide => self.hide_now(),
                Update::Timeout(generation) if self.generation.get() == generation => {
                    if self.motion.contains_pointer() {
                        self.schedule_timeout();
                    } else {
                        self.hide_now();
                    }
                }
                Update::Timeout(_) => {}
            }
        }
        self.updating.set(false);
    }
    fn present(self: &Rc<Self>, notification: Notification) {
        self.cancel_timeout();
        self.revealer.set_reveal_child(false);
        let previous = self.card.borrow_mut().take();
        if let Some(previous) = previous {
            previous.stop();
            self.content.remove(previous.widget());
        }
        let weak = Rc::downgrade(self);
        let card = NotificationCard::new(
            self.service.clone(),
            notification,
            CardMode::Osd,
            move |event| {
                if let Some(this) = weak.upgrade()
                    && !this.stopped.get()
                {
                    match event {
                        CardEvent::Hide => this.hide(),
                        CardEvent::Collapsed => this.window.window().set_default_size(440, 100),
                        CardEvent::Expanded => {}
                    }
                }
            },
        );
        self.card.replace(Some(card.clone()));
        self.content.append(card.widget());
        if self.stopped.get() {
            card.stop();
            return;
        }
        self.window.present();
        if !self.stopped.get() {
            self.revealer.set_reveal_child(true);
            self.schedule_timeout();
        }
    }
    fn hide_now(&self) {
        self.cancel_timeout();
        self.revealer.set_reveal_child(false);
        if !self.revealer.is_child_revealed() {
            self.window.hide();
        }
    }
    fn cancel_timeout(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        if let Some(timeout) = self.timeout.borrow_mut().take() {
            timeout.remove();
        }
    }
    fn schedule_timeout(self: &Rc<Self>) {
        self.cancel_timeout();
        let generation = self.generation.get();
        let weak = Rc::downgrade(self);
        self.timeout.replace(Some(glib::timeout_add_local_once(
            Duration::from_secs(8),
            move || {
                if let Some(this) = weak.upgrade()
                    && !this.stopped.get()
                    && this.generation.get() == generation
                {
                    this.timeout.borrow_mut().take();
                    this.update(Update::Timeout(generation));
                }
            },
        )));
    }
}
impl Drop for NotificationOsd {
    fn drop(&mut self) {
        self.stop();
    }
}
