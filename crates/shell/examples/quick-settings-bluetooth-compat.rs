//! Bluetooth layout and interactions against private BlueZ/rfkill fixtures.
use adw::prelude::*;
use std::time::Duration;
use way_shell::{
    services::theme::{Theme, ThemeService},
    ui::quick_settings::{
        QuickSettingsWindow,
        bluetooth::BluetoothControls,
        grid::{Grid, GridButton},
    },
};
#[path = "../tests/common/bluetooth.rs"]
mod fixture;
use fixture::*;
fn descendants(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut all = Vec::new();
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        all.extend(descendants(&widget));
        all.push(widget);
    }
    all
}
fn label(root: &impl IsA<gtk::Widget>, text: &str) -> gtk::Label {
    descendants(root)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Label>().ok())
        .find(|l| l.label() == text)
        .unwrap_or_else(|| panic!("Missing label {text}"))
}
fn button(root: &impl IsA<gtk::Widget>, text: &str) -> gtk::Button {
    let mut widget: gtk::Widget = label(root, text).upcast();
    loop {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
            return button;
        }
        widget = widget.parent().unwrap();
    }
}
fn animate(c: &glib::MainContext) {
    c.block_on(glib::timeout_future(Duration::from_millis(450)));
}
fn capture(window: &QuickSettingsWindow, theme: Theme, state: &str) {
    let Some(directory) = std::env::var_os("WAY_SHELL_TEST_CAPTURE_DIR") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    let snapshot = gtk::Snapshot::new();
    let paintable = gtk::WidgetPaintable::new(Some(window.window()));
    paintable.snapshot(
        &snapshot,
        f64::from(window.window().width()),
        f64::from(window.window().height()),
    );
    let node = snapshot.to_node().unwrap();
    let texture = window
        .window()
        .renderer()
        .unwrap()
        .render_texture(&node, None);
    texture
        .save_to_png(directory.join(format!("{theme:?}-{state}.png")))
        .unwrap();
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    adw::init()?;
    let c = glib::MainContext::default();
    let _guard = c.acquire()?;
    let source =
        gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)?;
    for theme in [Theme::Light, Theme::Dark] {
        let f = Fixture::new(&c);
        let settings = gio::Settings::new_full(
            &source
                .lookup("org.ldelossa.way-shell.system", false)
                .unwrap(),
            Some(&gio::memory_settings_backend_new()),
            None,
        );
        let styles = ThemeService::with_settings(
            settings.clone(),
            std::path::PathBuf::from("/nonexistent/bluetooth-fixture"),
        );
        styles.attach_display(&gtk::gdk::Display::default().unwrap());
        styles.set_theme(theme)?;
        let window = QuickSettingsWindow::new()?;
        let controls = BluetoothControls::new(f.service.clone(), settings.clone(), &window);
        let grid = Grid::new(|_| {});
        let other = GridButton::new("Other", None, "dialog-information-symbolic", None);
        grid.set_buttons(vec![controls.button().unwrap(), other]);
        window.content().append(grid.widget());
        window.show();
        animate(&c);
        controls.tile().toggle_menu();
        animate(&c);
        assert!(controls.tile().revealer().reveals_child());
        assert!(label(controls.tile().widget(), "Zebra mouse").is_visible());
        let width = window.window().width();
        assert!(width <= 550, "Bluetooth widens the settings panel: {width}");
        capture(&window, theme, "devices");
        let mouse = button(controls.menu().widget(), "Zebra mouse");
        let headphones = button(controls.menu().widget(), "Alpha headphones");
        let first = mouse.prev_sibling();
        assert!(first.is_none());
        f.fake.state.hold_device.set(true);
        mouse.emit_clicked();
        pump(&c);
        assert!(!mouse.is_sensitive());
        let spinner = descendants(&mouse)
            .into_iter()
            .find_map(|w| w.downcast::<gtk::Spinner>().ok())
            .unwrap();
        assert!(spinner.is_spinning());
        f.fake.set(MOUSE, DEVICE, "Connected", false);
        pump(&c);
        f.fake
            .finish(Some("org.freedesktop.DBus.Error.NoReply"), false);
        pump(&c);
        assert!(!controls.menu().banner().reveals_child());
        assert!(mouse.is_sensitive());
        assert!(!spinner.is_spinning());
        assert!(
            mouse.prev_sibling().is_none(),
            "Rows move only on menu reopening"
        );
        let subtitle = descendants(controls.tile().widget())
            .into_iter()
            .find_map(|w| {
                w.downcast::<gtk::Label>()
                    .ok()
                    .filter(|l| l.has_css_class("quick-settings-grid-button-subtitle"))
            })
            .unwrap();
        assert!(!subtitle.is_visible());
        controls.tile().toggle_menu();
        controls.tile().toggle_menu();
        animate(&c);
        assert!(headphones.prev_sibling().is_none());
        // Fail first, then recover through the later Connected property.
        mouse.emit_clicked();
        pump(&c);
        f.fake
            .finish(Some("org.freedesktop.DBus.Error.NoReply"), false);
        pump(&c);
        assert!(controls.menu().banner().reveals_child());
        f.fake.set(MOUSE, DEVICE, "Connected", true);
        pump(&c);
        assert!(!controls.menu().banner().reveals_child());
        f.fake.set(HEADPHONES, DEVICE, "Connected", true);
        pump(&c);
        assert!(label(controls.tile().widget(), "2 Connected").is_visible());
        // A single row must determine the viewport's natural height.
        f.fake.remove(HEADPHONES);
        pump(&c);
        animate(&c);
        let scroll = descendants(controls.menu().widget())
            .into_iter()
            .find_map(|w| w.downcast::<gtk::ScrolledWindow>().ok())
            .unwrap();
        assert_eq!(scroll.vscrollbar_policy(), gtk::PolicyType::Never);
        assert!(
            scroll.height()
                <= scroll
                    .child()
                    .unwrap()
                    .measure(gtk::Orientation::Vertical, scroll.width())
                    .1,
            "Single-row viewport retains scrollbar padding: viewport {}, row {}, natural {}",
            scroll.height(),
            mouse.height(),
            scroll
                .child()
                .unwrap()
                .measure(gtk::Orientation::Vertical, scroll.width())
                .1
        );
        capture(&window, theme, "one-device");
        let footer = button(controls.menu().widget(), "Bluetooth Settings");
        assert!(
            footer.compute_bounds(controls.menu().widget()).unwrap().y()
                >= scroll.compute_bounds(controls.menu().widget()).unwrap().y()
        );
        // Long launch failures wrap inside the panel and remain dismissible.
        settings.set_string(
            "bluetooth-settings-command",
            &format!("no-such-manager-{}", "x".repeat(150)),
        )?;
        footer.emit_clicked();
        animate(&c);
        assert!(controls.menu().banner().reveals_child());
        assert!(window.is_visible());
        assert!(window.window().width() <= width + 2);
        controls
            .menu()
            .banner()
            .child()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        assert!(!controls.menu().banner().reveals_child());
        capture(&window, theme, "error-dismissed");
        // Both power directions retain the acquiring icon and permit reversal.
        f.fake.state.hold_power.set(true);
        controls.tile().toggle().emit_clicked();
        wait(&c, || !f.fake.state.held.borrow().is_empty());
        assert_eq!(
            controls.tile().toggle().tooltip_text().as_deref(),
            Some("Turning Bluetooth off…")
        );
        assert!(controls.tile().toggle().is_sensitive());
        assert!(!mouse.is_sensitive());
        assert!(
            descendants(controls.tile().widget())
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Image>().ok())
                .any(|i| i.icon_name().as_deref() == Some("bluetooth-acquiring-symbolic"))
        );
        controls.tile().toggle().emit_clicked();
        pump(&c);
        assert_eq!(
            controls.tile().toggle().tooltip_text().as_deref(),
            Some("Turning Bluetooth on…")
        );
        f.fake.state.hold_power.set(false);
        f.fake.finish(None, true);
        f.settle(&c);
        assert!(f.service.state().powered);
        f.service.set_powered(false);
        f.settle(&c);
        animate(&c);
        assert!(!scroll.is_visible());
        assert!(
            label(
                controls.menu().widget(),
                "Turn on Bluetooth to connect to devices"
            )
            .is_visible()
        );
        assert!(window.window().width() <= width + 2);
        capture(&window, theme, "off");
        f.service.set_powered(true);
        f.settle(&c);
        // More devices scroll within the fixed limit and keep the footer outside.
        for n in 5..15 {
            f.fake.add(
                &format!("/org/bluez/hci0/dev_{n:02}"),
                device(
                    &format!("Long device name {} {n}", "x".repeat(80)),
                    HCI,
                    true,
                    false,
                    false,
                    "00001812-0000-1000-8000-00805f9b34fb",
                ),
            );
        }
        pump(&c);
        animate(&c);
        assert_eq!(scroll.vscrollbar_policy(), gtk::PolicyType::Automatic);
        assert!(scroll.height() <= 240);
        assert!(window.window().width() <= width + 2);
        capture(&window, theme, "many-devices");
        f.fake.ownership(false);
        wait(&c, || !f.service.state().ready);
        pump(&c);
        assert!(!controls.tile().toggle().is_sensitive());
        assert!(label(controls.menu().widget(), "Bluetooth service unavailable").is_visible());
        f.fake.ownership(true);
        wait(&c, || f.service.state().ready);
        pump(&c);
        assert!(controls.tile().toggle().is_sensitive());
        settings.set_string("bluetooth-settings-command", "true 'a b' '$(false)'")?;
        footer.emit_clicked();
        animate(&c);
        assert!(!window.is_visible());
        settings.set_string(
            "bluetooth-settings-command",
            "no-such-bluetooth-manager-92841",
        )?;
        footer.emit_clicked();
        pump(&c);
        assert!(controls.menu().banner().reveals_child());
        assert!(!window.is_visible());
        // Hardware disappears while closed: no stale tile, no popup on error.
        f.fake.remove(HCI);
        radio(&f.radio, 4, 1, false, false);
        pump(&c);
        assert!(controls.button().is_none());
        f.service.set_powered(true);
        pump(&c);
        assert!(!window.is_visible());
        controls.stop();
        grid.stop();
        window.close();
        f.service.stop();
    }
    println!("Bluetooth layout, actions, power transitions and recovery passed in both themes");
    Ok(())
}
