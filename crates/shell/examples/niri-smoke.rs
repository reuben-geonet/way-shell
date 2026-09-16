//! Real Niri workspace actions and a regular GTK toplevel in the private fixture.
use gtk::prelude::*;
use std::{path::PathBuf, time::Duration};
use way_shell::services::wm::WindowManager;
use way_shell_core::wm::Action;

fn wait(
    context: &glib::MainContext,
    predicate: impl Fn() -> bool,
) -> Result<(), glib::FutureWithTimeoutError> {
    context.block_on(glib::future_with_timeout(Duration::from_secs(8), async {
        while !predicate() {
            glib::timeout_future(Duration::from_millis(10)).await;
        }
    }))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    gtk::init()?;
    let context = glib::MainContext::default();
    context.with_thread_default(|| -> Result<(), Box<dyn std::error::Error>> {
        let source =
            gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)?;
        let schema = source
            .lookup("org.ldelossa.way-shell.window-manager", false)
            .ok_or("Missing test schema")?;
        let settings =
            gio::Settings::new_full(&schema, Some(&gio::memory_settings_backend_new()), None);
        let service = WindowManager::niri_with_settings(
            settings,
            PathBuf::from(std::env::var_os("NIRI_SOCKET").ok_or("Missing NIRI_SOCKET")?),
            PathBuf::from("/nonexistent/way-shell-test"),
        )?;
        wait(&context, || {
            service.is_connected()
                && service.workspaces().len() >= 2
                && !service.outputs().is_empty()
        })?;
        let initial = service
            .workspaces()
            .into_iter()
            .find(|workspace| workspace.focused)
            .ok_or("No focused workspace")?;
        let other = service
            .workspaces()
            .into_iter()
            .find(|workspace| workspace.name == "second")
            .ok_or("Missing second fixture workspace")?;
        for name in ["2: 日本語", "quotes ' and \" and \\ and ;", "$variable"] {
            service.perform(&Action::RenameWorkspace(name.into()))?;
            wait(&context, || {
                service
                    .workspaces()
                    .iter()
                    .any(|workspace| workspace.id == initial.id && workspace.name == name)
            })?;
        }
        let window = gtk::Window::builder()
            .title("Way Shell Niri smoke test")
            .default_width(240)
            .default_height(160)
            .build();
        window.present();
        wait(&context, || {
            window.is_mapped()
                && service
                    .workspaces()
                    .iter()
                    .any(|workspace| workspace.id == initial.id && !workspace.empty)
        })?;
        service.perform(&Action::MoveWindowToWorkspace((&other).into()))?;
        wait(&context, || {
            service
                .workspaces()
                .iter()
                .any(|workspace| workspace.id == other.id && !workspace.empty)
                && service
                    .workspaces()
                    .iter()
                    .any(|workspace| workspace.id == initial.id && workspace.empty)
        })?;
        assert!(
            service
                .workspaces()
                .iter()
                .any(|workspace| workspace.id == initial.id && workspace.focused)
        );
        service.perform(&Action::FocusWorkspace((&other).into()))?;
        wait(&context, || {
            service
                .workspaces()
                .iter()
                .any(|workspace| workspace.id == other.id && workspace.focused)
        })?;
        service.perform(&Action::MoveWorkspaceToOutput(
            other.output.clone().ok_or("Workspace has no output")?,
        ))?;
        window.close();
        wait(&context, || {
            service
                .workspaces()
                .iter()
                .any(|workspace| workspace.id == other.id && workspace.empty)
        })?;
        let weak = service.downgrade();
        drop(service);
        assert!(weak.upgrade().is_none());
        println!("Niri literal names, stable IDs, focus, GTK window movement and cleanup passed");
        Ok(())
    })?
}
