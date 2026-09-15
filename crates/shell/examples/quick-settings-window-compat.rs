//! Quick-settings animation and click-away ownership on real Sway/Niri.
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};
use way_shell::ui::{
    quick_settings::{QuickSettingsEvent, QuickSettingsWindow},
    window::Visibility,
};

fn until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "quick-settings window timed out");
        glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}
fn settle() {
    glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(300)));
}
fn window() -> Rc<QuickSettingsWindow> {
    let view = QuickSettingsWindow::new().unwrap();
    view.content()
        .append(&gtk::Label::new(Some("Quick settings")));
    view
}

fn main() {
    adw::init().unwrap();
    for _ in 0..3 {
        let view = window();
        let events = Rc::new(RefCell::new(Vec::new()));
        let seen = events.clone();
        let weak = Rc::downgrade(&view);
        view.on_event(move |event| {
            let view = weak.upgrade().unwrap();
            if event == QuickSettingsEvent::WillShow {
                assert!(!view.window().is_visible());
            }
            seen.borrow_mut().push(event);
        });
        assert!(view.show());
        until(|| view.visibility_controller().visibility() == Visibility::Visible);
        assert!(view.window().is_visible());
        assert!(view.underlay_button().is_mapped());
        assert_eq!(view.window().opacity(), 1.0);
        assert_eq!(
            *events.borrow(),
            [QuickSettingsEvent::WillShow, QuickSettingsEvent::Visible]
        );
        view.set_focused(true);
        assert!(view.window().has_css_class("focused"));
        view.underlay_button().emit_clicked();
        until(|| view.visibility_controller().visibility() == Visibility::Hidden);
        assert!(!view.window().is_visible());
        assert!(!view.underlay_button().is_mapped());
        assert!(!view.window().has_css_class("focused"));
        assert_eq!(events.borrow().last(), Some(&QuickSettingsEvent::Hidden));
        let retained_button = view.underlay_button().clone();
        let retained_window = view.window().clone();
        drop(view);
        retained_button.emit_clicked();
        assert!(!retained_window.is_visible());
    }

    let view = window();
    view.show();
    view.hide();
    view.show();
    until(|| view.visibility_controller().visibility() == Visibility::Visible);
    settle();
    assert!(view.window().is_visible());
    assert!(view.underlay_button().is_mapped());

    // GTK visibility callbacks can reopen while a completed hide unmaps.
    let flag = Rc::new(Cell::new(false));
    let reopened = flag.clone();
    let weak = Rc::downgrade(&view);
    let handler = view.window().connect_visible_notify(move |window| {
        if !window.is_visible() && !reopened.replace(true) {
            weak.upgrade().unwrap().show();
        }
    });
    view.hide();
    until(|| flag.get() && view.visibility_controller().visibility() == Visibility::Visible);
    view.window().disconnect(handler);
    settle();
    assert!(view.window().is_visible() && view.underlay_button().is_mapped());
    view.close();
    assert!(!view.show());
    assert!(!view.window().is_visible());

    // Final-owner teardown closes both surfaces without calling observers
    // whose weak controller is already unavailable.
    let dropped = window();
    let weak = Rc::downgrade(&dropped);
    dropped.on_event(move |_| {
        assert!(weak.upgrade().is_some());
    });
    dropped.show();
    until(|| dropped.visibility_controller().visibility() == Visibility::Visible);
    let retained = dropped.window().clone();
    let underlay = dropped.underlay_button().clone();
    drop(dropped);
    underlay.emit_clicked();
    settle();
    assert!(!retained.is_visible() && !underlay.is_mapped());

    // A mediator can cancel an opening before either surface is presented.
    let view = window();
    let weak = Rc::downgrade(&view);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let events = seen.clone();
    view.on_event(move |event| {
        events.borrow_mut().push(event);
        if event == QuickSettingsEvent::WillShow {
            weak.upgrade().unwrap().hide();
        }
    });
    assert!(!view.show());
    settle();
    assert!(!view.window().is_visible());
    assert_eq!(
        *seen.borrow(),
        [QuickSettingsEvent::WillShow, QuickSettingsEvent::Hidden]
    );
    view.close();
    println!("quick-settings animation, mediation, click-away and ownership checks passed");
}
