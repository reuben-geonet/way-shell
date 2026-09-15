//! A stable application group with the newest notification above older cards.
use super::notification_card::{CardEvent, CardMode, NotificationCard, widget_children};
use crate::services::notifications::{CloseReason, Notification, NotificationsService};
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupEvent {
    WillExpand,
    Empty,
    Resized { shrink: bool },
}
enum Update {
    Upsert(Box<Notification>),
    Remove(u32),
    Expanded(bool),
}
pub struct NotificationGroup {
    root: gtk::Box,
    app: String,
    service: NotificationsService,
    header_revealer: gtk::Revealer,
    older_revealer: gtk::Revealer,
    head: gtk::Box,
    older: gtk::Box,
    overlay_button: gtk::Button,
    conceal: gtk::Button,
    dismiss: gtk::Button,
    cards: RefCell<Vec<Rc<NotificationCard>>>,
    updates: RefCell<VecDeque<Update>>,
    updating: Cell<bool>,
    expanded: Cell<bool>,
    stopped: Cell<bool>,
    event: Box<dyn Fn(GroupEvent)>,
}
impl NotificationGroup {
    pub fn new(
        service: NotificationsService,
        app: String,
        event: impl Fn(GroupEvent) + 'static,
    ) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let header = gtk::CenterBox::new();
        header.add_css_class("notification-group-list-header");
        let title = gtk::Label::new(Some(&app));
        title.add_css_class("notification-group-app-name");
        header.set_start_widget(Some(&title));
        let conceal = gtk::Button::from_icon_name("view-restore-symbolic");
        conceal.add_css_class("circular");
        conceal.add_css_class("notification-group-conceal-button");
        let dismiss = gtk::Button::from_icon_name("window-close-symbolic");
        dismiss.add_css_class("circular");
        dismiss.add_css_class("notification-group-dismiss-button");
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        buttons.append(&conceal);
        buttons.append(&dismiss);
        header.set_end_widget(Some(&buttons));
        let header_revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .child(&header)
            .build();
        let head = gtk::Box::new(gtk::Orientation::Vertical, 0);
        head.add_css_class("notification-group-head-notification-container");
        let older = gtk::Box::new(gtk::Orientation::Vertical, 0);
        older.add_css_class("notification-group-notification-list");
        let older_revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .child(&older)
            .build();
        let overlay = gtk::Overlay::new();
        let overlay_button = gtk::Button::new();
        overlay_button.add_css_class("notification-group-overlay-expand-button");
        overlay_button.set_visible(false);
        overlay.set_child(Some(&head));
        overlay.add_overlay(&overlay_button);
        root.append(&overlay);
        let this = Rc::new(Self {
            root,
            app,
            service,
            header_revealer,
            older_revealer,
            head,
            older,
            overlay_button,
            conceal,
            dismiss,
            cards: RefCell::new(Vec::new()),
            updates: RefCell::new(VecDeque::new()),
            updating: Cell::new(false),
            expanded: Cell::new(false),
            stopped: Cell::new(false),
            event: Box::new(event),
        });
        for button in [&this.overlay_button, &this.conceal] {
            let weak = Rc::downgrade(&this);
            button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.update(Update::Expanded(!this.expanded.get()));
                }
            });
        }
        let weak = Rc::downgrade(&this);
        this.dismiss.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.dismiss_all();
            }
        });
        let weak = Rc::downgrade(&this);
        this.header_revealer
            .connect_child_revealed_notify(move |revealer| {
                if let Some(this) = weak.upgrade()
                    && !this.stopped.get()
                {
                    (this.event)(GroupEvent::Resized {
                        shrink: !revealer.is_child_revealed(),
                    });
                }
            });
        this
    }
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }
    pub fn app(&self) -> &str {
        &self.app
    }
    pub fn cards(&self) -> Vec<Rc<NotificationCard>> {
        self.cards.borrow().clone()
    }
    pub fn card(&self, id: u32) -> Option<Rc<NotificationCard>> {
        self.cards
            .borrow()
            .iter()
            .find(|card| card.id() == id)
            .cloned()
    }
    pub fn is_empty(&self) -> bool {
        self.cards.borrow().is_empty()
    }
    pub fn is_expanded(&self) -> bool {
        self.expanded.get()
    }
    pub fn expand_button(&self) -> &gtk::Button {
        &self.overlay_button
    }
    pub fn dismiss_button(&self) -> &gtk::Button {
        &self.dismiss
    }
    pub fn upsert(self: &Rc<Self>, notification: Notification) {
        self.update(Update::Upsert(Box::new(notification)));
    }
    pub fn remove(self: &Rc<Self>, id: u32) {
        self.update(Update::Remove(id));
    }
    pub fn collapse(self: &Rc<Self>) {
        self.update(Update::Expanded(false));
        for card in self.cards() {
            card.collapse();
        }
    }
    pub fn dismiss_all(&self) {
        if self.stopped.get() {
            return;
        }
        for card in self.cards() {
            self.service.close(card.id(), CloseReason::Dismissed);
        }
    }
    pub fn stop(&self) {
        if self.stopped.replace(true) {
            return;
        }
        self.updates.borrow_mut().clear();
        for card in self.cards() {
            card.stop();
        }
        self.root.set_sensitive(false);
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
                Update::Upsert(notification) => {
                    let notification = *notification;
                    if let Some(card) = self.card(notification.id) {
                        card.set_notification(notification);
                    } else {
                        let weak = Rc::downgrade(self);
                        let card = NotificationCard::new(
                            self.service.clone(),
                            notification,
                            CardMode::Tray,
                            move |event| {
                                if let Some(this) = weak.upgrade()
                                    && !this.stopped.get()
                                {
                                    (this.event)(GroupEvent::Resized {
                                        shrink: event == CardEvent::Collapsed,
                                    });
                                }
                            },
                        );
                        self.cards.borrow_mut().insert(0, card);
                    }
                    self.rebuild();
                }
                Update::Remove(id) => {
                    let index = self.cards.borrow().iter().position(|card| card.id() == id);
                    if let Some(index) = index {
                        let card = self.cards.borrow_mut().remove(index);
                        card.stop();
                    }
                    shrink = true;
                    self.rebuild();
                }
                Update::Expanded(expanded) => {
                    let expanded = expanded && self.cards.borrow().len() > 1;
                    if self.expanded.replace(expanded) == expanded {
                        continue;
                    }
                    if expanded {
                        (self.event)(GroupEvent::WillExpand);
                    }
                    shrink = !expanded;
                    self.apply_expansion();
                }
            }
            if !self.stopped.get() {
                (self.event)(if self.is_empty() {
                    GroupEvent::Empty
                } else {
                    GroupEvent::Resized { shrink }
                });
            }
        }
        self.updating.set(false);
    }
    fn rebuild(&self) {
        let cards = self.cards();
        let mut head = vec![self.header_revealer.clone().upcast::<gtk::Widget>()];
        if let Some(card) = cards.first() {
            head.push(card.widget().clone().upcast());
        }
        head.push(self.older_revealer.clone().upcast());
        let older: Vec<_> = cards
            .iter()
            .skip(1)
            .map(|card| card.widget().clone().upcast::<gtk::Widget>())
            .collect();
        for child in widget_children(&self.head) {
            if !head.contains(&child) {
                self.head.remove(&child);
            }
        }
        for child in widget_children(&self.older) {
            if !older.contains(&child) {
                self.older.remove(&child);
            }
        }
        for (container, children) in [(&self.head, &head), (&self.older, &older)] {
            let mut previous: Option<&gtk::Widget> = None;
            for child in children {
                if child.parent().is_none() {
                    container.append(child);
                }
                container.reorder_child_after(child, previous);
                previous = Some(child);
            }
        }
        for card in cards.iter().skip(1) {
            card.set_stack(false);
        }
        self.apply_expansion();
    }
    fn apply_expansion(&self) {
        let cards = self.cards();
        let stacked = cards.len() > 1;
        if !stacked {
            self.expanded.set(false);
        }
        let expanded = stacked && self.expanded.get() && !self.stopped.get();
        self.header_revealer.set_reveal_child(expanded);
        self.older_revealer.set_reveal_child(expanded);
        self.overlay_button.set_visible(stacked && !expanded);
        if let Some(head) = cards.first() {
            head.set_stack(stacked && !expanded);
        }
    }
}
impl Drop for NotificationGroup {
    fn drop(&mut self) {
        self.stop();
    }
}
