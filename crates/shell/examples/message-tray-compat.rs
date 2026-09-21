//! Composed message-tray mediation, history and shared-service ownership.
use adw::prelude::*;
use std::{rc::Rc, time::Duration};
use way_shell::{
    services::{
        clock::ClockService,
        media::MediaService,
        notifications::{CloseReason, NotificationsService},
    },
    ui::{
        message_tray::{MessageTray, MessageTrayServices},
        window::Visibility,
    },
};
use way_shell_core::notifications::NotificationRequest;
#[allow(dead_code)]
#[path = "../tests/common/network.rs"]
mod bus;
fn settings() -> gio::Settings {
    let source =
        gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)
            .unwrap();
    gio::Settings::new_full(
        &source
            .lookup("org.ldelossa.way-shell.notifications", false)
            .unwrap(),
        Some(&gio::memory_settings_backend_new()),
        None,
    )
}
fn request(summary: &str) -> NotificationRequest {
    NotificationRequest {
        app_name: "Fixture".into(),
        summary: summary.into(),
        ..NotificationRequest::default()
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    adw::init()?;
    let context = glib::MainContext::default();
    let _guard = context.acquire()?;
    let (_bus, address) = bus::Bus::start();
    let client = bus::connect(&address);
    let notifications = NotificationsService::on_connection(&client);
    let media = MediaService::on_connection(&client);
    let clock = ClockService::new();
    bus::wait(&context, || notifications.state().available);
    for _ in 0..2 {
        let view = MessageTray::new(
            MessageTrayServices {
                clock: clock.clone(),
                media: media.clone(),
                notifications: notifications.clone(),
            },
            settings(),
        )?;
        let first = notifications.send_internal(request("One"))?;
        bus::wait(&context, || view.notification_osd().is_visible());
        view.window().show();
        bus::wait(&context, || {
            view.window().visibility_controller().visibility() == Visibility::Visible
                && !view.notification_osd().is_visible()
        });
        notifications.send_internal(request("Two"))?;
        assert!(
            !view.notification_osd().is_visible(),
            "an open tray suppresses notification popups"
        );
        let group = view.notifications().group("Fixture").unwrap();
        group.expand_button().emit_clicked();
        assert!(group.is_expanded());
        view.window().hide();
        bus::wait(&context, || {
            view.window().visibility_controller().visibility() == Visibility::Hidden
        });
        assert!(
            !group.is_expanded(),
            "the completed tray fade collapses notification groups"
        );
        let next = notifications.send_internal(request("Three"))?;
        bus::wait(&context, || view.notification_osd().is_visible());
        assert!(
            view.notifications()
                .group("Fixture")
                .unwrap()
                .card(first)
                .is_some()
        );
        view.notification_osd().hide();
        bus::wait(&context, || !view.notification_osd().is_visible());
        view.notifications().dnd_switch().set_active(true);
        notifications.send_internal(request("Muted"))?;
        context.block_on(glib::timeout_future(Duration::from_millis(30)));
        assert!(!view.notification_osd().is_visible());
        assert!(
            view.notifications()
                .group("Fixture")
                .unwrap()
                .card(next)
                .is_some()
        );
        let clear = view.notifications().clear_button().clone();
        let window = view.window().window().clone();
        let osd = view.notification_osd().window().clone();
        let weak = Rc::downgrade(&view);
        drop(view);
        assert!(weak.upgrade().is_none());
        clear.emit_clicked();
        assert!(!window.is_visible() && !osd.is_visible());
        assert!(clock.is_enabled() && notifications.state().available);
        for notification in notifications.state().notifications {
            notifications.close(notification.id, CloseReason::Dismissed);
        }
    }
    notifications.stop();
    media.stop();
    clock.set_enabled(false);
    println!("composed message-tray popup suppression, history, collapse and ownership passed");
    Ok(())
}
