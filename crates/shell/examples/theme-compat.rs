//! Run on an isolated Wayland display to exercise CSS and layer-shell together.
use gio::prelude::*;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use std::time::Duration;
use way_shell::services::theme::{Theme, ThemeService};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    gtk::init()?;
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
        let window = gtk::Window::new();
        window.init_layer_shell();
        window.set_layer(Layer::Top);
        window.set_namespace(Some("way-shell-theme-test"));
        window.set_anchor(Edge::Top, true);
        window.set_default_size(320, 40);
        window.set_child(Some(&gtk::Label::new(Some(
            "Way Shell theme compatibility",
        ))));
        window.present();
        for selected in [Theme::Light, Theme::Dark] {
            theme.set_theme(selected)?;
            context.block_on(glib::timeout_future(Duration::from_millis(50)));
        }
        assert!(window.is_mapped());
        let weak = theme.downgrade();
        window.close();
        drop(window);
        drop(theme);
        assert!(weak.upgrade().is_none());
    }
    println!("three layer-window/theme/change/cleanup cycles passed");
    Ok(())
}
