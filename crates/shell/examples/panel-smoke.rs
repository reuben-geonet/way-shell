//! Exercise the composed Rust panel on an isolated Sway or Niri session.
use adw::prelude::*;
use gtk4_layer_shell::LayerShell;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};
use way_shell::{
    services::{
        clock::ClockService,
        notifications::{CloseReason, NotificationRequest, NotificationsService},
        wayland::WaylandService,
        wm::WindowManager,
    },
    ui::panel::{PanelAction, PanelServices, Panels, status::StatusServices},
};

fn settings(id: &str) -> gio::Settings {
    let source =
        gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)
            .unwrap();
    gio::Settings::new_full(
        &source.lookup(id, false).unwrap(),
        Some(&gio::memory_settings_backend_new()),
        None,
    )
}
fn until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "panel smoke test timed out");
        glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}
fn children(widget: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut children = Vec::new();
    let mut next = widget.first_child();
    while let Some(child) = next {
        next = child.next_sibling();
        children.push(child);
    }
    children
}
fn content(window: &adw::Window) -> gtk::CenterBox {
    window.content().unwrap().downcast().unwrap()
}
fn clock_widgets(window: &adw::Window) -> (gtk::Button, gtk::Label, gtk::Image) {
    let clock = content(window)
        .center_widget()
        .unwrap()
        .first_child()
        .unwrap();
    let children = children(&clock);
    let button = children[0].clone().downcast::<gtk::Button>().unwrap();
    let label = button.child().unwrap().downcast::<gtk::Label>().unwrap();
    (button, label, children[1].clone().downcast().unwrap())
}
fn workspace_list(window: &adw::Window) -> gtk::Box {
    let root = content(window)
        .start_widget()
        .unwrap()
        .first_child()
        .unwrap();
    let scroll = root
        .first_child()
        .unwrap()
        .downcast::<gtk::ScrolledWindow>()
        .unwrap();
    // GTK wraps a non-scrollable child in a viewport.
    let child = scroll.child().unwrap();
    child.clone().downcast::<gtk::Box>().unwrap_or_else(|_| {
        child
            .downcast::<gtk::Viewport>()
            .unwrap()
            .child()
            .unwrap()
            .downcast()
            .unwrap()
    })
}

fn status_button(window: &adw::Window) -> gtk::Widget {
    content(window)
        .end_widget()
        .unwrap()
        .last_child()
        .unwrap()
        .first_child()
        .unwrap()
}

fn reentrant_visibility(panels: &Panels) {
    type VisibilityCase = (
        &'static str,
        fn(&Panels, bool),
        fn(&adw::Window) -> gtk::Widget,
        &'static str,
    );
    let cases: [VisibilityCase; 3] = [
        (
            "message tray",
            Panels::set_message_tray_visible,
            |window| clock_widgets(window).0.upcast(),
            "panel-button-toggled",
        ),
        (
            "quick settings",
            Panels::set_quick_settings_visible,
            status_button,
            "panel-button-toggled",
        ),
        (
            "activities",
            Panels::set_activities_visible,
            |window| content(window).upcast(),
            "activities-visible",
        ),
    ];
    let mut stale = Vec::new();
    for (name, set_visible, widget, class) in cases {
        set_visible(panels, false);
        let first = widget(&panels.windows()[0]);
        let changed = Rc::new(Cell::new(false));
        let flag = changed.clone();
        let owner = panels.clone();
        let handler = first.connect_notify_local(Some("css-classes"), move |_, _| {
            if !flag.replace(true) {
                set_visible(&owner, false);
            }
        });
        set_visible(panels, true);
        first.disconnect(handler);
        assert!(changed.get(), "{name} did not emit its CSS notification");
        if panels
            .windows()
            .iter()
            .any(|window| widget(window).has_css_class(class))
        {
            stale.push(name);
        }
    }
    assert!(
        stale.is_empty(),
        "nested visibility changes left stale panel state: {stale:?}"
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    adw::init()?;
    let display = gtk::gdk::Display::default().unwrap();
    let context = glib::MainContext::default();
    let _guard = context.acquire()?;
    let wm_settings = settings("org.ldelossa.way-shell.window-manager");
    let manager = if let Some(path) = std::env::var_os("NIRI_SOCKET") {
        WindowManager::niri_with_settings(
            wm_settings.clone(),
            path.into(),
            PathBuf::from("/nonexistent/way-shell-panel-test"),
        )?
    } else {
        WindowManager::sway_with_settings(
            wm_settings.clone(),
            std::env::var_os("SWAYSOCK").unwrap().into(),
            PathBuf::from("/nonexistent/way-shell-panel-test"),
        )?
    };
    let wayland = WaylandService::with_settings(wm_settings)?;
    let clock = ClockService::new();
    let notifications = NotificationsService::new();
    until(|| manager.is_connected() && wayland.is_ready() && notifications.state().available);
    let panel_settings = settings("org.ldelossa.way-shell.panel");
    panel_settings.set_string("clock-format", "%H.%M")?;
    let notification_settings = settings("org.ldelossa.way-shell.notifications");
    notification_settings.set_boolean("do-not-disturb", true)?;
    for iteration in 0..3 {
        let actions = Rc::new(RefCell::new(Vec::new()));
        let received = actions.clone();
        let callback_owner = Rc::new(());
        let weak_callback = Rc::downgrade(&callback_owner);
        let panels = Panels::new(
            &display,
            PanelServices {
                clock: clock.clone(),
                notifications: notifications.clone(),
                manager: manager.clone(),
                status: StatusServices {
                    // Missing optional daemons have ordinary unavailable state.
                    // Their actual D-Bus updates have separate status-widget coverage.
                    audio: glib::Object::new(),
                    network: glib::Object::new(),
                    power: glib::Object::new(),
                    logind: glib::Object::new(),
                    wayland: wayland.clone(),
                },
                tray: None,
            },
            panel_settings.clone(),
            notification_settings.clone(),
            move |action| {
                let _owner = &callback_owner;
                received.borrow_mut().push(action);
            },
        )?;
        until(|| {
            panels.windows().len() == display.monitors().n_items() as usize
                && panels.windows().iter().all(|window| window.is_mapped())
        });
        let window = panels.windows().remove(0);
        let (button, label, indicator) = clock_widgets(&window);
        assert!(indicator.is_visible());
        assert_eq!(
            indicator.icon_name().as_deref(),
            Some("notifications-disabled-symbolic")
        );
        let now = glib::DateTime::from_utc(2026, 9, 16, 12, 35, 0.0)?;
        clock.emit_by_name::<()>("tick", &[&now]);
        assert_eq!(label.label(), "12.35");
        panels.set_message_tray_visible(true);
        assert!(button.has_css_class("panel-button-toggled"));
        panels.set_activities_visible(true);
        assert!(content(&window).has_css_class("activities-visible"));
        button.emit_clicked();
        assert_eq!(
            actions.borrow().as_slice(),
            &[PanelAction::ToggleMessageTray]
        );

        let help = content(&window)
            .end_widget()
            .unwrap()
            .last_child()
            .unwrap()
            .prev_sibling()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        assert_eq!(help.tooltip_text().as_deref(), Some("Keyboard shortcuts"));
        help.emit_clicked();
        assert!(
            matches!(actions.borrow().last(), Some(PanelAction::ToggleShortcuts(monitor)) if monitor.is_valid())
        );

        notification_settings.set_boolean("do-not-disturb", false)?;
        assert!(!indicator.is_visible());
        let id = notifications.send_internal(NotificationRequest {
            summary: "Panel fixture".into(),
            ..Default::default()
        })?;
        assert!(indicator.is_visible());
        notifications.close(id, CloseReason::Dismissed);
        assert!(!indicator.is_visible());

        // A settings observer can reverse DND while its icon is being updated.
        let changed = Rc::new(Cell::new(false));
        let flag = changed.clone();
        let settings = notification_settings.clone();
        let handler = indicator.connect_icon_name_notify(move |_| {
            if !flag.replace(true) {
                settings.set_boolean("do-not-disturb", false).unwrap();
            }
        });
        notification_settings.set_boolean("do-not-disturb", true)?;
        indicator.disconnect(handler);
        assert!(changed.get());
        assert!(
            !indicator.is_visible(),
            "reentrant DND changes must win over the old snapshot"
        );
        assert_eq!(
            indicator.icon_name().as_deref(),
            Some("preferences-system-notifications-symbolic")
        );

        // Workspace labels can notify code which immediately publishes another inventory.
        let list = workspace_list(&window);
        until(|| !children(&list).is_empty());
        let count = children(&list).len();
        let workspace = list
            .first_child()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        workspace.set_label("force a label update");
        let nested = Rc::new(Cell::new(false));
        let flag = nested.clone();
        let source = manager.clone();
        let handler = workspace.connect_label_notify(move |_| {
            if !flag.replace(true) {
                source.emit_by_name::<()>("workspaces-changed", &[]);
            }
        });
        manager.emit_by_name::<()>("workspaces-changed", &[]);
        workspace.disconnect(handler);
        assert!(nested.get());
        assert_eq!(
            children(&list).len(),
            count,
            "reentrant inventories must not duplicate workspace buttons"
        );
        // A single surface must also keep the innermost requested visibility.
        reentrant_visibility(&panels);
        panels.set_message_tray_visible(true);
        panels.set_activities_visible(true);

        if iteration == 0 && std::env::var_os("NIRI_SOCKET").is_none() {
            let before = panels.windows().len();
            assert!(
                std::process::Command::new("swaymsg")
                    .arg("create_output; create_output")
                    .status()?
                    .success()
            );
            until(|| {
                panels.windows().len() == before + 2
                    && panels.windows().iter().all(|window| window.is_mapped())
            });
            for window in panels.windows() {
                assert!(
                    clock_widgets(&window)
                        .0
                        .has_css_class("panel-button-toggled")
                );
                assert!(content(&window).has_css_class("activities-visible"));
            }
            // The first surface can synchronously reverse a global change;
            // later surfaces must receive the latest state as well.
            reentrant_visibility(&panels);
            assert!(
                std::process::Command::new("swaymsg")
                    .arg("output HEADLESS-2 disable; output HEADLESS-3 disable")
                    .status()?
                    .success()
            );
            until(|| panels.windows().len() == before);

            // Output destruction can synchronously ask the whole panel set to stop.
            assert!(
                std::process::Command::new("swaymsg")
                    .arg("create_output")
                    .status()?
                    .success()
            );
            until(|| {
                panels.windows().len() == before + 1
                    && panels.windows().iter().all(|window| window.is_mapped())
            });
            let removed = panels
                .windows()
                .into_iter()
                .find(|candidate| candidate != &window)
                .unwrap();
            let connector = removed.monitor().unwrap().connector().unwrap();
            let stopped = Rc::new(Cell::new(false));
            let flag = stopped.clone();
            let stopper = panels.clone();
            let _finalizer = clock_widgets(&removed)
                .1
                .add_weak_ref_notify_local(move || {
                    flag.set(true);
                    stopper.stop();
                });
            drop(removed);
            assert!(
                std::process::Command::new("swaymsg")
                    .args(["output", &connector, "disable"])
                    .status()?
                    .success()
            );
            until(|| stopped.get() && display.monitors().n_items() as usize == before);
            assert!(
                panels.windows().is_empty(),
                "stopping during output removal must not recreate other panels"
            );
        }
        let windows = panels.windows();
        panels.stop();
        panels.stop();
        assert!(panels.windows().is_empty());
        assert!(windows.iter().all(|window| !window.is_visible()));
        button.emit_clicked();
        assert_eq!(actions.borrow().len(), 1);
        label.set_label("released");
        clock.emit_by_name::<()>("tick", &[&now]);
        assert_eq!(label.label(), "released");
        drop(panels);
        assert!(weak_callback.upgrade().is_none());
        notification_settings.set_boolean("do-not-disturb", true)?;
    }
    clock.set_enabled(false);
    notifications.stop();
    println!(
        "panel clock, notification state, workspace reentrancy, hotplug and controller cleanup passed"
    );
    Ok(())
}
