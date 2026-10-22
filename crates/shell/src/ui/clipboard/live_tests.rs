//! Explicitly run in a private Sway/Niri session with isolated XDG directories.
use super::*;
use crate::services::{clipboard::now, wayland::WaylandService, wm::Backend};
use std::{path::PathBuf, time::Instant};
use way_shell_clipboard::{Content, Retention};

#[track_caller]
fn wait(context: &glib::MainContext, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !condition() {
        assert!(Instant::now() < deadline, "Clipboard fixture timed out");
        context.block_on(glib::timeout_future(Duration::from_millis(10)));
    }
}

#[test]
#[ignore = "requires an isolated Wayland compositor"]
fn clipboard_capture_privacy_restore_profiles_and_cleanup() {
    adw::init().unwrap();
    crate::resources::get();
    let context = glib::MainContext::default();
    let display = gdk::Display::default().unwrap();
    adw::StyleManager::for_display(&display).set_color_scheme(adw::ColorScheme::ForceDark);
    let provider = gtk::CssProvider::new();
    provider.load_from_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/theme/way-shell-dark.css"),
    );
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let schemas =
        gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)
            .unwrap();
    let backend = gio::memory_settings_backend_new();
    let setting =
        |name| gio::Settings::new_full(&schemas.lookup(name, false).unwrap(), Some(&backend), None);
    let wm = setting("org.ldelossa.way-shell.window-manager");
    let settings = setting("org.ldelossa.way-shell.clipboard");
    settings.set_string("database-backend", "memory").unwrap();
    settings
        .child("link-item")
        .set_boolean("show-link-preview", false)
        .unwrap();
    let directory = glib::user_runtime_dir().join("clipboard-test");
    std::fs::create_dir(&directory).unwrap();
    let kind = if std::env::var_os("NIRI_SOCKET").is_some() {
        Backend::Niri
    } else {
        Backend::Sway
    };
    let manager = match kind {
        Backend::Sway => WindowManager::sway_with_settings(
            wm.clone(),
            std::env::var_os("SWAYSOCK").unwrap().into(),
            directory.clone(),
        ),
        Backend::Niri => WindowManager::niri_with_settings(
            wm.clone(),
            std::env::var_os("NIRI_SOCKET").unwrap().into(),
            directory.clone(),
        ),
    }
    .unwrap();
    let wayland = WaylandService::with_settings(wm.clone()).unwrap();
    let service = ClipboardService::with_settings(wayland.clone(), settings.clone(), None);
    let shortcuts = ShortcutsService::new(manager.clone(), wm);
    let editor = gtk::TextView::new();
    let sample = gtk::Window::builder()
        .title("Clipboard paste target")
        .default_width(500)
        .default_height(300)
        .child(&editor)
        .build();
    sample.present();
    editor.grab_focus();
    wait(&context, || {
        service.ready()
            && manager.is_connected()
            && wayland.has_clipboard()
            && wayland.focused_toplevel().is_some()
    });
    // Keep a keyboard attached: GDK needs a live seat and an input serial to own a selection.
    wayland.paste(false).unwrap();
    context.block_on(glib::timeout_future(Duration::from_millis(100)));
    wayland.paste(false).unwrap();
    context.block_on(glib::timeout_future(Duration::from_millis(30)));
    let clipboard = display.clipboard();
    clipboard.set_text("First captured clipboard item");
    wait(&context, || {
        service
            .entries()
            .iter()
            .any(|entry| entry.content.text == "First captured clipboard item")
    });
    service.incognito(true).unwrap();
    clipboard.set_text("Private content must never be stored");
    context.block_on(glib::timeout_future(Duration::from_millis(150)));
    assert_eq!(service.entries().len(), 1);
    service.incognito(false).unwrap();
    clipboard.set_text("Second captured clipboard item");
    wait(&context, || service.entries().len() == 2);
    let first = service
        .entries()
        .iter()
        .find(|entry| entry.content.text.starts_with("First"))
        .unwrap()
        .id;
    service.restore(first).unwrap();
    wait(&context, || {
        clipboard
            .formats()
            .contain_mime_type("application/x-way-shell-clipboard")
    });
    assert_eq!(
        context
            .block_on(clipboard.read_text_future())
            .unwrap()
            .as_deref(),
        Some("First captured clipboard item")
    );
    context.block_on(glib::timeout_future(Duration::from_millis(100)));
    assert_eq!(
        service.entries().len(),
        2,
        "restoration must not recapture its own offer"
    );
    settings.set_string("clipboard-history", "clear").unwrap();
    context.block_on(glib::timeout_future(Duration::from_millis(30)));
    assert_eq!(
        service.entries().len(),
        2,
        "changing restart policy keeps current history"
    );
    let image =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 64, 64).unwrap();
    image.fill(0x3584e4ff);
    let image = image.save_to_bufferv("png", &[]).unwrap();
    service.change(move |store| {
        for (mime, data) in [
            ("text/plain", "#3584e4"),
            ("text/plain", "👨‍👨‍👶‍👧"),
            ("text/plain", "https://example.org/clipboard"),
            (
                "text/plain",
                "#!/usr/bin/env python3\nfor i in range(5):\n    print(i)",
            ),
            ("text/uri-list", "file:///tmp/clipboard-example.txt"),
            ("text/uri-list", "file:///tmp/one\nfile:///tmp/two"),
        ] {
            store.insert(
                way_shell_clipboard::classify(mime, data.as_bytes().to_vec(), 1)?,
                now(),
                Retention::default(),
            )?;
        }
        store.insert(
            Content {
                kind: Kind::Image,
                mime: "image/png".into(),
                text: String::new(),
                bytes: image,
                metadata: serde_json::Value::Null,
            },
            now(),
            Retention::default(),
        )?;
        store.entries()
    });
    wait(&context, || service.entries().len() == 9);
    let dialog = ClipboardDialog::new(service.clone(), manager.clone(), shortcuts.clone()).unwrap();
    dialog.show(None).unwrap();
    assert!(!dialog.light_scheme());
    dialog
        .style_manager
        .set_color_scheme(adw::ColorScheme::ForceLight);
    assert!(dialog.light_scheme());
    dialog
        .style_manager
        .set_color_scheme(adw::ColorScheme::ForceDark);
    wait(&context, || {
        dialog.panel.width() > 100 && dialog.model.n_items() == 9
    });
    context.block_on(glib::timeout_future(Duration::from_millis(600)));
    let _ = std::process::Command::new("grim")
        .arg(directory.join("default.png"))
        .status();
    dialog.search.set_text("First captured");
    assert_eq!(dialog.model.n_items(), 1);
    let mut entry = service.entry(first).unwrap();
    entry.pinned = true;
    entry.tag = Some("blue".into());
    service.update(entry);
    wait(&context, || {
        service.entry(first).is_some_and(|entry| entry.pinned)
    });
    dialog.hide();
    preferences::apply_profile(&settings, true);
    dialog.show(None).unwrap();
    wait(&context, || {
        dialog.list.orientation() == gtk::Orientation::Vertical
    });
    context.block_on(glib::timeout_future(Duration::from_millis(500)));
    let _ = std::process::Command::new("grim")
        .arg(directory.join("compact.png"))
        .status();
    dialog.hide();
    preferences::apply_profile(&settings, false);
    dialog.open_preferences();
    let prefs = dialog.preferences.borrow().clone().unwrap();
    wait(&context, || prefs.is_mapped());
    context.block_on(glib::timeout_future(Duration::from_millis(200)));
    prefs.close();
    wait(&context, || dialog.preferences.borrow().is_none());
    sample.present();
    editor.grab_focus();
    wait(&context, || {
        wayland
            .focused_toplevel()
            .is_some_and(|top| top.title.as_deref() == Some("Clipboard paste target"))
    });
    dialog.show(None).unwrap();
    context.block_on(glib::timeout_future(Duration::from_millis(100)));
    dialog.activate(first, false);
    wait(&context, || editor.buffer().char_count() > 0);
    assert_eq!(
        editor.buffer().text(
            &editor.buffer().start_iter(),
            &editor.buffer().end_iter(),
            true
        ),
        "First captured clipboard item"
    );
    service.clear(false);
    wait(&context, || service.entries().len() == 1);
    service.delete(first, false);
    context.block_on(glib::timeout_future(Duration::from_millis(50)));
    assert_eq!(service.entries().len(), 1);
    service.delete(first, true);
    wait(&context, || service.entries().is_empty());
    let unavailable = directory.join("unavailable.db");
    std::fs::create_dir(&unavailable).unwrap();
    settings
        .set_string("database-location", unavailable.to_str().unwrap())
        .unwrap();
    settings.set_string("database-backend", "sqlite").unwrap();
    wait(&context, || service.error().is_some());
    dialog.show(None).unwrap();
    assert!(dialog.retry.is_sensitive());
    std::fs::remove_dir(&unavailable).unwrap();
    dialog.retry.emit_clicked();
    wait(&context, || service.ready() && service.error().is_none());
    assert!(service.entries().is_empty());
    dialog.close();
    service.stop();
    wayland.stop();
    shortcuts.stop();
    manager.stop();
    sample.destroy();
    assert!(!dialog.progress.visible());
    println!(
        "Clipboard capture, privacy, restoration, automatic paste, profiles, preferences and cleanup passed; snapshots: {}",
        directory.display()
    );
}
