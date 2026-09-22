//! Battery, mixer and session menus share the original header reveal policy.
use super::{
    battery::{BatteryPresentation, WarningTracker},
    controls::Subscription,
    menu::Menu,
    power_menu::{Confirmation, PowerMenu},
};
use crate::services::{
    logind::LogindService, notifications::NotificationsService, power::PowerService,
};
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub struct MixerSlot {
    pub button: gtk::Button,
    pub content: gtk::Widget,
    pub revealed: Rc<dyn Fn(bool)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderMenu {
    Power,
    Mixer,
    Battery,
}
pub struct SystemHeader {
    root: gtk::Box,
    power: PowerService,
    notifications: NotificationsService,
    battery_button: gtk::Button,
    battery_icon: gtk::Image,
    battery_label: gtk::Label,
    battery_menu: Menu,
    battery_scale: gtk::Scale,
    battery_time: gtk::Label,
    battery_percent: gtk::Label,
    power_button: gtk::Button,
    power_menu: Rc<PowerMenu>,
    mixer: Option<MixerSlot>,
    revealers: [gtk::Revealer; 3],
    subscriptions: RefCell<Vec<Subscription>>,
    warnings: RefCell<WarningTracker>,
    selected: Cell<Option<HeaderMenu>>,
    pending_menu: Cell<Option<Option<HeaderMenu>>>,
    revealing: Cell<bool>,
    refreshing: Cell<bool>,
    refresh_pending: Cell<bool>,
    stopped: Cell<bool>,
    focused: Box<dyn Fn(bool)>,
}
impl SystemHeader {
    pub fn new(
        power: PowerService,
        notifications: NotificationsService,
        logind: LogindService,
        system_settings: gio::Settings,
        mixer: Option<MixerSlot>,
        focused: impl Fn(bool) + 'static,
        confirmation: impl Fn(Confirmation) + 'static,
    ) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_widget_name("quick-settings-header-container");
        let center = gtk::CenterBox::new();
        center.set_widget_name("quick-settings-header");
        root.append(&center);
        let battery_button = gtk::Button::new();
        battery_button.add_css_class("battery-button");
        let contents = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        contents.set_widget_name("quick-settings-header-bat-button");
        let battery_icon = gtk::Image::from_icon_name("battery-missing-symbolic");
        let battery_label = gtk::Label::new(Some("—"));
        contents.append(&battery_icon);
        contents.append(&battery_label);
        battery_button.set_child(Some(&contents));
        center.set_start_widget(Some(&battery_button));
        let end = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        if let Some(mixer) = &mixer {
            end.append(&mixer.button);
        }
        let power_button = gtk::Button::from_icon_name("system-shutdown-symbolic");
        power_button.add_css_class("circular");
        end.append(&power_button);
        center.set_end_widget(Some(&end));
        let battery_menu = Menu::new("Battery", "battery-full-symbolic");
        let battery_contents = gtk::Box::new(gtk::Orientation::Vertical, 0);
        battery_contents.set_widget_name("battery-menu");
        let battery_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
        battery_scale.set_sensitive(false);
        battery_contents.append(&battery_scale);
        let stats = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let battery_time = gtk::Label::new(None);
        battery_time.set_xalign(0.0);
        battery_time.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let battery_percent = gtk::Label::new(None);
        battery_percent.set_xalign(1.0);
        stats.append(&battery_time);
        stats.append(&battery_percent);
        battery_contents.append(&stats);
        battery_menu.options().append(&battery_contents);
        let power_menu = PowerMenu::new(logind, system_settings, confirmation);
        let revealers = std::array::from_fn(|_| {
            gtk::Revealer::builder()
                .transition_type(gtk::RevealerTransitionType::SlideDown)
                .transition_duration(250)
                .build()
        });
        revealers[HeaderMenu::Power as usize].set_child(Some(power_menu.widget()));
        if let Some(mixer) = &mixer {
            revealers[HeaderMenu::Mixer as usize].set_child(Some(&mixer.content));
        }
        revealers[HeaderMenu::Battery as usize].set_child(Some(battery_menu.widget()));
        for revealer in &revealers {
            root.append(revealer);
        }
        let this = Rc::new(Self {
            root,
            power,
            notifications,
            battery_button,
            battery_icon,
            battery_label,
            battery_menu,
            battery_scale,
            battery_time,
            battery_percent,
            power_button,
            power_menu,
            mixer,
            revealers,
            subscriptions: RefCell::new(Vec::new()),
            warnings: RefCell::new(WarningTracker::default()),
            selected: Cell::new(None),
            pending_menu: Cell::new(None),
            revealing: Cell::new(false),
            refreshing: Cell::new(false),
            refresh_pending: Cell::new(false),
            stopped: Cell::new(false),
            focused: Box::new(focused),
        });
        for (button, menu) in [
            (&this.power_button, HeaderMenu::Power),
            (&this.battery_button, HeaderMenu::Battery),
        ]
        .into_iter()
        .chain(
            this.mixer
                .as_ref()
                .map(|slot| (&slot.button, HeaderMenu::Mixer)),
        ) {
            let weak = Rc::downgrade(&this);
            button.connect_clicked(move |button| {
                if let Some(this) = weak.upgrade()
                    && !this.stopped.get()
                    && button.is_sensitive()
                {
                    this.reveal((this.selected.get() != Some(menu)).then_some(menu));
                }
            });
        }
        this.watch(&this.power);
        this.watch(&this.notifications);
        this.refresh();
        this
    }
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }
    pub fn battery_button(&self) -> &gtk::Button {
        &self.battery_button
    }
    pub fn power_button(&self) -> &gtk::Button {
        &self.power_button
    }
    pub fn power_menu(&self) -> &Rc<PowerMenu> {
        &self.power_menu
    }
    pub fn selected_menu(&self) -> Option<HeaderMenu> {
        self.selected.get()
    }
    pub fn hide_menus(&self) {
        self.reveal(None);
        self.power_menu.hide_confirmations();
    }
    pub fn stop(&self) {
        if self.stopped.replace(true) {
            return;
        }
        self.subscriptions.borrow_mut().clear();
        self.hide_menus();
        self.power_menu.stop();
        self.root.set_sensitive(false);
    }
    fn watch(self: &Rc<Self>, object: &impl IsA<glib::Object>) {
        let weak = Rc::downgrade(self);
        let handler = object.connect_local("changed", false, move |_| {
            if let Some(this) = weak.upgrade() {
                this.refresh();
            }
            None
        });
        self.subscriptions
            .borrow_mut()
            .push(Subscription::new(object, handler));
    }
    fn reveal(&self, menu: Option<HeaderMenu>) {
        self.pending_menu
            .set(Some(if self.stopped.get() { None } else { menu }));
        if self.revealing.replace(true) {
            return;
        }
        while let Some(menu) = self.pending_menu.take() {
            let menu = if self.stopped.get() { None } else { menu };
            self.selected.set(menu);
            for (index, revealer) in self.revealers.iter().enumerate() {
                revealer.set_reveal_child(menu.is_some_and(|menu| index == menu as usize));
            }
            if menu != Some(HeaderMenu::Power) {
                self.power_menu.hide_confirmations();
            }
            if let Some(mixer) = &self.mixer {
                (mixer.revealed)(menu == Some(HeaderMenu::Mixer));
            }
            (self.focused)(menu.is_some());
        }
        self.revealing.set(false);
    }
    fn refresh(&self) {
        if self.stopped.get() {
            return;
        }
        self.refresh_pending.set(true);
        if self.refreshing.replace(true) {
            return;
        }
        while self.refresh_pending.replace(false) && !self.stopped.get() {
            let device = self.power.primary_device();
            let presentation = BatteryPresentation::from_device(device.as_ref());
            self.battery_button.set_sensitive(presentation.available);
            self.battery_button.set_tooltip_text(
                (!presentation.available).then_some("Power information is unavailable"),
            );
            self.battery_icon.set_icon_name(Some(presentation.icon));
            self.battery_label.set_label(&presentation.button);
            self.battery_menu.set_icon(presentation.icon);
            self.battery_scale.set_value(presentation.value);
            self.battery_time.set_label(&presentation.time);
            self.battery_time
                .set_width_chars(if presentation.value == 100.0 { 27 } else { 28 });
            self.battery_percent.set_label(&presentation.percentage);
            if !presentation.available && self.selected.get() == Some(HeaderMenu::Battery) {
                self.reveal(None);
            }
            let warning = self
                .warnings
                .borrow_mut()
                .next(device.as_ref(), self.notifications.state().available);
            if let Some(warning) = warning
                && let Some(percentage) = device.and_then(|device| device.percentage)
                && self
                    .notifications
                    .send_internal(warning.request(percentage))
                    .is_ok()
            {
                self.warnings.borrow_mut().acknowledge(warning);
            }
        }
        self.refreshing.set(false);
    }
}
impl Drop for SystemHeader {
    fn drop(&mut self) {
        self.stop();
    }
}
