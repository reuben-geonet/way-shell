//! Notification inventory, grouping, DND, and the media prefix owned by the tray.
use super::{
    notification_card::widget_children,
    notification_group::{GroupEvent, NotificationGroup},
};
use crate::services::notifications::{
    CloseReason, Notification, NotificationEvent, NotificationsService,
};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};

enum Update {
    Event(NotificationEvent),
    Seed(Notification),
    Media(Vec<gtk::Widget>),
    Collapse,
    Hidden,
    PruneEmpty,
}
pub struct NotificationsList {
    root: gtk::Box,
    list: gtk::Box,
    scroll: gtk::ScrolledWindow,
    status: adw::StatusPage,
    clear: gtk::Button,
    dnd: adw::SwitchRow,
    service: NotificationsService,
    settings: gio::Settings,
    service_handler: RefCell<Option<glib::SignalHandlerId>>,
    settings_handler: RefCell<Option<glib::SignalHandlerId>>,
    groups: RefCell<Vec<Rc<NotificationGroup>>>,
    media: RefCell<Vec<gtk::Widget>>,
    updates: RefCell<VecDeque<Update>>,
    updating: Cell<bool>,
    changing_dnd: Cell<bool>,
    stopped: Cell<bool>,
    shrink: Box<dyn Fn()>,
}
impl NotificationsList {
    pub fn new(
        service: NotificationsService,
        settings: gio::Settings,
        shrink: impl Fn() + 'static,
    ) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_widget_name("notifications-list");
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.set_widget_name("notifications-list-container");
        root.append(&container);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Never);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.set_widget_name("notifications-list-list");
        list.set_vexpand(true);
        scroll.set_child(Some(&list));
        let status = adw::StatusPage::builder()
            .icon_name("notifications-disabled-symbolic")
            .title("No Notifications")
            .vexpand(true)
            .build();
        container.append(&status);
        container.append(&scroll);
        let controls = gtk::CenterBox::new();
        controls.set_widget_name("notifications-list-controls");
        let dnd = adw::SwitchRow::new();
        dnd.set_title("Do Not Disturb");
        dnd.set_size_request(200, -1);
        dnd.add_css_class("notifications-list-dnd");
        controls.set_start_widget(Some(&dnd));
        let clear = gtk::Button::with_label("Clear");
        clear.add_css_class("notifications-list-clear");
        controls.set_end_widget(Some(&clear));
        container.append(&controls);
        let this = Rc::new(Self {
            root,
            list,
            scroll,
            status,
            clear,
            dnd,
            service,
            settings,
            service_handler: RefCell::new(None),
            settings_handler: RefCell::new(None),
            groups: RefCell::new(Vec::new()),
            media: RefCell::new(Vec::new()),
            updates: RefCell::new(VecDeque::new()),
            updating: Cell::new(false),
            changing_dnd: Cell::new(false),
            stopped: Cell::new(false),
            shrink: Box::new(shrink),
        });
        let weak = Rc::downgrade(&this);
        this.clear.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade()
                && !this.stopped.get()
            {
                for notification in this.service.state().notifications {
                    this.service.close(notification.id, CloseReason::Dismissed);
                }
            }
        });
        let weak = Rc::downgrade(&this);
        this.dnd.connect_active_notify(move |switch| {
            if let Some(this) = weak.upgrade()
                && !this.stopped.get()
                && !this.changing_dnd.get()
            {
                let _ = this
                    .settings
                    .set_boolean("do-not-disturb", switch.is_active());
            }
        });
        let weak = Rc::downgrade(&this);
        let handler = this
            .settings
            .connect_changed(Some("do-not-disturb"), move |_, _| {
                if let Some(this) = weak.upgrade() {
                    this.refresh_dnd();
                }
            });
        this.settings_handler.replace(Some(handler));
        this.refresh_dnd();
        let weak = Rc::downgrade(&this);
        let handler = this.service.connect_local("event", false, move |values| {
            if let Some(this) = weak.upgrade() {
                this.update(Update::Event(values[1].get::<NotificationEvent>().unwrap()));
            }
            None
        });
        this.service_handler.replace(Some(handler));
        for notification in this.service.state().notifications {
            this.update(Update::Seed(notification));
        }
        this.rebuild();
        this
    }
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }
    pub fn clear_button(&self) -> &gtk::Button {
        &self.clear
    }
    pub fn dnd_switch(&self) -> &adw::SwitchRow {
        &self.dnd
    }
    pub fn is_dnd(&self) -> bool {
        self.settings.boolean("do-not-disturb")
    }
    pub fn groups(&self) -> Vec<Rc<NotificationGroup>> {
        self.groups.borrow().clone()
    }
    pub fn group(&self, app: &str) -> Option<Rc<NotificationGroup>> {
        self.groups
            .borrow()
            .iter()
            .find(|group| group.app() == app)
            .cloned()
    }
    pub fn is_empty(&self) -> bool {
        self.groups.borrow().is_empty() && self.media.borrow().is_empty()
    }
    pub fn set_media_widgets(self: &Rc<Self>, widgets: Vec<gtk::Widget>) {
        self.update(Update::Media(widgets));
    }
    pub fn collapse(self: &Rc<Self>) {
        self.update(Update::Collapse);
    }
    pub fn hidden(self: &Rc<Self>) {
        self.update(Update::Hidden);
    }
    pub fn stop(&self) {
        if self.stopped.replace(true) {
            return;
        }
        if let Some(handler) = self.service_handler.borrow_mut().take() {
            self.service.disconnect(handler);
        }
        if let Some(handler) = self.settings_handler.borrow_mut().take() {
            self.settings.disconnect(handler);
        }
        self.updates.borrow_mut().clear();
        for group in self.groups() {
            group.stop();
        }
        self.root.set_sensitive(false);
    }
    fn refresh_dnd(&self) {
        if self.stopped.get() {
            return;
        }
        self.changing_dnd.set(true);
        self.dnd.set_active(self.settings.boolean("do-not-disturb"));
        self.dnd
            .set_sensitive(self.settings.is_writable("do-not-disturb"));
        self.changing_dnd.set(false);
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
            let mut shrink = false;
            match update {
                Update::Seed(notification)
                | Update::Event(NotificationEvent::Added { notification, .. }) => {
                    self.add(notification)
                }
                Update::Event(NotificationEvent::Replaced {
                    notification,
                    previous,
                    ..
                }) => {
                    if previous.request.app_name != notification.request.app_name {
                        self.remove(&previous.request.app_name, notification.id);
                        shrink = true;
                    }
                    self.add(notification);
                }
                Update::Event(NotificationEvent::Closed { notification, .. }) => {
                    self.remove(&notification.request.app_name, notification.id);
                    shrink = true;
                }
                Update::Media(widgets) => {
                    let mut unique = Vec::new();
                    for widget in widgets {
                        if !unique.contains(&widget) {
                            unique.push(widget);
                        }
                    }
                    self.media.replace(unique);
                    shrink = true;
                }
                Update::Collapse => {
                    for group in self.groups() {
                        group.collapse();
                    }
                    shrink = true;
                }
                Update::Hidden => {
                    shrink = true;
                }
                Update::PruneEmpty => {
                    let groups = self.groups();
                    for group in groups.iter().filter(|group| group.is_empty()) {
                        group.stop();
                    }
                    self.groups.borrow_mut().retain(|group| !group.is_empty());
                    shrink = true;
                }
            }
            self.rebuild();
            self.resize(shrink);
        }
        self.updating.set(false);
    }
    fn add(self: &Rc<Self>, notification: Notification) {
        let app = &notification.request.app_name;
        let group = self.group(app).unwrap_or_else(|| {
            let weak = Rc::downgrade(self);
            let group = NotificationGroup::new(self.service.clone(), app.clone(), move |event| {
                if let Some(this) = weak.upgrade()
                    && !this.stopped.get()
                {
                    match event {
                        GroupEvent::WillExpand => {
                            this.scroll.set_size_request(-1, -1);
                            this.scroll
                                .set_policy(gtk::PolicyType::Never, gtk::PolicyType::Always);
                        }
                        GroupEvent::Resized { shrink } => this.resize(shrink),
                        GroupEvent::Empty => this.update(Update::PruneEmpty),
                    }
                }
            });
            self.groups.borrow_mut().insert(0, group.clone());
            group
        });
        group.upsert(notification);
    }
    fn remove(&self, app: &str, id: u32) {
        if let Some(group) = self.group(app) {
            group.remove(id);
            if group.is_empty() {
                group.stop();
                self.groups
                    .borrow_mut()
                    .retain(|current| !Rc::ptr_eq(current, &group));
            }
        }
    }
    fn rebuild(&self) {
        let groups = self.groups();
        let media = self.media.borrow().clone();
        let desired: Vec<_> = media
            .iter()
            .cloned()
            .chain(
                groups
                    .iter()
                    .map(|group| group.widget().clone().upcast::<gtk::Widget>()),
            )
            .collect();
        for child in widget_children(&self.list) {
            if !desired.contains(&child) {
                self.list.remove(&child);
            }
        }
        let mut previous: Option<&gtk::Widget> = None;
        for widget in &desired {
            if widget.parent().is_none() {
                self.list.append(widget);
            }
            self.list.reorder_child_after(widget, previous);
            previous = Some(widget);
        }
        let empty = groups.is_empty() && media.is_empty();
        self.status.set_visible(empty);
        self.scroll.set_visible(!empty);
    }
    fn resize(&self, shrink: bool) {
        if self.stopped.get() {
            return;
        }
        if shrink {
            (self.shrink)();
        }
        let (_, natural, _, _) = self.list.measure(gtk::Orientation::Vertical, -1);
        self.scroll
            .set_size_request(-1, if natural >= 900 { 1080 } else { -1 });
        self.scroll.set_policy(
            gtk::PolicyType::Never,
            if natural >= 900 {
                gtk::PolicyType::Always
            } else {
                gtk::PolicyType::Never
            },
        );
    }
}
impl Drop for NotificationsList {
    fn drop(&mut self) {
        self.stop();
    }
}
