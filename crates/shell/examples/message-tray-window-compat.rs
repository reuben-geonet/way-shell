//! Message-tray will-hide remains at the end of the fade, before unmapping.
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};
use way_shell::ui::{
    message_tray::window::{MessageTrayEvent, MessageTrayWindow},
    window::Visibility,
};
fn until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "message-tray window timed out");
        glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}
fn main() {
    adw::init().unwrap();
    let view = MessageTrayWindow::new().unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    let seen = events.clone();
    let weak = Rc::downgrade(&view);
    view.on_event(move |event| {
        let view = weak.upgrade().unwrap();
        match event {
            MessageTrayEvent::WillShow | MessageTrayEvent::Hidden => {
                assert!(!view.window().is_visible())
            }
            MessageTrayEvent::WillHide | MessageTrayEvent::Visible => {
                assert!(view.window().is_visible())
            }
        }
        seen.borrow_mut().push(event);
    });
    view.show();
    until(|| view.visibility_controller().visibility() == Visibility::Visible);
    view.underlay_button().emit_clicked();
    until(|| view.visibility_controller().visibility() == Visibility::Hidden);
    assert_eq!(
        *events.borrow(),
        [
            MessageTrayEvent::WillShow,
            MessageTrayEvent::Visible,
            MessageTrayEvent::WillHide,
            MessageTrayEvent::Hidden
        ]
    );
    drop(view);

    let view = MessageTrayWindow::new().unwrap();
    let weak = Rc::downgrade(&view);
    let reopened = Rc::new(Cell::new(false));
    let observed = reopened.clone();
    view.on_event(move |event| {
        if event == MessageTrayEvent::WillHide && !observed.replace(true) {
            weak.upgrade().unwrap().show();
        }
    });
    view.show();
    until(|| view.visibility_controller().visibility() == Visibility::Visible);
    view.hide();
    until(|| reopened.get() && view.visibility_controller().visibility() == Visibility::Visible);
    assert!(view.underlay_button().is_mapped());
    let retained = view.window().clone();
    let underlay = view.underlay_button().clone();
    drop(view);
    underlay.emit_clicked();
    assert!(!retained.is_visible() && !underlay.is_mapped());
    println!("message-tray fade timing, reentrant reopen and lifetime checks passed");
}
