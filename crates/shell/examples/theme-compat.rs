//! Run on an isolated Wayland display to exercise CSS and layer-shell together.
use adw::prelude::*;
use std::{cell::RefCell, rc::Rc, time::Duration};
use way_shell::resources;
use way_shell::services::theme::{Theme, ThemeService};
use way_shell::ui::window::{LayerWindow, WindowRole};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    adw::init()?;
    resources::get();
    for selected in [Theme::Light, Theme::Dark] {
        let provider = gtk::CssProvider::new();
        let errors = Rc::new(RefCell::new(Vec::new()));
        let observed = errors.clone();
        provider.connect_parsing_error(move |_, section, error| {
            observed.borrow_mut().push(format!(
                "{}:{}: {error}",
                section.start_location().lines() + 1,
                section.start_location().line_chars() + 1
            ));
        });
        provider.load_from_resource(selected.resource_path());
        assert!(
            errors.borrow().is_empty(),
            "Bundled {selected:?} CSS must parse without errors: {:?}",
            errors.borrow()
        );
    }
    if !gtk4_layer_shell::is_supported() {
        return Err("The compositor does not support layer-shell".into());
    }
    let display = gtk::gdk::Display::default().ok_or("GTK did not open a display")?;
    let source =
        gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)?;
    let schema = source
        .lookup("org.ldelossa.way-shell.system", false)
        .ok_or("Missing test schema")?;
    let settings =
        gio::Settings::new_full(&schema, Some(&gio::memory_settings_backend_new()), None);
    let context = glib::MainContext::default();
    for _ in 0..3 {
        // This path deliberately cannot override the embedded themes or run hooks.
        let theme = ThemeService::with_settings(
            settings.clone(),
            std::path::PathBuf::from("/nonexistent/way-shell-theme-test"),
        );
        theme.attach_display(&display);
        let window = LayerWindow::new(WindowRole::Panel, None)?;
        window.window().set_default_size(320, 40);
        window.window().set_content(Some(&gtk::Label::new(Some(
            "Way Shell theme compatibility",
        ))));
        window.present();
        for selected in [Theme::Light, Theme::Dark] {
            theme.set_theme(selected)?;
            context.block_on(glib::timeout_future(Duration::from_millis(50)));
        }
        assert!(window.window().is_mapped());
        let weak = theme.downgrade();
        window.close();
        drop(window);
        drop(theme);
        assert!(weak.upgrade().is_none());
    }
    println!("three layer-window/theme/change/cleanup cycles passed");
    Ok(())
}
