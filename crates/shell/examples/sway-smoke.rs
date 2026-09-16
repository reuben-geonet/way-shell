//! Exercises workspace actions and output hotplug on a private headless Sway.
use gio::prelude::*;
use std::{path::PathBuf, time::Duration};
use way_shell::services::wm::WindowManager;
use way_shell_core::wm::{Action, WorkspaceTarget};

fn wait(
    context: &glib::MainContext,
    predicate: impl Fn() -> bool,
) -> Result<(), glib::FutureWithTimeoutError> {
    context.block_on(glib::future_with_timeout(Duration::from_secs(5), async {
        while !predicate() {
            glib::timeout_future(Duration::from_millis(10)).await;
        }
    }))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let context = glib::MainContext::default();
    context.with_thread_default(|| -> Result<(), Box<dyn std::error::Error>> {
        let source =
            gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)?;
        let schema = source
            .lookup("org.ldelossa.way-shell.window-manager", false)
            .ok_or("Missing test schema")?;
        let settings =
            gio::Settings::new_full(&schema, Some(&gio::memory_settings_backend_new()), None);
        let socket = PathBuf::from(std::env::var_os("SWAYSOCK").ok_or("SWAYSOCK is missing")?);
        let service = WindowManager::sway_with_settings(
            settings,
            socket,
            PathBuf::from("/nonexistent/way-shell-test"),
        )?;
        wait(&context, || {
            !service.workspaces().is_empty() && !service.outputs().is_empty()
        })?;
        eprintln!("initial Sway state: {:?}", service.workspaces());
        for name in [
            "Rust; 日本語 \"one\"",
            "both ' and \" quotes, [brackets]",
            r"back\slash\",
            r#"back\"quote and \\"two"#,
            r"$mod and \$mod and \\$mod and $$mod",
        ] {
            service.perform(&Action::RenameWorkspace(name.into()))?;
            if let Err(error) = wait(&context, || {
                service
                    .workspaces()
                    .iter()
                    .any(|workspace| workspace.name == name)
            }) {
                return Err(format!(
                    "rename {name:?} failed: {error}; workspaces={:?}",
                    service.workspaces()
                )
                .into());
            }
        }
        eprintln!("Sway literal workspace names passed");
        service.perform(&Action::FocusWorkspace(WorkspaceTarget {
            id: None,
            number: 42,
            name: "42".into(),
        }))?;
        wait(&context, || {
            service
                .workspaces()
                .iter()
                .any(|workspace| workspace.num == 42 && workspace.focused)
        })?;
        if !std::process::Command::new("swaymsg")
            .arg("create_output")
            .status()?
            .success()
        {
            return Err("Could not create a headless output".into());
        }
        wait(&context, || service.outputs().len() == 2)?;
        let workspace = service
            .workspaces()
            .into_iter()
            .find(|workspace| workspace.num == 42)
            .ok_or("workspace 42 disappeared")?;
        service.perform(&Action::FocusWorkspace((&workspace).into()))?;
        let output = service
            .outputs()
            .into_iter()
            .find(|output| Some(&output.name) != workspace.output.as_ref())
            .ok_or("No second output")?;
        service.perform(&Action::MoveWorkspaceToOutput(output.name.clone()))?;
        wait(&context, || {
            service.workspaces().iter().any(|entry| {
                entry.id == workspace.id && entry.output.as_ref() == Some(&output.name)
            })
        })?;
        let weak = service.downgrade();
        drop(service);
        assert!(weak.upgrade().is_none());
        println!(
            "Sway framing, rename, focus, output hotplug, workspace movement and cleanup passed"
        );
        Ok(())
    })?
}
