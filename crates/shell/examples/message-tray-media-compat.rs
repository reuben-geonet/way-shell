//! Calendar and media cards under real GTK, with a private MPRIS bus.
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};
use way_shell::{
    services::{clock::ClockService, media::MediaService},
    ui::message_tray::{calendar::Calendar, media::MediaCards},
};

#[path = "../tests/common/media.rs"]
mod fixture;

fn descendants(widget: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut result = vec![widget.as_ref().clone()];
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        result.extend(descendants(&widget));
    }
    result
}
fn child<T: IsA<gtk::Widget> + glib::object::ObjectType>(root: &impl IsA<gtk::Widget>) -> T {
    descendants(root)
        .into_iter()
        .find_map(|widget| widget.downcast::<T>().ok())
        .unwrap()
}
fn label(root: &impl IsA<gtk::Widget>, class: &str) -> gtk::Label {
    descendants(root)
        .into_iter()
        .find(|widget| widget.has_css_class(class))
        .unwrap()
        .downcast()
        .unwrap()
}
fn button(root: &impl IsA<gtk::Widget>, icon: &str) -> gtk::Button {
    descendants(root)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| {
            button.icon_name().as_deref() == Some(icon)
                || button
                    .child()
                    .and_downcast::<gtk::Image>()
                    .is_some_and(|image| image.icon_name().as_deref() == Some(icon))
        })
        .unwrap()
}
fn date(year: i32, month: i32, day: i32) -> glib::DateTime {
    glib::DateTime::from_local(year, month, day, 12, 0, 0.0).unwrap()
}
fn calendar(host: &gtk::Box) {
    let clock = ClockService::new();
    clock.set_enabled(false);
    let calendar = Calendar::new(clock.clone()).unwrap();
    host.append(calendar.widget());
    let selector = descendants(calendar.widget())
        .into_iter()
        .find(|widget| widget.widget_name() == "calendar-month-selector")
        .unwrap();
    let month: gtk::Label = child(&selector);
    let today = descendants(calendar.widget())
        .into_iter()
        .find(|widget| widget.widget_name() == "calendar-today")
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    let row: adw::ActionRow = child(&today);
    assert!(!month.text().is_empty());
    assert!(!row.title().is_empty());
    assert!(row.subtitle().is_some());
    let january = date(2024, 1, 31);
    clock.emit_by_name::<()>("tick", &[&january]);
    calendar.calendar().select_day(&january);
    assert!(!today.has_css_class("calendar-today-dirty"));
    button(&selector, "pan-end-symbolic").emit_clicked();
    assert_eq!(calendar.calendar().date().day_of_month(), 29);
    assert_eq!(calendar.calendar().date().month(), 2);
    assert!(today.has_css_class("calendar-today-dirty"));
    let next_year = date(2025, 1, 31);
    clock.emit_by_name::<()>("tick", &[&next_year]);
    assert_eq!(
        row.subtitle().as_deref(),
        Some(next_year.format("%B %d %Y").unwrap().as_str())
    );
    assert_eq!(month.text(), date(2024, 2, 29).format("%B %Y").unwrap());
    today.emit_clicked();
    assert_eq!(calendar.calendar().date().year(), 2025);
    assert_eq!(calendar.calendar().date().day_of_month(), 31);
    assert!(!today.has_css_class("calendar-today-dirty"));
    calendar.calendar().mark_day(3);
    let february = date(2025, 2, 1);
    clock.emit_by_name::<()>("tick", &[&february]);
    assert_eq!(month.text(), next_year.format("%B %Y").unwrap());
    assert!(today.has_css_class("calendar-today-dirty"));
    today.emit_clicked();
    assert_eq!(calendar.calendar().date().month(), 2);
    assert!(!calendar.calendar().day_is_marked(3));
    let captured = clock.clone();
    let once = Cell::new(false);
    let handler = row.connect_title_notify(move |_| {
        if !once.replace(true) {
            captured.emit_by_name::<()>("tick", &[&date(2031, 7, 8)]);
        }
    });
    clock.emit_by_name::<()>("tick", &[&date(2030, 6, 7)]);
    assert_eq!(
        row.subtitle().as_deref(),
        Some(date(2031, 7, 8).format("%B %d %Y").unwrap().as_str())
    );
    row.disconnect(handler);
    let old = row.subtitle();
    let weak = Rc::downgrade(&calendar);
    drop(calendar);
    assert!(weak.upgrade().is_none());
    clock.emit_by_name::<()>("tick", &[&date(2040, 1, 1)]);
    assert_eq!(row.subtitle(), old);
    today.emit_clicked();
    assert_eq!(row.subtitle(), old);
}
fn metadata(title: &str, art: Option<&str>) -> glib::Variant {
    let mut values = HashMap::from([
        ("xesam:title", title.to_variant()),
        ("xesam:artist", vec!["One", "Two"].to_variant()),
    ]);
    if let Some(art) = art {
        values.insert("mpris:artUrl", art.to_variant());
    }
    values.to_variant()
}
fn media(context: &glib::MainContext, host: &gtk::Box) {
    let (_bus, address) = fixture::Bus::start();
    let connection = fixture::connect(&address);
    let service = MediaService::on_connection(&connection);
    let prefix = gtk::Box::new(gtk::Orientation::Vertical, 0);
    prefix.set_widget_name("notifications-list");
    host.append(&prefix);
    let previous = Rc::new(RefCell::new(Vec::<gtk::Widget>::new()));
    let captured = previous.clone();
    let container = prefix.clone();
    let cards = MediaCards::new(service.clone(), move |widgets| {
        for old in captured.borrow().iter() {
            if !widgets.contains(old) && old.parent().is_some() {
                container.remove(old);
            }
        }
        let mut prior: Option<gtk::Widget> = None;
        for widget in &widgets {
            if widget.parent().is_none() {
                container.append(widget);
            }
            container.reorder_child_after(widget, prior.as_ref());
            prior = Some(widget.clone());
        }
        captured.replace(widgets);
    });
    assert!(cards.widgets().is_empty());
    let art = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/tray-icon.png"
    );
    let first = fixture::Player::new(&address, "org.mpris.MediaPlayer2.first", "First");
    first.property(fixture::PLAYER, "Metadata", metadata("First", Some(art)));
    first.acquire(false);
    fixture::wait(context, || cards.widgets().len() == 1);
    let original = cards.widgets()[0].clone();
    let avatar: adw::Avatar = child(&original);
    fixture::wait(context, || avatar.custom_image().is_some());
    assert_eq!(avatar.custom_image().unwrap().intrinsic_width(), 48);
    assert_eq!(label(&original, "summary").text(), "One, Two");
    assert_eq!(label(&original, "body").text(), "First");
    let main = descendants(&original)
        .into_iter()
        .find(|widget| widget.has_css_class("notification-widget-button"))
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    for icon in [
        "go-previous-symbolic",
        "media-playback-pause-symbolic",
        "go-next-symbolic",
    ] {
        button(&original, icon).emit_clicked();
    }
    main.emit_clicked();
    fixture::wait(context, || first.calls.borrow().len() == 4);
    assert_eq!(
        first
            .calls
            .borrow()
            .iter()
            .map(|(_, method)| method.as_str())
            .collect::<Vec<_>>(),
        vec!["Previous", "PlayPause", "Next", "Raise"]
    );
    assert_eq!(first.calls.borrow()[3].0, fixture::ROOT);
    first.property(fixture::PLAYER, "PlaybackStatus", "Paused".to_variant());
    fixture::wait(context, || {
        descendants(&original).iter().any(|widget| {
            widget.downcast_ref::<gtk::Button>().is_some_and(|button| {
                button.icon_name().as_deref() == Some("media-playback-start-symbolic")
            })
        })
    });
    first.property(
        fixture::PLAYER,
        "Metadata",
        metadata("<b>Literal new title</b>", None),
    );
    fixture::wait(context, || {
        label(&original, "body").text() == "<b>Literal new title</b>"
    });
    assert!(avatar.custom_image().is_none());
    assert_eq!(cards.widgets()[0], original);

    let second = fixture::Player::new(&address, "org.mpris.MediaPlayer2.second", "Second");
    second.acquire(false);
    fixture::wait(context, || cards.widgets().len() == 2);
    assert_eq!(cards.widgets()[1], original);
    let second_root = cards.widgets()[0].clone();
    first.property(
        fixture::PLAYER,
        "Metadata",
        metadata("Updated in place", Some(art)),
    );
    fixture::wait(context, || {
        label(&original, "body").text() == "Updated in place"
    });
    assert_eq!(cards.widgets(), vec![second_root.clone(), original.clone()]);
    first.property(fixture::PLAYER, "CanGoNext", false.to_variant());
    let next = button(&original, "go-next-symbolic");
    fixture::wait(context, || !next.is_sensitive());
    let calls = first.calls.borrow().len();
    next.emit_clicked();
    assert_eq!(first.calls.borrow().len(), calls);
    first.mode.set(fixture::Reply::Failure);
    let play = button(&original, "media-playback-start-symbolic");
    play.emit_clicked();
    fixture::wait(context, || play.tooltip_text().is_some());
    assert!(original.is_sensitive());

    let replacement = fixture::Player::new(&address, &first.name, "Replacement");
    replacement.acquire(true);
    fixture::wait(context, || {
        cards.widgets().len() == 2 && !cards.widgets().contains(&original)
    });
    play.emit_clicked();
    main.emit_clicked();
    assert!(replacement.calls.borrow().is_empty());
    assert!(original.parent().is_none());
    assert!(!original.is_sensitive());
    // Closing the complete controller during one removed card's GTK notification
    // must not let the in-progress reconcile restore its surviving cards.
    let terminal = MediaCards::new(service.clone(), |_| {});
    let removed = terminal
        .widgets()
        .into_iter()
        .find(|widget| label(widget, "body").text() == "Second")
        .unwrap();
    let weak = Rc::downgrade(&terminal);
    let handler = removed.connect_sensitive_notify(move |widget| {
        if !widget.is_sensitive()
            && let Some(cards) = weak.upgrade()
        {
            cards.close();
        }
    });
    second.release();
    fixture::wait(context, || cards.widgets().len() == 1);
    assert!(
        terminal.widgets().is_empty(),
        "terminal close must survive nested card removal"
    );
    removed.disconnect(handler);
    drop(terminal);
    let replacement_root = cards.widgets()[0].clone();
    let captured = service.clone();
    let once = Cell::new(false);
    let title = label(&replacement_root, "body");
    let handler = title.connect_label_notify(move |_| {
        if !once.replace(true) {
            captured.stop();
        }
    });
    replacement.property(
        fixture::PLAYER,
        "Metadata",
        metadata("Stop during UI refresh", None),
    );
    fixture::wait(context, || cards.widgets().is_empty());
    assert!(prefix.first_child().is_none());
    title.disconnect(handler);
    service.start();
    fixture::wait(context, || cards.widgets().len() == 1);
    replacement.property(
        fixture::PLAYER,
        "Metadata",
        metadata("Load then close", Some(art)),
    );
    fixture::wait(context, || {
        label(&cards.widgets()[0], "body").text() == "Load then close"
    });
    let retained = cards.widgets()[0].clone();
    let weak = Rc::downgrade(&cards);
    drop(cards);
    assert!(weak.upgrade().is_none());
    assert!(!retained.is_sensitive());
    assert!(prefix.first_child().is_none());
    context.block_on(glib::timeout_future(Duration::from_millis(40)));
    service.stop();
    connection.close_sync(gio::Cancellable::NONE).unwrap();
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    adw::init()?;
    let context = glib::MainContext::default();
    let _guard = context.acquire()?;
    let host = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    host.set_widget_name("message-tray");
    let window = gtk::Window::builder()
        .default_width(1100)
        .default_height(700)
        .child(&host)
        .build();
    window.present();
    calendar(&host);
    media(&context, &host);
    window.close();
    println!("calendar/date rollover and media metadata/artwork/transport/ownership checks passed");
    Ok(())
}
