//! Notification, media and calendar presentation with owned layer windows.
pub mod calendar;
pub mod media;
pub mod notification_card;
pub mod notification_group;
pub mod notification_list;
pub mod notification_osd;
pub mod window;

use crate::services::{
    clock::ClockService, media::MediaService, notifications::NotificationsService,
};
use calendar::Calendar;
use gtk::prelude::*;
use media::MediaCards;
use notification_list::NotificationsList;
use notification_osd::NotificationOsd;
use std::{cell::Cell, rc::Rc};
use window::{MessageTrayEvent, MessageTrayWindow};

pub struct MessageTrayServices {
    pub clock: ClockService,
    pub notifications: NotificationsService,
    pub media: MediaService,
}
pub struct MessageTray {
    window: Rc<MessageTrayWindow>,
    notifications: Rc<NotificationsList>,
    osd: Rc<NotificationOsd>,
    media: Rc<MediaCards>,
    calendar: Rc<Calendar>,
    stopped: Cell<bool>,
}
impl MessageTray {
    pub fn new(services: MessageTrayServices, settings: gio::Settings) -> Result<Rc<Self>, String> {
        let window = MessageTrayWindow::new()?;
        let weak = Rc::downgrade(&window);
        let notifications = NotificationsList::new(
            services.notifications.clone(),
            settings.clone(),
            move |height| {
                if let Some(window) = weak.upgrade() {
                    window.fit_height(height);
                }
            },
        );
        let weak = Rc::downgrade(&notifications);
        let media = MediaCards::new(services.media, move |widgets| {
            if let Some(notifications) = weak.upgrade() {
                notifications.set_media_widgets(widgets);
            }
        });
        let calendar = Calendar::new(services.clock).map_err(|error| error.to_string())?;
        let osd = NotificationOsd::new(services.notifications, settings)?;
        let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
        left.set_width_request(469);
        left.append(notifications.widget());
        let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
        right.set_hexpand(true);
        right.set_vexpand(true);
        right.append(calendar.widget());
        window.content().append(&left);
        window
            .content()
            .append(&gtk::Separator::new(gtk::Orientation::Vertical));
        window.content().append(&right);
        let weak = Rc::downgrade(&notifications);
        window.on_height_budget(move |budget| {
            if let Some(notifications) = weak.upgrade() {
                notifications.set_height_budget(budget);
            }
        });
        let this = Rc::new(Self {
            window,
            notifications,
            osd,
            media,
            calendar,
            stopped: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        this.window.on_event(move |event| {
            if let Some(this) = weak.upgrade().filter(|this| !this.stopped.get()) {
                match event {
                    MessageTrayEvent::WillShow => this.osd.tray_will_show(),
                    MessageTrayEvent::Visible => this.osd.set_tray_visible(true),
                    MessageTrayEvent::WillHide => this.notifications.collapse(),
                    MessageTrayEvent::Hidden => {
                        this.osd.set_tray_visible(false);
                        this.notifications.hidden();
                    }
                }
            }
        });
        let weak = Rc::downgrade(&this);
        this.window
            .visibility_controller()
            .main()
            .on_closed(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.close();
                }
            });
        Ok(this)
    }
    pub fn window(&self) -> &Rc<MessageTrayWindow> {
        &self.window
    }
    pub fn notifications(&self) -> &Rc<NotificationsList> {
        &self.notifications
    }
    pub fn notification_osd(&self) -> &Rc<NotificationOsd> {
        &self.osd
    }
    pub fn close(&self) {
        if self.stopped.replace(true) {
            return;
        }
        self.notifications.stop();
        self.osd.stop();
        self.media.close();
        self.calendar.close();
        self.window.close();
    }
}
impl Drop for MessageTray {
    fn drop(&mut self) {
        self.close();
    }
}
