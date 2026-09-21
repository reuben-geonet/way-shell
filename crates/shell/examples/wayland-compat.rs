//! Real compositor coverage for the separate Rust Wayland connection and GDK.
use gio::prelude::*;
use gtk::prelude::*;
use std::time::{Duration, Instant};
use way_shell::services::wayland::{ToplevelAction, WaylandService};
fn until(context: &glib::MainContext, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "Wayland component smoke test timed out"
        );
        context.block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    gtk::init()?;
    let context = glib::MainContext::default();
    context.with_thread_default(|| -> Result<(), Box<dyn std::error::Error>> {
        let source =
            gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)?;
        let schema = source
            .lookup("org.ldelossa.way-shell.window-manager", false)
            .unwrap();
        for cycle in 0..3 {
            let settings =
                gio::Settings::new_full(&schema, Some(&gio::memory_settings_backend_new()), None);
            let service = WaylandService::with_settings(settings.clone())?;
            until(&context, || service.is_ready() || service.error().is_some());
            assert!(service.error().is_none(), "{:?}", service.error());
            assert!(!service.outputs().is_empty());
            assert!(!service.seats().is_empty());
            assert!(
                service.has_foreign_toplevel(),
                "Compositor must expose existing foreign-toplevel functionality"
            );
            let title = format!("Way Shell protocol smoke 日本語 {cycle}");
            let window = gtk::Window::builder()
                .title(&title)
                .default_width(240)
                .default_height(120)
                .build();
            window.present();
            until(&context, || {
                service
                    .toplevels()
                    .iter()
                    .any(|top| top.title.as_deref() == Some(&title) && !top.outputs.is_empty())
            });
            let top = service
                .toplevels()
                .into_iter()
                .find(|top| top.title.as_deref() == Some(&title))
                .unwrap();
            assert!(!top.outputs.is_empty());
            service.action(top.id, ToplevelAction::Activate)?;
            until(&context, || {
                service
                    .toplevels()
                    .iter()
                    .any(|value| value.id == top.id && value.active)
            });
            let changed = format!("{title} updated");
            window.set_title(Some(&changed));
            until(&context, || {
                service
                    .toplevels()
                    .iter()
                    .any(|value| value.id == top.id && value.title.as_deref() == Some(&changed))
            });
            settings.set_string("ignored-toplevels-titles", &changed)?;
            assert!(!service.toplevels().iter().any(|value| value.id == top.id));
            settings.set_string("ignored-toplevels-titles", "")?;
            assert!(service.toplevels().iter().any(|value| value.id == top.id));
            if service.has_shortcut_inhibition() {
                service.inhibit_shortcuts(&window)?;
                assert!(service.inhibit_shortcuts(&window).is_err());
                assert!(service.restore_shortcuts());
                assert!(!service.restore_shortcuts());
            }
            if service.has_gamma() {
                service.set_temperature(4500)?;
                context.block_on(glib::timeout_future(Duration::from_millis(100)));
                service.disable_gamma();
                assert!(!service.gamma_enabled());
            }
            if cycle == 0 && std::env::var_os("NIRI_SOCKET").is_none() {
                let before = service.outputs().len();
                assert!(
                    std::process::Command::new("swaymsg")
                        .arg("create_output")
                        .status()?
                        .success()
                );
                until(&context, || service.outputs().len() > before);
                let output = service
                    .outputs()
                    .into_iter()
                    .find(|output| output.name.as_deref() == Some("HEADLESS-2"))
                    .unwrap();
                assert!(
                    std::process::Command::new("swaymsg")
                        .args(["output", "HEADLESS-2", "disable"])
                        .status()?
                        .success()
                );
                until(&context, || {
                    !service.outputs().iter().any(|value| value.id == output.id)
                });
            }
            service.action(top.id, ToplevelAction::Maximize)?;
            service.action(top.id, ToplevelAction::Close)?;
            until(&context, || {
                !service.toplevels().iter().any(|value| value.id == top.id)
            });
            assert!(service.action(top.id, ToplevelAction::Activate).is_err());
            window.destroy();
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
            eprintln!("Wayland protocol ownership cycle {cycle} passed");
        }
        Ok(())
    })??;
    Ok(())
}
