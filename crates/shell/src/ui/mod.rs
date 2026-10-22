//! GTK components share the application's GLib main loop.
pub mod activities;
pub mod app_switcher;
pub mod dialog;
pub mod message_tray;
pub mod osd;
pub mod output_switcher;
pub mod panel;
pub mod panel_popup;
mod progress;
pub mod quick_settings;
pub mod rename_switcher;
pub mod screenshot;
pub mod shortcuts;
pub mod switcher;
pub mod tray_support;
pub mod window;
pub mod workspace_switcher;

#[cfg(test)]
pub(crate) fn init_test_display() {
    adw::init().unwrap();
    crate::resources::get();
    let display = gtk::gdk::Display::default().unwrap();
    let provider = gtk::CssProvider::new();
    provider.load_from_path(
        std::env::var_os("WAY_SHELL_SCREENSHOT_TEST_CSS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../data/theme/way-shell-dark.css")
            }),
    );
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
