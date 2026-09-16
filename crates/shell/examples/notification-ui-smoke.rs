//! Notification cards/groups/popups against a private bus and real Wayland GTK.
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fs,
    rc::Rc,
    time::{Duration, Instant},
};
use way_shell::{
    services::notifications::{
        Action, CloseReason, Hints, Image, Notification, NotificationRequest, NotificationsService,
    },
    ui::{
        message_tray::{
            notification_card::{CardMode, NotificationCard},
            notification_list::NotificationsList,
            notification_osd::NotificationOsd,
        },
        window::{LayerWindow, WindowRole},
    },
};
#[path = "../tests/common/media.rs"]
mod fixture;

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
fn until(predicate: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(Instant::now() < end, "notification UI fixture timed out");
        glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}
fn settle() {
    glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(400)));
}
fn request(app: &str, summary: &str) -> NotificationRequest {
    NotificationRequest {
        app_name: app.into(),
        summary: summary.into(),
        body: "<b>Body</b>".into(),
        actions: vec![
            Action {
                key: "default".into(),
                label: "Open".into(),
            },
            Action {
                key: "reply".into(),
                label: "Reply".into(),
            },
        ],
        ..NotificationRequest::default()
    }
}
fn notify(client: &gio::DBusConnection, request: &NotificationRequest) -> u32 {
    let hints = HashMap::from([("urgency", request.hints.urgency.to_variant())]);
    let actions: Vec<_> = request
        .actions
        .iter()
        .flat_map(|action| [action.key.clone(), action.label.clone()])
        .collect();
    let args = (
        &request.app_name,
        request.replaces_id,
        &request.app_icon,
        &request.summary,
        &request.body,
        actions,
        hints,
        request.expire_timeout,
    )
        .to_variant();
    glib::MainContext::default()
        .block_on(client.call_future(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
            "Notify",
            Some(&args),
            None,
            gio::DBusCallFlags::NONE,
            2000,
        ))
        .unwrap()
        .get::<(u32,)>()
        .unwrap()
        .0
}
fn button(card: &NotificationCard, text: &str) -> gtk::Button {
    let mut child = card.actions().first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Ok(button) = widget.downcast::<gtk::Button>()
            && button.label().as_deref() == Some(text)
        {
            return button;
        }
    }
    panic!("Missing action {text}")
}
fn first_revealer(root: &gtk::Widget) -> Option<gtk::Revealer> {
    if let Ok(revealer) = root.clone().downcast::<gtk::Revealer>() {
        return Some(revealer);
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(revealer) = first_revealer(&widget) {
            return Some(revealer);
        }
        child = widget.next_sibling();
    }
    None
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    adw::init()?;
    let (_bus, address) = fixture::Bus::start();
    let daemon = fixture::connect(&address);
    let client = fixture::connect(&address);
    let service = NotificationsService::on_connection(&daemon);
    until(|| service.state().available);
    let events = Rc::new(RefCell::new(Vec::new()));
    let seen = events.clone();
    let subscription = client.subscribe_to_signal(
        None,
        Some("org.freedesktop.Notifications"),
        Some("ActionInvoked"),
        Some("/org/freedesktop/Notifications"),
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| {
            seen.borrow_mut()
                .push(signal.parameters.get::<(u32, String)>().unwrap());
        },
    );
    let config = settings();
    let one = notify(&client, &request("Mail", "First"));
    let list = NotificationsList::new(service.clone(), config.clone(), || {});
    let window = LayerWindow::new(WindowRole::MessageTray, None)?;
    window.window().set_content(Some(list.widget()));
    window.present();
    until(|| window.window().is_mapped());
    assert!(list.dnd_switch().grab_focus());
    assert_eq!(list.groups().len(), 1);
    let group = list.group("Mail").unwrap();
    let first = group.card(one).unwrap();
    let root = first.widget().clone();
    assert_eq!(first.body().text(), "Body");
    assert_eq!(first.body().label(), "<b>Body</b>");
    let old_action = button(&first, "Reply");
    assert!(old_action.has_css_class("only"));
    old_action.emit_clicked();
    until(|| events.borrow().len() == 1);
    assert_eq!(events.borrow()[0], (one, "reply".into()));
    let two = notify(&client, &request("Mail", "Second"));
    assert_eq!(
        group
            .cards()
            .iter()
            .map(|card| card.id())
            .collect::<Vec<_>>(),
        [two, one]
    );
    group.expand_button().emit_clicked();
    assert!(group.is_expanded());
    let mut replacement = request("Mail", " New\nsummary ");
    replacement.replaces_id = one;
    replacement.body = "<b>broken".into();
    replacement.actions = vec![Action {
        key: "new".into(),
        label: "New".into(),
    }];
    replacement.hints.urgency = 2;
    let parent_changes = Rc::new(Cell::new(0));
    let changes = parent_changes.clone();
    let parent_handler = first
        .widget()
        .connect_parent_notify(move |_| changes.set(changes.get() + 1));
    assert_eq!(notify(&client, &replacement), one);
    first.widget().disconnect(parent_handler);
    assert_eq!(
        parent_changes.get(),
        0,
        "replacing content must retain its parent and focus"
    );
    assert!(Rc::ptr_eq(&group.card(one).unwrap(), &first));
    assert_eq!(first.widget(), &root);
    assert_eq!(first.summary().text(), "New summary");
    assert_eq!(first.body().text(), "<b>broken");
    assert!(
        first
            .button()
            .has_css_class("notification-widget-button-critical")
    );
    assert!(group.is_expanded());
    assert_eq!(
        group
            .cards()
            .iter()
            .map(|card| card.id())
            .collect::<Vec<_>>(),
        [two, one]
    );
    old_action.emit_clicked();
    settle();
    assert_eq!(
        events.borrow().len(),
        1,
        "retained obsolete actions must be inert"
    );
    button(&first, "New").emit_clicked();
    until(|| events.borrow().len() == 2);
    replacement.app_name = "Chat".into();
    replacement.hints.urgency = 0;
    notify(&client, &replacement);
    assert!(group.card(one).is_none());
    assert_eq!(group.cards()[0].id(), two);
    assert!(!group.is_expanded());
    let moved = list.group("Chat").unwrap().card(one).unwrap();
    assert!(
        !moved
            .button()
            .has_css_class("notification-widget-button-critical")
    );
    service.close(two, CloseReason::Requested);
    assert!(list.group("Mail").is_none());
    first.dismiss_button().emit_clicked();
    first.button().emit_clicked();
    assert!(
        service
            .state()
            .notifications
            .iter()
            .any(|notification| notification.id == one),
        "a removed card must not dismiss its replacement in another group"
    );
    moved.button().emit_clicked();
    assert!(list.group("Chat").is_none());
    notify(&client, &request("Clear", "Clear this"));
    let media = gtk::Label::new(Some("Media prefix"));
    list.set_media_widgets(vec![media.clone().upcast()]);
    let parent_changes = Rc::new(Cell::new(0));
    let changes = parent_changes.clone();
    let handler = media.connect_parent_notify(move |_| changes.set(changes.get() + 1));
    notify(&client, &request("Unrelated", "Preserve media parent"));
    media.disconnect(handler);
    assert_eq!(parent_changes.get(), 0);
    list.clear_button().emit_clicked();
    assert!(list.groups().is_empty());
    assert!(!list.is_empty());
    list.set_media_widgets(Vec::new());
    assert!(list.is_empty());
    assert!(list.dnd_switch().grab_focus());
    list.dnd_switch().set_active(true);
    assert!(config.boolean("do-not-disturb"));
    config.set_boolean("do-not-disturb", false)?;
    assert!(!list.dnd_switch().is_active());
    notify(&client, &request("Reentrant", "One"));
    notify(&client, &request("Reentrant", "Two"));
    let group = list.group("Reentrant").unwrap();
    let header = first_revealer(group.widget().upcast_ref()).unwrap();
    let source = service.clone();
    let handler = header.connect_reveal_child_notify(move |revealer| {
        if revealer.reveals_child() {
            for notification in source
                .state()
                .notifications
                .into_iter()
                .filter(|notification| notification.request.app_name == "Reentrant")
            {
                source.close(notification.id, CloseReason::Requested);
            }
        }
    });
    group.expand_button().emit_clicked();
    header.disconnect(handler);
    assert!(list.group("Reentrant").is_none());
    assert!(
        list.is_empty(),
        "closing while expanding must remove the now-empty group"
    );
    list.hidden();

    // Owned image bytes, replacements, queued GTK reentrancy and cancellation.
    let data = Image::new(1, 1, 4, true, 8, 4, vec![255, 0, 0, 255]).unwrap();
    let mut snapshot = Notification {
        id: 77,
        request: request("Images", "Original"),
        created_on_us: glib::real_time() - 120_000_000,
    };
    snapshot.request.hints = Hints {
        image: Some(data),
        urgency: 2,
        ..Hints::default()
    };
    let card = NotificationCard::new(service.clone(), snapshot.clone(), CardMode::Tray, |_| {});
    root.hide();
    let image = card.avatar().custom_image().unwrap();
    let texture = image.downcast::<gtk::gdk::Texture>().unwrap();
    assert_eq!(texture.width(), 1);
    card.expand_button().emit_clicked();
    assert!(card.is_expanded());
    let weak = Rc::downgrade(&card);
    let fired = Rc::new(Cell::new(false));
    let once = fired.clone();
    let mut nested = snapshot.clone();
    nested.request.summary = "Newest".into();
    nested.request.hints.image = None;
    nested.request.hints.urgency = 0;
    let handler = card.summary().connect_label_notify(move |_| {
        if !once.replace(true)
            && let Some(card) = weak.upgrade()
        {
            card.set_notification(nested.clone());
        }
    });
    snapshot.request.summary = "Superseded".into();
    card.set_notification(snapshot.clone());
    card.summary().disconnect(handler);
    assert!(fired.get());
    assert_eq!(card.summary().text(), "Newest");
    assert!(card.avatar().custom_image().is_none());
    assert!(card.is_expanded());
    assert!(
        !card
            .button()
            .has_css_class("notification-widget-button-critical")
    );
    let directory =
        std::env::temp_dir().join(format!("way-shell-notification-ui-{}", std::process::id()));
    fs::create_dir_all(&directory)?;
    let path = directory.join("art.png");
    let pixbuf =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 64, 64).unwrap();
    pixbuf.fill(0x00ff00ff);
    pixbuf.savev(&path, "png", &[])?;
    snapshot.request.hints.image = None;
    snapshot.request.hints.image_path = Some(path.to_string_lossy().into_owned());
    card.set_notification(snapshot.clone());
    until(|| card.avatar().custom_image().is_some());
    assert_eq!(card.avatar().custom_image().unwrap().intrinsic_width(), 48);
    card.set_notification(snapshot.clone());
    snapshot.request.hints.image_path = None;
    card.set_notification(snapshot.clone());
    settle();
    assert!(card.avatar().custom_image().is_none());
    snapshot.request.hints.image_path = Some(path.to_string_lossy().into_owned());
    card.set_notification(snapshot.clone());
    let weak = Rc::downgrade(&card);
    let retained = card.widget().clone();
    drop(card);
    assert!(weak.upgrade().is_none());
    assert!(!retained.is_sensitive());
    settle();
    fs::remove_dir_all(directory)?;

    // Hidden/DND replacements update existing content without reopening a popup.
    let osd = NotificationOsd::new(service.clone(), config.clone())?;
    let mut popup = request("Popup", "Visible");
    let id = notify(&client, &popup);
    until(|| osd.is_visible());
    let initial = osd.card().unwrap();
    let hide_button = button(&initial, "Hide");
    assert_eq!(
        initial
            .actions()
            .last_child()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .label()
            .as_deref(),
        Some("Hide")
    );
    popup.replaces_id = id;
    popup.summary = "Replacement".into();
    notify(&client, &popup);
    assert!(Rc::ptr_eq(&initial, &osd.card().unwrap()));
    assert_eq!(initial.summary().text(), "Replacement");
    assert_eq!(button(&initial, "Hide"), hide_button);
    assert!(hide_button.has_css_class("last"));
    popup.actions.clear();
    notify(&client, &popup);
    assert_eq!(button(&initial, "Hide"), hide_button);
    assert!(hide_button.has_css_class("only"));
    assert!(!hide_button.has_css_class("last"));
    button(&initial, "Hide").emit_clicked();
    until(|| !osd.window().get_visible());
    popup.summary = "Hidden replacement".into();
    notify(&client, &popup);
    settle();
    assert!(!osd.is_visible());
    assert_eq!(initial.summary().text(), "Hidden replacement");
    config.set_boolean("do-not-disturb", true)?;
    let mut critical = request("Popup", "Critical in DND");
    critical.hints.urgency = 2;
    notify(&client, &critical);
    assert!(!osd.is_visible());
    config.set_boolean("do-not-disturb", false)?;
    osd.set_tray_visible(true);
    notify(&client, &request("Popup", "Tray open"));
    assert!(!osd.is_visible());
    osd.set_tray_visible(false);
    let timed = notify(&client, &request("Popup", "Timeout"));
    until(|| osd.is_visible());
    until(|| !osd.window().get_visible());
    assert!(
        service
            .state()
            .notifications
            .iter()
            .any(|notification| notification.id == timed),
        "popup timeout must retain history"
    );
    notify(&client, &request("Popup", "Drop visible"));
    until(|| osd.is_visible());
    let retained = osd.window().clone();
    let weak = Rc::downgrade(&osd);
    drop(osd);
    assert!(weak.upgrade().is_none());
    assert!(!retained.get_visible());

    let clear = list.clear_button().clone();
    let weak = Rc::downgrade(&list);
    let count = service.state().notifications.len();
    drop(list);
    assert!(weak.upgrade().is_none());
    clear.emit_clicked();
    assert_eq!(service.state().notifications.len(), count);
    window.close();
    drop(subscription);
    service.stop();
    println!(
        "notification cards, groups, actions, replacement, images, DND, popup timeout and ownership passed"
    );
    Ok(())
}
