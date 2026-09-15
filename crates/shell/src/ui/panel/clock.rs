//! Panel clock and notification indicator, sharing the application's services.
use crate::services::{clock::ClockService, notifications::NotificationsService};
use gio::prelude::*;
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct Inner {
    root: gtk::Box,
    button: gtk::Button,
    label: gtk::Label,
    indicator: gtk::Image,
    format: String,
    notifications: NotificationsService,
    settings: gio::Settings,
    action: Box<dyn Fn()>,
    handlers: RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>,
    updating: Cell<bool>,
    pending: Cell<bool>,
}
impl Inner {
    fn update_notifications(&self) {
        self.pending.set(true);
        if self.updating.replace(true) {
            return;
        }
        while self.pending.replace(false) {
            let dnd = self.settings.boolean("do-not-disturb");
            self.indicator.set_icon_name(Some(if dnd {
                "notifications-disabled-symbolic"
            } else {
                "preferences-system-notifications-symbolic"
            }));
            self.indicator
                .set_visible(dnd || !self.notifications.state().notifications.is_empty());
        }
        self.updating.set(false);
    }
    fn tick(&self, now: &glib::DateTime) {
        if let Ok(text) = now.format(&self.format) {
            self.label.set_label(&text);
        }
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        for (source, handler) in self.handlers.get_mut().drain(..) {
            source.disconnect(handler);
        }
    }
}

#[derive(Clone)]
pub struct PanelClock(Rc<Inner>);
impl PanelClock {
    pub fn new(
        clock: ClockService,
        notifications: NotificationsService,
        panel_settings: &gio::Settings,
        notification_settings: gio::Settings,
        action: impl Fn() + 'static,
    ) -> Result<Self, glib::BoolError> {
        let now = glib::DateTime::now_local()?;
        let requested = panel_settings.string("clock-format");
        let fallback = panel_settings
            .default_value("clock-format")
            .and_then(|value| value.get::<String>())
            .unwrap_or_else(|| "%H:%M".into());
        let format = valid_format(&now, &requested, &fallback);
        if format != requested {
            glib::g_warning!(
                "way-shell",
                "Invalid clock-format setting {requested:?}; using the default"
            );
        }
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("panel-clock");
        let button = gtk::Button::new();
        button.add_css_class("panel-button");
        let label = gtk::Label::new(None);
        button.set_child(Some(&label));
        root.append(&button);
        let indicator = gtk::Image::new();
        indicator.add_css_class("panel-clock-notif");
        root.append(&indicator);
        let inner = Rc::new(Inner {
            root,
            button,
            label,
            indicator,
            format,
            notifications,
            settings: notification_settings,
            action: Box::new(action),
            handlers: RefCell::new(Vec::new()),
            updating: Cell::new(false),
            pending: Cell::new(false),
        });
        let weak = Rc::downgrade(&inner);
        inner.button.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                (inner.action)();
            }
        });
        let weak = Rc::downgrade(&inner);
        let handler = clock.connect_local("tick", false, move |values| {
            if let Some(inner) = weak.upgrade() {
                inner.tick(&values[1].get::<glib::DateTime>().expect("clock tick"));
            }
            None
        });
        inner.handlers.borrow_mut().push((clock.upcast(), handler));
        let weak = Rc::downgrade(&inner);
        let handler = inner
            .notifications
            .connect_local("changed", false, move |_| {
                if let Some(inner) = weak.upgrade() {
                    inner.update_notifications();
                }
                None
            });
        inner
            .handlers
            .borrow_mut()
            .push((inner.notifications.clone().upcast(), handler));
        let weak = Rc::downgrade(&inner);
        let handler = inner
            .settings
            .connect_changed(Some("do-not-disturb"), move |_, _| {
                if let Some(inner) = weak.upgrade() {
                    inner.update_notifications();
                }
            });
        inner
            .handlers
            .borrow_mut()
            .push((inner.settings.clone().upcast(), handler));
        inner.tick(&now);
        inner.update_notifications();
        Ok(Self(inner))
    }
    pub fn widget(&self) -> &gtk::Box {
        &self.0.root
    }
    pub fn set_toggled(&self, toggled: bool) {
        if toggled {
            self.0.button.add_css_class("panel-button-toggled");
        } else {
            self.0.button.remove_css_class("panel-button-toggled");
        }
    }
}

fn valid_format(now: &glib::DateTime, requested: &str, fallback: &str) -> String {
    [requested, fallback, "%H:%M"]
        .into_iter()
        .find(|format| now.format(format).is_ok())
        .expect("the fixed clock format is valid")
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clock_format_fallback_is_owned_and_keeps_the_schema_default() {
        let now = glib::DateTime::from_utc(2026, 9, 16, 12, 34, 0.0).unwrap();
        let fallback = String::from("%Y-%m-%d %H:%M");
        let chosen = valid_format(&now, "%Q", &fallback);
        drop(fallback);
        assert_eq!(now.format(&chosen).unwrap(), "2026-09-16 12:34");
        assert_eq!(valid_format(&now, "%H.%M", "%Q"), "%H.%M");
        assert_eq!(valid_format(&now, "%Q", "%Q"), "%H:%M");
    }
}
