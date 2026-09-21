//! Exercise owned windows on the isolated real Sway/Niri component harness.
use adw::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};
use way_shell::ui::window::{
    CloseReason, LayerWindow, PopupCoordinator, PopupEvent, PopupRole, Visibility,
    VisibilityController, WindowRole,
};

fn until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "window smoke test timed out");
        glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}

fn popup(role: WindowRole, underlay: Option<WindowRole>) -> VisibilityController {
    let main = LayerWindow::new(role, None).unwrap();
    main.window()
        .set_content(Some(&gtk::Label::new(Some("Window ownership smoke"))));
    VisibilityController::new(
        main,
        underlay.map(|role| {
            let window = LayerWindow::new(role, None).unwrap();
            window.window().set_content(Some(&gtk::Button::new()));
            window
        }),
    )
}

fn show(controller: &VisibilityController) {
    let transition = controller.begin_show().unwrap();
    assert!(controller.finish_transition(&transition));
    until(|| controller.main().window().is_mapped());
}

fn transition_and_focus() {
    let controller = popup(
        WindowRole::QuickSettings,
        Some(WindowRole::QuickSettingsUnderlay),
    );
    let window = controller.main().window();
    assert_eq!(
        window.namespace().as_deref(),
        Some("way-shell-quick-settings")
    );
    assert_eq!(window.layer(), Layer::Overlay);
    assert_eq!(window.keyboard_mode(), KeyboardMode::Exclusive);
    assert_eq!(window.margin(Edge::Top), 8);
    assert_eq!(window.margin(Edge::Right), 20);
    assert_eq!(window.exclusive_zone(), 0);
    let entry = gtk::Entry::new();
    window.set_content(Some(&entry));
    show(&controller);
    let underlay = controller.underlay().unwrap().window();
    // The headless seat has no physical keyboard. Check the requested keyboard
    // mode and GTK's focus chain without requiring native keyboard-enter.
    until(|| underlay.is_mapped());
    assert_eq!(underlay.layer(), Layer::Top);
    assert_eq!(underlay.keyboard_mode(), KeyboardMode::None);
    assert!(entry.grab_focus());
    assert!(entry.has_focus() || entry.focus_child().is_some());

    let hide = controller.begin_hide().unwrap();
    assert!(window.is_visible() && underlay.is_visible());
    let reopen = controller.begin_show().unwrap();
    assert!(!controller.finish_transition(&hide));
    assert!(controller.finish_transition(&reopen));
    assert!(window.is_visible() && underlay.is_visible());
    let hide = controller.begin_hide().unwrap();
    assert!(controller.finish_transition(&hide));
    assert!(!window.is_visible() && !underlay.is_visible());

    let other = popup(WindowRole::MessageTray, None);
    let unrelated = other.begin_show().unwrap();
    assert!(!controller.finish_transition(&unrelated));
    other.hide_now();
    assert!(!other.finish_transition(&unrelated));

    // Real GTK notify handlers can reopen while a completed hide is unmapping.
    // Its old completion must not hide the newly presented click-away surface.
    show(&controller);
    let reopen_on_hide = controller.clone();
    let reopened = Rc::new(Cell::new(false));
    let flag = reopened.clone();
    let handler = window.connect_visible_notify(move |window| {
        if !window.is_visible() && !flag.replace(true) {
            let token = reopen_on_hide.begin_show().unwrap();
            assert!(reopen_on_hide.finish_transition(&token));
        }
    });
    let hide = controller.begin_hide().unwrap();
    assert!(controller.finish_transition(&hide));
    window.disconnect(handler);
    assert!(reopened.get());
    assert_eq!(controller.visibility(), Visibility::Visible);
    assert!(window.is_visible() && underlay.is_visible());
    controller.close();
    assert_eq!(controller.visibility(), Visibility::Closed);
    assert!(!window.is_visible() && !underlay.is_visible());
    assert!(controller.begin_show().is_none());
}

fn coordinator_and_cleanup() {
    let coordinator = PopupCoordinator::default();
    let message = popup(
        WindowRole::MessageTray,
        Some(WindowRole::MessageTrayUnderlay),
    );
    assert_eq!(message.main().window().keyboard_mode(), KeyboardMode::None);
    let app = popup(WindowRole::AppSwitcher, None);
    coordinator.register(PopupRole::MessageTray, &message);
    coordinator.register(PopupRole::AppSwitcher, &app);
    show(&message);
    show(&app);
    let requests = coordinator.request_peer_hides(PopupEvent::QuickSettingsVisible);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].role, PopupRole::MessageTray);
    assert_eq!(message.visibility(), Visibility::Hiding);
    assert_eq!(app.visibility(), Visibility::Visible);
    assert!(message.main().window().is_visible());
    for request in requests {
        assert!(request.controller.finish_transition(&request.transition));
    }
    let main = message.main().clone();
    let underlay = message.underlay().unwrap().clone();
    let pending = message.begin_show().unwrap();
    drop(message);
    // Neither a coordinator registration nor an animation token owns a popup.
    assert!(main.is_closed() && underlay.is_closed());
    assert!(
        coordinator
            .request_peer_hides(PopupEvent::QuickSettingsVisible)
            .is_empty()
    );
    drop(pending);
    app.close();

    let retained = LayerWindow::new(WindowRole::Panel, None).unwrap();
    assert!(retained.window().auto_exclusive_zone_is_enabled());
    assert!(retained.window().is_anchor(Edge::Top));
    assert!(!retained.window().is_anchor(Edge::Bottom));
    let widget = retained.window().clone();
    retained.present();
    until(|| widget.is_mapped());
    let called = Rc::new(Cell::new(false));
    let flag = called.clone();
    retained.on_closed(move |_| flag.set(true));
    drop(retained);
    assert!(!widget.is_visible());
    assert!(
        !called.get(),
        "final owner drop must not trigger recreation callbacks"
    );
    let weak = widget.downgrade();
    drop(widget);
    until(|| weak.upgrade().is_none());
}

fn native_close_and_output_loss() -> Result<(), Box<dyn std::error::Error>> {
    let controller = popup(WindowRole::Dialog, None);
    show(&controller);
    controller.main().window().close();
    assert_eq!(controller.visibility(), Visibility::Closed);
    assert_eq!(
        controller.main().close_reason(),
        Some(CloseReason::Compositor)
    );

    let display = gtk::gdk::Display::default().unwrap();
    let monitors = display.monitors();
    if std::env::var_os("NIRI_SOCKET").is_none() {
        let before = monitors.n_items();
        assert!(
            std::process::Command::new("swaymsg")
                .arg("create_output")
                .status()?
                .success()
        );
        until(|| monitors.n_items() > before);
        let monitor = (0..monitors.n_items())
            .filter_map(|index| monitors.item(index).and_downcast::<gtk::gdk::Monitor>())
            .find(|monitor| monitor.connector().as_deref() == Some("HEADLESS-2"))
            .unwrap();
        let window = LayerWindow::new(WindowRole::Panel, Some(&monitor))?;
        window.present();
        until(|| window.window().is_mapped());
        assert_eq!(window.window().monitor().as_ref(), Some(&monitor));
        assert!(
            std::process::Command::new("swaymsg")
                .args(["output", "HEADLESS-2", "disable"])
                .status()?
                .success()
        );
        until(|| !monitor.is_valid() && window.is_closed());
        assert!(!window.window().is_visible());
        assert!(LayerWindow::new(WindowRole::Panel, Some(&monitor)).is_err());
    }
    // Exercise the same invalidation signal in nested Niri, whose test backend
    // has one persistent output, and verify it closes a paired underlay too.
    let monitor = monitors
        .item(0)
        .and_downcast::<gtk::gdk::Monitor>()
        .unwrap();
    let main = LayerWindow::new(WindowRole::QuickSettings, Some(&monitor))?;
    let underlay = LayerWindow::new(WindowRole::QuickSettingsUnderlay, Some(&monitor))?;
    let controller = VisibilityController::new(main, Some(underlay));
    show(&controller);
    monitor.emit_by_name::<()>("invalidate", &[]);
    assert_eq!(controller.visibility(), Visibility::Closed);
    assert_eq!(
        controller.main().close_reason(),
        Some(CloseReason::OutputRemoved)
    );
    assert!(controller.underlay().unwrap().is_closed());
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    adw::init()?;
    for _ in 0..3 {
        transition_and_focus();
        coordinator_and_cleanup();
    }
    native_close_and_output_loss()?;
    println!(
        "layer geometry, focus, interrupted transitions, explicit popup policy, output loss and ownership passed"
    );
    Ok(())
}
