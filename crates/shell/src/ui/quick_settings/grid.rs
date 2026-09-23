//! Two-column quick-settings tiles with one owned menu open at a time.
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::{Rc, Weak},
};

pub struct GridButton {
    root: gtk::Box,
    toggle: gtk::Button,
    title: gtk::Label,
    subtitle: gtk::Label,
    icon: gtk::Image,
    reveal_button: Option<gtk::Button>,
    revealer: gtk::Revealer,
    reveal_action: RefCell<Option<Rc<dyn Fn()>>>,
    reveal_changed: RefCell<Option<Rc<dyn Fn()>>>,
}

impl GridButton {
    pub fn new(
        title: &str,
        subtitle: Option<&str>,
        icon: &str,
        menu: Option<&gtk::Widget>,
    ) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.set_hexpand(true);
        root.set_size_request(100, 60);
        root.add_css_class("quick-settings-grid-button");
        let toggle = gtk::Button::new();
        toggle.add_css_class("quick-settings-grid-button-toggle");
        toggle.set_hexpand(true);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&toggle));
        root.append(&overlay);
        let contents = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        contents.set_valign(gtk::Align::Center);
        let icon = gtk::Image::from_icon_name(icon);
        icon.set_pixel_size(20);
        icon.set_halign(gtk::Align::Start);
        icon.add_css_class("quick-settings-grid-button-icon");
        contents.append(&icon);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let title = tile_label(title, "quick-settings-grid-button-title");
        let subtitle_widget = tile_label(
            subtitle.unwrap_or(""),
            "quick-settings-grid-button-subtitle",
        );
        subtitle_widget.set_visible(subtitle.is_some());
        if subtitle.is_some() {
            toggle.add_css_class("with-subtitle");
        } else {
            title.set_vexpand(true);
            title.set_valign(gtk::Align::Center);
        }
        text.append(&title);
        text.append(&subtitle_widget);
        contents.append(&text);
        toggle.set_child(Some(&contents));
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(350)
            .build();
        revealer.add_css_class("quick-settings-grid-button-revealer");
        revealer.set_child(menu);
        let reveal_button = menu.map(|_| {
            toggle.add_css_class("with-revealer");
            let button = gtk::Button::from_icon_name("go-next-symbolic");
            button.add_css_class("quick-settings-grid-button-reveal-hidden");
            button.add_css_class("quick-settings-grid-button-reveal-visible");
            button.set_halign(gtk::Align::End);
            overlay.add_overlay(&button);
            button
        });
        let this = Rc::new(Self {
            root,
            toggle,
            title,
            subtitle: subtitle_widget,
            icon,
            reveal_button,
            revealer,
            reveal_action: RefCell::new(None),
            reveal_changed: RefCell::new(None),
        });
        let weak = Rc::downgrade(&this);
        this.revealer.connect_reveal_child_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                let action = this.reveal_changed.borrow().clone();
                if let Some(action) = action {
                    action();
                }
            }
        });
        if let Some(button) = &this.reveal_button {
            let weak = Rc::downgrade(&this);
            button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.toggle_menu();
                }
            });
        }
        this
    }
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }
    pub fn toggle(&self) -> &gtk::Button {
        &self.toggle
    }
    pub fn reveal_button(&self) -> Option<&gtk::Button> {
        self.reveal_button.as_ref()
    }
    pub fn revealer(&self) -> &gtk::Revealer {
        &self.revealer
    }
    pub fn set_title(&self, title: &str) {
        self.title.set_label(title);
    }
    pub fn set_subtitle(&self, subtitle: &str) {
        self.subtitle.set_label(subtitle);
    }
    pub fn set_optional_subtitle(&self, subtitle: Option<&str>) {
        self.subtitle.set_label(subtitle.unwrap_or(""));
        self.subtitle.set_visible(subtitle.is_some());
        self.title.set_vexpand(false);
        if let Some(text) = self.title.parent() {
            text.set_valign(gtk::Align::Center);
        }
        if subtitle.is_some() {
            self.toggle.add_css_class("with-subtitle");
        } else {
            self.toggle.remove_css_class("with-subtitle");
        }
    }
    pub fn set_icon(&self, icon: &str) {
        self.icon.set_icon_name(Some(icon));
    }
    pub fn set_sensitive(&self, sensitive: bool) {
        self.root.set_sensitive(sensitive);
        if !sensitive {
            self.revealer.set_reveal_child(false);
        }
    }
    pub fn set_toggled(&self, toggled: bool) {
        for widget in std::iter::once(&self.toggle).chain(self.reveal_button.iter()) {
            if toggled {
                widget.remove_css_class("off");
            } else {
                widget.add_css_class("off");
            }
        }
    }
    pub fn toggle_menu(&self) {
        if !self.root.is_sensitive() || self.reveal_button.is_none() {
            return;
        }
        let action = self.reveal_action.borrow().clone();
        if let Some(action) = action {
            action();
        } else {
            self.revealer
                .set_reveal_child(!self.revealer.reveals_child());
        }
    }
}

fn tile_label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_width_chars(12);
    label.set_max_width_chars(12);
    label.set_xalign(0.0);
    label.set_halign(gtk::Align::Start);
    label.add_css_class(class);
    label
}

struct Row {
    root: gtk::Box,
    center: gtk::CenterBox,
}
enum Update {
    Buttons(Vec<Rc<GridButton>>),
    Toggle(Weak<GridButton>),
    Hide,
    RefreshFocus,
}

pub struct Grid {
    root: gtk::Box,
    buttons: RefCell<Vec<Rc<GridButton>>>,
    rows: RefCell<Vec<Row>>,
    updates: RefCell<VecDeque<Update>>,
    updating: Cell<bool>,
    stopped: Cell<bool>,
    focused: Box<dyn Fn(bool)>,
    focus_state: Cell<bool>,
}

impl Grid {
    pub fn new(focused: impl Fn(bool) + 'static) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_widget_name("quick-settings-grid");
        root.set_hexpand(true);
        root.set_vexpand(true);
        Rc::new(Self {
            root,
            buttons: RefCell::new(Vec::new()),
            rows: RefCell::new(Vec::new()),
            updates: RefCell::new(VecDeque::new()),
            updating: Cell::new(false),
            stopped: Cell::new(false),
            focused: Box::new(focused),
            focus_state: Cell::new(false),
        })
    }
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }
    /// Supply the complete order, including dynamic network tiles. Existing
    /// button/menu objects survive moves and reconnection.
    pub fn set_buttons(self: &Rc<Self>, buttons: Vec<Rc<GridButton>>) {
        self.update(Update::Buttons(buttons));
    }
    pub fn hide_menus(self: &Rc<Self>) {
        self.update(Update::Hide);
    }
    pub fn stop(&self) {
        self.stopped.set(true);
        self.updates.borrow_mut().clear();
        let buttons = self.buttons.borrow().clone();
        for button in buttons {
            button.revealer.set_reveal_child(false);
        }
        self.root.set_sensitive(false);
        self.publish_focus(false);
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
            let next = self.updates.borrow_mut().pop_front();
            let Some(next) = next else {
                break;
            };
            if self.stopped.get() {
                self.updates.borrow_mut().clear();
                break;
            }
            match next {
                Update::Buttons(buttons) => self.rebuild(buttons),
                Update::Toggle(button) => {
                    if let Some(button) = button.upgrade() {
                        let buttons = self.buttons.borrow().clone();
                        if !buttons.iter().any(|value| Rc::ptr_eq(value, &button)) {
                            continue;
                        }
                        let reveal = !button.revealer.reveals_child();
                        for value in buttons {
                            value.revealer.set_reveal_child(
                                !self.stopped.get() && reveal && Rc::ptr_eq(&value, &button),
                            );
                        }
                        self.publish_focus(!self.stopped.get() && reveal);
                    }
                }
                Update::Hide => {
                    let buttons = self.buttons.borrow().clone();
                    for button in buttons {
                        button.revealer.set_reveal_child(false);
                    }
                    self.publish_focus(false);
                }
                Update::RefreshFocus => {
                    let focused = self
                        .buttons
                        .borrow()
                        .iter()
                        .any(|button| button.revealer.reveals_child());
                    self.publish_focus(!self.stopped.get() && focused);
                }
            }
        }
        self.updating.set(false);
    }
    fn rebuild(self: &Rc<Self>, buttons: Vec<Rc<GridButton>>) {
        let mut unique = Vec::with_capacity(buttons.len());
        for button in buttons {
            if !unique.iter().any(|old| Rc::ptr_eq(old, &button)) {
                unique.push(button);
            }
        }
        let unchanged = {
            let current = self.buttons.borrow();
            current.len() == unique.len()
                && current
                    .iter()
                    .zip(&unique)
                    .all(|(old, new)| Rc::ptr_eq(old, new))
        };
        if unchanged {
            return;
        }
        let focused = self
            .root
            .root()
            .and_then(|root| root.focus())
            .filter(|widget| widget.is_ancestor(&self.root));
        let old_buttons = self.buttons.replace(unique.clone());
        for button in old_buttons {
            if !unique.iter().any(|new| Rc::ptr_eq(new, &button)) {
                button.revealer.set_reveal_child(false);
                button.reveal_action.take();
                button.reveal_changed.take();
            }
        }
        let old_rows = self.rows.take();
        for row in old_rows {
            row.center.set_start_widget(None::<&gtk::Widget>);
            row.center.set_end_widget(None::<&gtk::Widget>);
            while let Some(child) = row
                .root
                .last_child()
                .filter(|child| child != row.center.upcast_ref::<gtk::Widget>())
            {
                row.root.remove(&child);
            }
            self.root.remove(&row.root);
        }
        let mut rows = Vec::new();
        for pair in unique.chunks(2) {
            let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let center = gtk::CenterBox::new();
            root.append(&center);
            center.set_start_widget(Some(pair[0].widget()));
            if let Some(right) = pair.get(1) {
                center.set_end_widget(Some(right.widget()));
            } else {
                let dummy = GridButton::new("dummy", Some("dummy"), "image-missing", None);
                dummy
                    .widget()
                    .add_css_class("quick-settings-grid-button-transparent");
                dummy.widget().set_sensitive(false);
                center.set_end_widget(Some(dummy.widget()));
            }
            for button in pair {
                root.append(button.revealer());
                let weak_grid = Rc::downgrade(self);
                let weak_button = Rc::downgrade(button);
                button.reveal_action.replace(Some(Rc::new(move || {
                    if let Some(grid) = weak_grid.upgrade() {
                        grid.update(Update::Toggle(weak_button.clone()));
                    }
                })));
                let weak_grid = Rc::downgrade(self);
                button.reveal_changed.replace(Some(Rc::new(move || {
                    if let Some(grid) = weak_grid.upgrade() {
                        grid.update(Update::RefreshFocus);
                    }
                })));
            }
            self.root.append(&root);
            rows.push(Row { root, center });
        }
        self.rows.replace(rows);
        if let Some(widget) = focused.filter(|widget| widget.is_ancestor(&self.root)) {
            widget.grab_focus();
        }
        self.publish_focus(
            !self.stopped.get() && unique.iter().any(|button| button.revealer.reveals_child()),
        );
    }
    fn publish_focus(&self, focused: bool) {
        if self.focus_state.replace(focused) != focused {
            (self.focused)(focused);
        }
    }
}
impl Drop for Grid {
    fn drop(&mut self) {
        self.stop();
    }
}
