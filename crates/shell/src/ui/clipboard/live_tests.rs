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

fn response_button(dialog: &adw::MessageDialog, text: &str) -> gtk::Button {
    let mut pending = vec![dialog.clone().upcast::<gtk::Widget>()];
    while let Some(widget) = pending.pop() {
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && button.label().as_deref() == Some(text)
        {
            assert!(button.is_sensitive() && button.is_mapped());
            return button.clone();
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            pending.push(widget);
        }
    }
    panic!("Missing dialog button: {text}");
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
    let monitors = display.monitors();
    let monitor = (0..monitors.n_items())
        .filter_map(|index| monitors.item(index).and_downcast::<gdk::Monitor>())
        .max_by_key(|monitor| monitor.geometry().width())
        .unwrap();
    let screenshot = |name: &str| {
        context.block_on(glib::timeout_future(Duration::from_millis(100)));
        assert!(
            std::process::Command::new("grim")
                .arg("-o")
                .arg(monitor.connector().unwrap())
                .arg(directory.join(name))
                .status()
                .unwrap()
                .success()
        );
    };
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
    dialog.show(Some(&monitor)).unwrap();
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
    screenshot("default.png");
    let row = dialog.list.first_child().unwrap();
    let card = row.first_child().unwrap();
    let mut controls = Vec::new();
    let mut child = card.first_child().unwrap().first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget.is::<gtk::Button>() {
            controls.push(widget);
        } else if widget.is::<gtk::MenuButton>() {
            controls.push(widget.first_child().unwrap());
        }
    }
    assert_eq!(controls.len(), 3);
    for button in &controls {
        assert_eq!(button.width(), button.height());
        button.set_state_flags(gtk::StateFlags::PRELIGHT, false);
    }
    screenshot("card-controls-hover.png");
    for button in &controls {
        button.unset_state_flags(gtk::StateFlags::PRELIGHT);
    }
    let card_bounds = card.compute_bounds(&dialog.panel).unwrap();
    let search_bounds = dialog.search_box.compute_bounds(&dialog.panel).unwrap();
    let above = card_bounds.y() - search_bounds.y() - search_bounds.height();
    let below = dialog.panel.height() as f32 - card_bounds.y() - card_bounds.height();
    assert!(
        (above - below).abs() <= 2.0,
        "Uneven card gaps: {above} above, {below} below"
    );
    assert!(!dialog.scroller.is_overlay_scrolling());
    let scrollbar = dialog.scroller.hscrollbar();
    wait(&context, || scrollbar.is_mapped());
    let bounds = scrollbar.compute_bounds(&dialog.panel).unwrap();
    assert!(bounds.x() >= 18.0);
    assert!(bounds.x() + bounds.width() <= dialog.panel.width() as f32 - 18.0);
    let thickness = scrollbar.height();
    assert_eq!(thickness, 8);
    scrollbar.set_state_flags(gtk::StateFlags::PRELIGHT, false);
    screenshot("scrollbar-hover.png");
    assert_eq!(scrollbar.height(), thickness);
    scrollbar.unset_state_flags(gtk::StateFlags::PRELIGHT);
    assert!(dialog.search_box.width() <= 360);
    let bounds = dialog.search_box.compute_bounds(&dialog.panel).unwrap();
    assert!((bounds.x() + bounds.width() / 2.0 - dialog.panel.width() as f32 / 2.0).abs() <= 1.0);
    for button in [
        dialog.controls.first_child().unwrap(),
        dialog.controls.last_child().unwrap(),
        dialog.pinned.clone().upcast(),
    ] {
        assert_eq!(
            button.width(),
            button.height(),
            "Icon controls must be circular"
        );
    }
    dialog.filter_menu.popup();
    screenshot("search-menu.png");
    dialog.filter_menu.popdown();
    // Selecting a type closes the menu without a nested dropdown retaining a grab.
    dialog.filter_menu.popup();
    let type_list = dialog
        .filter_menu
        .popover()
        .unwrap()
        .child()
        .unwrap()
        .last_child()
        .unwrap()
        .downcast::<gtk::ListView>()
        .unwrap();
    dialog.kind.set_selected(1);
    type_list.emit_by_name::<()>("activate", &[&1u32]);
    wait(&context, || {
        !dialog.filter_menu.popover().unwrap().is_visible()
    });
    dialog.kind.set_selected(0);
    for (index, kind) in Kind::ALL.iter().enumerate() {
        dialog.kind.set_selected(index as u32 + 1);
        assert!(dialog.model.n_items() > 0);
        screenshot(&format!("kind-{}.png", kind.name().to_lowercase()));
    }
    dialog.kind.set_selected(0);
    let menu = dialog.item_menu(first);
    menu.set_parent(&dialog.panel);
    menu.popup();
    screenshot("item-menu.png");
    menu.popdown();
    menu.unparent();
    provider.load_from_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/theme/way-shell-light.css"),
    );
    dialog
        .style_manager
        .set_color_scheme(adw::ColorScheme::ForceLight);
    screenshot("light.png");
    dialog.filter_menu.popup();
    screenshot("search-menu-light.png");
    dialog.filter_menu.popdown();
    provider.load_from_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/theme/way-shell-dark.css"),
    );
    dialog
        .style_manager
        .set_color_scheme(adw::ColorScheme::ForceDark);
    assert!(!dialog.empty.is_visible());
    dialog.pinned.set_active(true);
    wait(&context, || {
        dialog.empty.is_mapped() && dialog.empty.width() > 0
    });
    assert_eq!(dialog.model.n_items(), 0);
    assert!(dialog.status.text().is_empty());
    wait(&context, || !scrollbar.is_mapped());
    screenshot("empty-pinned.png");
    let bounds = dialog.empty.compute_bounds(&dialog.scroller).unwrap();
    assert!(bounds.y() >= 0.0 && bounds.y() + bounds.height() <= dialog.scroller.height() as f32);
    assert!(
        (bounds.x() + bounds.width() / 2.0 - dialog.scroller.width() as f32 / 2.0).abs() <= 1.0
    );
    assert!(
        (bounds.y() + bounds.height() / 2.0 - dialog.scroller.height() as f32 / 2.0).abs() <= 1.0
    );
    dialog.pinned.set_active(false);
    assert_eq!(dialog.model.n_items(), 9);
    assert!(!dialog.empty.is_visible());
    dialog.search.set_text("First captured");
    assert_eq!(dialog.model.n_items(), 1);
    wait(&context, || !scrollbar.is_mapped());
    screenshot("single-item.png");
    dialog.search.set_text("");
    wait(&context, || scrollbar.is_mapped());
    settings.set_boolean("show-scrollbar", false).unwrap();
    wait(&context, || !scrollbar.is_mapped());
    settings.set_boolean("show-scrollbar", true).unwrap();
    wait(&context, || scrollbar.is_mapped());
    dialog.search.set_text("First captured");
    let mut entry = service.entry(first).unwrap();
    entry.pinned = true;
    entry.tag = Some("blue".into());
    service.update(entry);
    wait(&context, || {
        service.entry(first).is_some_and(|entry| entry.pinned)
    });
    dialog.hide();
    preferences::apply_profile(&settings, true);
    dialog.show(Some(&monitor)).unwrap();
    wait(&context, || {
        dialog.list.orientation() == gtk::Orientation::Vertical
    });
    context.block_on(glib::timeout_future(Duration::from_millis(500)));
    dialog.search.set_text("First captured");
    let vertical_scrollbar = dialog.scroller.vscrollbar();
    wait(&context, || !vertical_scrollbar.is_mapped());
    screenshot("compact.png");
    dialog.search.set_text("");
    wait(&context, || vertical_scrollbar.is_mapped());
    assert_eq!(vertical_scrollbar.width(), 8);
    let bounds = vertical_scrollbar.compute_bounds(&dialog.panel).unwrap();
    assert!(bounds.y() >= 18.0);
    screenshot("compact-overflow.png");
    dialog.search.set_text("No matching clipboard item");
    wait(&context, || !vertical_scrollbar.is_mapped());
    screenshot("compact-empty.png");
    dialog.search.set_text("First captured");
    dialog.hide();
    preferences::apply_profile(&settings, false);
    dialog.open_preferences();
    let prefs = dialog.preferences.borrow().clone().unwrap();
    wait(&context, || prefs.is_mapped());
    assert!(!prefs.is_modal(), "Preferences must not block panel clicks");
    dialog.show(Some(&monitor)).unwrap();
    wait(&context, || dialog.window().is_active());
    assert!(dialog.window().is_sensitive());
    assert!(prefs.is_mapped());
    screenshot("history-with-preferences.png");
    dialog.hide();
    context.block_on(glib::timeout_future(Duration::from_millis(200)));
    prefs.close();
    wait(&context, || dialog.preferences.borrow().is_none());
    dialog.show(Some(&monitor)).unwrap();
    dialog.edit(first, false);
    let edit = gtk::Window::list_toplevels()
        .into_iter()
        .find(|window| window.has_css_class("clipboard-edit-dialog"))
        .unwrap()
        .downcast::<adw::MessageDialog>()
        .unwrap();
    wait(&context, || edit.is_mapped());
    screenshot("edit.png");
    response_button(&edit, "Cancel").emit_clicked();
    wait(&context, || !edit.is_visible());
    dialog.run_action(first, service.actions().find("qrcode").unwrap().clone());
    let qr = gtk::Window::list_toplevels()
        .into_iter()
        .find(|window| window.has_css_class("clipboard-qr-dialog"))
        .unwrap()
        .downcast::<adw::MessageDialog>()
        .unwrap();
    wait(&context, || qr.is_mapped());
    screenshot("qr.png");
    response_button(&qr, "Close").emit_clicked();
    wait(&context, || !qr.is_visible());
    let confirmation = dialog.confirm_clear();
    wait(&context, || confirmation.is_mapped());
    assert!(confirmation.is_layer_window());
    assert_eq!(confirmation.layer(), Layer::Overlay);
    screenshot("clear-confirmation.png");
    response_button(&confirmation, "Cancel").emit_clicked();
    wait(&context, || !confirmation.is_visible());
    assert_eq!(service.entries().len(), 9);
    let confirmation = dialog.confirm_clear();
    wait(&context, || confirmation.is_mapped());
    response_button(&confirmation, "Clear").emit_clicked();
    wait(&context, || {
        service.entries().len() == 1 && !confirmation.is_visible()
    });
    dialog.hide();
    sample.present();
    editor.grab_focus();
    wait(&context, || {
        wayland
            .focused_toplevel()
            .is_some_and(|top| top.title.as_deref() == Some("Clipboard paste target"))
    });
    dialog.show(Some(&monitor)).unwrap();
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
    dialog.show(Some(&monitor)).unwrap();
    assert!(dialog.retry.is_sensitive());
    assert!(!dialog.empty.is_visible());
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
