//! Real pointer routing across outputs; run only in the private Sway harness.
use adw::prelude::*;
use gtk4_layer_shell::LayerShell;
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};
use way_shell::ui::{
    message_tray::window::MessageTrayWindow, quick_settings::QuickSettingsWindow,
    window::Visibility,
};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop, protocol::wl_registry};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

#[derive(Default)]
struct PointerState {
    manager: Option<ZwlrVirtualPointerManagerV1>,
}
impl Dispatch<wl_registry::WlRegistry, ()> for PointerState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
            && interface == "zwlr_virtual_pointer_manager_v1"
        {
            state.manager = Some(registry.bind(name, 1, qh, ()));
        }
    }
}
delegate_noop!(PointerState: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(PointerState: ignore ZwlrVirtualPointerV1);

struct Pointer {
    connection: Connection,
    pointer: ZwlrVirtualPointerV1,
    started: Instant,
}
impl Pointer {
    fn new() -> Self {
        let connection = Connection::connect_to_env().unwrap();
        let mut queue = connection.new_event_queue();
        let qh = queue.handle();
        connection.display().get_registry(&qh, ());
        let mut state = PointerState::default();
        queue.roundtrip(&mut state).unwrap();
        let pointer = state.manager.unwrap().create_virtual_pointer(None, &qh, ());
        connection.flush().unwrap();
        Self {
            connection,
            pointer,
            started: Instant::now(),
        }
    }
    fn click(&self, x: i32, y: i32, width: i32, height: i32) {
        use wayland_client::protocol::wl_pointer::ButtonState;
        let time = self.started.elapsed().as_millis() as u32;
        self.pointer
            .motion_absolute(time, x as u32, y as u32, width as u32, height as u32);
        self.pointer.frame();
        self.pointer.button(time, 0x110, ButtonState::Pressed);
        self.pointer.frame();
        self.pointer.button(time + 1, 0x110, ButtonState::Released);
        self.pointer.frame();
        self.connection.flush().unwrap();
    }
}

fn until(description: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out: {description}");
        glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}
fn settle() {
    glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(350)));
}
fn sway(command: &str) {
    let result = std::process::Command::new("swaymsg")
        .args(["-r", command])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{command}: {} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let reply: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(
        reply
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["success"] == true),
        "{reply}"
    );
}
fn monitors() -> Vec<gtk::gdk::Monitor> {
    let model = gtk::gdk::Display::default().unwrap().monitors();
    (0..model.n_items())
        .map(|i| model.item(i).and_downcast().unwrap())
        .collect()
}

fn main() {
    assert!(
        std::env::var_os("NIRI_SOCKET").is_none(),
        "this probe needs two Sway outputs"
    );
    adw::init().unwrap();
    assert_eq!(
        monitors().len(),
        1,
        "start with the standard single-output harness"
    );
    sway("create_output");
    until("second monitor", || monitors().len() == 2);
    sway("output * resolution 1280x800");
    until("output sizes", || {
        monitors().iter().all(|m| m.geometry().width() == 1280)
    });
    let monitors = monitors();
    let width = monitors
        .iter()
        .map(|m| m.geometry().x() + m.geometry().width())
        .max()
        .unwrap();
    let height = monitors
        .iter()
        .map(|m| m.geometry().y() + m.geometry().height())
        .max()
        .unwrap();
    let pointer = Pointer::new();
    let clicks = Rc::new(Cell::new(0));
    let mut apps = Vec::new();
    for (index, monitor) in monitors.iter().enumerate() {
        let title = format!("click-away-target-{index}");
        let app = gtk::Window::builder().title(&title).build();
        let button = gtk::Button::with_label("Underlying application");
        let clicks = clicks.clone();
        button.connect_clicked(move |_| clicks.set(clicks.get() + 1));
        app.set_child(Some(&button));
        app.present();
        until("application mapped", || app.is_mapped());
        until("compositor sees application", || {
            let tree = std::process::Command::new("swaymsg")
                .args(["-r", "-t", "get_tree"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&tree.stdout).contains(&title)
        });
        sway(&format!(
            "[title=\"{title}\"] move container to output {}",
            monitor.connector().unwrap()
        ));
        apps.push(app);
    }
    settle();

    for empty_desktop in [false, true] {
        if empty_desktop {
            for app in &apps {
                app.set_visible(false);
            }
            settle();
        }
        for message_tray in [false, true] {
            for (index, monitor) in monitors.iter().enumerate() {
                let quick = (!message_tray).then(|| QuickSettingsWindow::new().unwrap());
                let message = message_tray.then(|| MessageTrayWindow::new().unwrap());
                let (window, content, visibility) = if let Some(view) = &quick {
                    (view.window(), view.content(), view.visibility_controller())
                } else {
                    let view = message.as_ref().unwrap();
                    (view.window(), view.content(), view.visibility_controller())
                };
                window.set_monitor(Some(monitor));
                let inside = gtk::Button::with_label("Inside popup");
                inside.set_hexpand(true);
                inside.set_vexpand(true);
                let inside_clicks = Rc::new(Cell::new(0));
                let seen = inside_clicks.clone();
                inside.connect_clicked(move |_| seen.set(seen.get() + 1));
                content.append(&inside);
                if let Some(view) = &quick {
                    view.show();
                } else {
                    message.as_ref().unwrap().show();
                }
                until("popup visible", || {
                    visibility.visibility() == Visibility::Visible
                });
                settle();

                // Real input inside the overlay must not reach its underlay.
                let rect = monitor.geometry();
                let x = if message_tray {
                    rect.x() + rect.width() / 2
                } else {
                    rect.x() + rect.width() - 20 - window.width() / 2
                };
                pointer.click(x, rect.y() + 8 + window.height() / 2, width, height);
                until("inside click", || inside_clicks.get() == 1);
                assert_eq!(visibility.visibility(), Visibility::Visible);

                let other = monitors[1 - index].geometry();
                let before = clicks.get();
                pointer.click(
                    other.x() + other.width() / 2,
                    other.y() + other.height() / 2,
                    width,
                    height,
                );
                until("cross-monitor click dismisses popup", || {
                    visibility.visibility() == Visibility::Hidden
                });
                assert!(!window.is_mapped());
                assert!(
                    visibility
                        .underlays()
                        .unwrap()
                        .windows()
                        .iter()
                        .all(|surface| !surface.window().is_mapped())
                );
                assert_eq!(clicks.get(), before, "dismissal click must be consumed");
                if !empty_desktop {
                    pointer.click(
                        other.x() + other.width() / 2,
                        other.y() + other.height() / 2,
                        width,
                        height,
                    );
                    until("next click reaches application", || {
                        clicks.get() == before + 1
                    });
                }
                // Retain same-monitor click-away behavior on reopening as well.
                if let Some(view) = &quick {
                    view.show();
                } else {
                    message.as_ref().unwrap().show();
                }
                until("popup reopened", || {
                    visibility.visibility() == Visibility::Visible
                });
                pointer.click(rect.x() + 20, rect.y() + rect.height() - 20, width, height);
                until("same-monitor dismissal", || {
                    visibility.visibility() == Visibility::Hidden
                });
                if let Some(view) = &quick {
                    view.close();
                } else {
                    message.as_ref().unwrap().close();
                }
            }
        }
    }
    for app in apps {
        app.destroy();
    }
    hotplug_and_reentry();
    println!(
        "real cross-monitor dismissal, click consumption, hotplug and reentrant teardown passed"
    );
}

fn hotplug_and_reentry() {
    let view = QuickSettingsWindow::new().unwrap();
    view.content()
        .append(&gtk::Label::new(Some("Monitor lifecycle")));
    let visibility = view.visibility_controller();
    let underlays = visibility.underlays().unwrap();
    let initial_count = monitors().len();
    assert_eq!(underlays.windows().len(), initial_count);
    assert!(
        underlays
            .windows()
            .iter()
            .all(|surface| !surface.window().is_visible())
    );

    view.show();
    until("popup visible before hotplug", || {
        visibility.visibility() == Visibility::Visible
    });
    sway("create_output");
    until("hotplugged underlay mapped", || {
        underlays.windows().len() == initial_count + 1
            && underlays
                .windows()
                .iter()
                .all(|surface| surface.window().is_mapped())
    });
    let added = monitors()
        .into_iter()
        .find(|m| m.connector().as_deref() == Some("HEADLESS-3"))
        .unwrap();
    let retained = underlays
        .windows()
        .into_iter()
        .find(|surface| surface.window().monitor().as_ref() == Some(&added))
        .unwrap();
    let removed_button = retained
        .window()
        .content()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    sway("output HEADLESS-3 disable");
    until("output removal dismisses popup", || {
        visibility.visibility() == Visibility::Hidden && underlays.windows().len() == initial_count
    });
    assert!(retained.is_closed());

    // Add while hidden, then reopen; retained buttons from removed outputs
    // must not act on the replacement surfaces.
    sway("output HEADLESS-3 enable");
    until("hidden hotplug", || {
        underlays.windows().len() == initial_count + 1
    });
    assert!(
        underlays
            .windows()
            .iter()
            .all(|surface| !surface.window().is_visible())
    );
    view.show();
    view.hide();
    view.show();
    until("interrupted fade reopened", || {
        visibility.visibility() == Visibility::Visible
    });
    removed_button.emit_clicked();
    settle();
    assert_eq!(visibility.visibility(), Visibility::Visible);

    // Reopen while the first blocker is being hidden. The obsolete hide must
    // not continue through the remaining outputs and unmap the new blockers.
    let surfaces = underlays.windows();
    let reopened = Rc::new(Cell::new(false));
    let flag = reopened.clone();
    let weak = Rc::downgrade(&view);
    let handler = surfaces[0].window().connect_visible_notify(move |window| {
        if !window.is_visible() && !flag.replace(true) {
            weak.upgrade().unwrap().show();
        }
    });
    view.hide();
    until("underlay hide reentry", || {
        reopened.get() && visibility.visibility() == Visibility::Visible
    });
    surfaces[0].window().disconnect(handler);
    assert!(surfaces.iter().all(|surface| surface.window().is_visible()));

    let retained_set = underlays.clone();
    let retained_buttons = view.underlay_buttons();
    let main = view.window().clone();
    drop(view);
    assert!(!main.is_visible());
    assert!(surfaces.iter().all(|surface| surface.is_closed()));
    for button in retained_buttons {
        button.emit_clicked();
    }
    sway("create_output");
    until("monitor added after teardown", || {
        monitors().len() == initial_count + 2
    });
    assert!(
        retained_set.windows().is_empty(),
        "closed set must not recreate blockers"
    );
}
