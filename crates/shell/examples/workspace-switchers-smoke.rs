//! Exercise workspace, application movement, output and rename selectors on Sway/Niri.
use gtk::prelude::*;
use std::{path::PathBuf, rc::Rc, time::Duration};
use way_shell::{
    services::wm::WindowManager,
    ui::{
        output_switcher::OutputSwitcher,
        rename_switcher::RenameSwitcher,
        workspace_switcher::{WorkspaceMode, WorkspaceSwitcher},
    },
};
use way_shell_core::wm::Action;

const TITLE: &str = "Way Shell workspace switcher probe";
const TARGET_TITLE: &str = "Way Shell destination workspace keeper";

fn raw_return(window: &adw::Window) {
    let controllers = window.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .find(|keys| keys.name().as_deref() == Some("way-shell-switcher-keys"))
        .unwrap();
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Return,
            &0_u32,
            &gtk::gdk::ModifierType::CONTROL_MASK,
        ]
    ));
}

fn wait(context: &glib::MainContext, predicate: impl Fn() -> bool) {
    context
        .block_on(glib::future_with_timeout(Duration::from_secs(8), async {
            while !predicate() {
                glib::timeout_future(Duration::from_millis(10)).await;
            }
        }))
        .expect("Workspace switcher fixture timed out");
}

fn settings() -> gio::Settings {
    let source =
        gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)
            .unwrap();
    gio::Settings::new_full(
        &source
            .lookup("org.ldelossa.way-shell.window-manager", false)
            .unwrap(),
        Some(&gio::memory_settings_backend_new()),
        None,
    )
}

fn sway_window_workspace(
    node: &serde_json::Value,
    title: &str,
    workspace: Option<&str>,
) -> Option<String> {
    let workspace = if node["type"].as_str() == Some("workspace") {
        node["name"].as_str()
    } else {
        workspace
    };
    if node["name"].as_str() == Some(title) {
        return workspace.map(str::to_owned);
    }
    for list in ["nodes", "floating_nodes"] {
        if let Some(children) = node[list].as_array() {
            for child in children {
                if let Some(workspace) = sway_window_workspace(child, title, workspace) {
                    return Some(workspace);
                }
            }
        }
    }
    None
}

fn window_is_on_workspace(niri: bool, title: &str, target: &way_shell_core::wm::Workspace) -> bool {
    let output = if niri {
        std::process::Command::new("niri")
            .args(["msg", "--json", "windows"])
            .output()
            .unwrap()
    } else {
        std::process::Command::new("swaymsg")
            .args(["-t", "get_tree", "-r"])
            .output()
            .unwrap()
    };
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    if niri {
        value.as_array().unwrap().iter().any(|window| {
            window["title"].as_str() == Some(title)
                && window["workspace_id"].as_u64() == Some(target.id)
        })
    } else {
        sway_window_workspace(&value, title, None).as_deref() == Some(&target.name)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    adw::init()?;
    let context = glib::MainContext::default();
    let _guard = context.acquire()?;
    let niri = std::env::var_os("NIRI_SOCKET").is_some();
    let manager = if niri {
        WindowManager::niri_with_settings(
            settings(),
            PathBuf::from(std::env::var_os("NIRI_SOCKET").unwrap()),
            PathBuf::from("/nonexistent/way-shell-switcher-fixture"),
        )?
    } else {
        WindowManager::sway_with_settings(
            settings(),
            PathBuf::from(std::env::var_os("SWAYSOCK").unwrap()),
            PathBuf::from("/nonexistent/way-shell-switcher-fixture"),
        )?
    };
    wait(&context, || {
        manager.is_connected() && !manager.workspaces().is_empty() && !manager.outputs().is_empty()
    });
    let workspace = WorkspaceSwitcher::new(manager.clone())?;
    let output = OutputSwitcher::new(manager.clone())?;
    let rename = RenameSwitcher::new(manager.clone())?;
    let app = gtk::Window::builder()
        .title(TITLE)
        .default_width(240)
        .default_height(160)
        .build();
    app.set_child(Some(&gtk::Label::new(Some(
        "Move this window through the Rust selector",
    ))));
    app.present();
    let initial = manager
        .workspaces()
        .into_iter()
        .find(|workspace| workspace.focused)
        .unwrap();
    // GTK mapping can precede the compositor placing the window. Ensure this
    // workspace is occupied before switching away so Sway does not remove it.
    wait(&context, || {
        app.is_mapped() && window_is_on_workspace(niri, TITLE, &initial)
    });
    rename.show();
    wait(&context, || rename.switcher().window().is_mapped());
    rename.switcher().activate(false);
    assert!(
        rename.is_visible(),
        "empty rename must not close the selector"
    );
    let exact = "Work Café; 日本語";
    rename.switcher().search().set_text(exact);
    rename.switcher().activate(false);
    assert!(!rename.is_visible());
    wait(&context, || {
        manager
            .workspaces()
            .iter()
            .any(|workspace| workspace.id == initial.id && workspace.name == exact)
    });

    workspace.show(WorkspaceMode::FocusWorkspace);
    wait(&context, || workspace.switcher().window().is_mapped());
    workspace.switcher().search().set_text("second");
    raw_return(workspace.switcher().window());
    wait(&context, || {
        manager
            .workspaces()
            .iter()
            .any(|workspace| workspace.name == "second" && workspace.focused)
    });
    let target = manager
        .workspaces()
        .into_iter()
        .find(|workspace| workspace.name == "second")
        .unwrap();
    // Sway removes empty workspaces after focus moves away. Keep this target
    // alive while the source window is selected and moved through the UI.
    let target_window = gtk::Window::builder()
        .title(TARGET_TITLE)
        .default_width(200)
        .default_height(120)
        .build();
    target_window.present();
    wait(&context, || {
        target_window.is_mapped() && window_is_on_workspace(niri, TARGET_TITLE, &target)
    });
    let source = manager
        .workspaces()
        .into_iter()
        .find(|workspace| workspace.id == initial.id)
        .unwrap();
    manager.perform(&Action::FocusWorkspace((&source).into()))?;
    wait(&context, || {
        manager
            .workspaces()
            .iter()
            .any(|workspace| workspace.id == source.id && workspace.focused)
    });

    workspace.show(WorkspaceMode::FocusWorkspace);
    workspace.switcher().search().set_text("second");
    assert_eq!(
        workspace.switcher().selected_id(),
        Some(target.id.to_string())
    );
    let target_position = manager
        .workspaces()
        .iter()
        .position(|workspace| workspace.id == target.id)
        .unwrap();
    manager.perform(&Action::RenameWorkspace("zz source 日本語".into()))?;
    wait(&context, || {
        manager
            .workspaces()
            .iter()
            .any(|workspace| workspace.id == source.id && workspace.name == "zz source 日本語")
    });
    if !niri {
        assert_ne!(
            manager
                .workspaces()
                .iter()
                .position(|workspace| workspace.id == target.id),
            Some(target_position),
            "alphabetical renaming must exercise an actual row reorder"
        );
    }
    assert_eq!(
        workspace.switcher().selected_id(),
        Some(target.id.to_string()),
        "inventory reorder must retain selected identity"
    );
    workspace.hide();

    workspace.show(WorkspaceMode::MoveWindow);
    wait(&context, || workspace.switcher().window().is_mapped());
    workspace.switcher().search().set_text("second");
    raw_return(workspace.switcher().window());
    assert!(!workspace.is_visible());
    assert_eq!(workspace.mode(), WorkspaceMode::FocusWorkspace);
    wait(&context, || window_is_on_workspace(niri, TITLE, &target));
    // A raw move must leave the original workspace focused, not perform a focus command.
    assert!(
        manager
            .workspaces()
            .iter()
            .any(|workspace| workspace.id == source.id && workspace.focused)
    );
    workspace.show(WorkspaceMode::FocusWorkspace);
    workspace.switcher().search().set_text("second");
    workspace.switcher().activate(false);
    wait(&context, || {
        manager
            .workspaces()
            .iter()
            .any(|workspace| workspace.id == target.id && workspace.focused)
    });

    if !niri {
        assert!(
            std::process::Command::new("swaymsg")
                .arg("create_output")
                .status()?
                .success()
        );
        wait(&context, || manager.outputs().len() >= 2);
    }
    manager.perform(&Action::FocusWorkspace((&target).into()))?;
    wait(&context, || {
        manager
            .workspaces()
            .iter()
            .any(|workspace| workspace.id == target.id && workspace.focused)
    });
    output.show();
    wait(&context, || output.switcher().window().is_mapped());
    let current_output = manager
        .workspaces()
        .into_iter()
        .find(|workspace| workspace.id == target.id)
        .unwrap()
        .output;
    let outputs = manager.outputs();
    let destination = outputs
        .iter()
        .find(|output| Some(&output.name) != current_output.as_ref())
        .unwrap_or(&outputs[0])
        .name
        .clone();
    output.switcher().search().set_text(&destination);
    assert_eq!(
        output.switcher().selected_id().as_deref(),
        Some(destination.as_str())
    );
    output.switcher().activate(false);
    assert!(!output.is_visible());
    wait(&context, || {
        manager.workspaces().iter().any(|workspace| {
            workspace.id == target.id && workspace.output.as_ref() == Some(&destination)
        })
    });

    workspace.show(WorkspaceMode::MoveWindow);
    workspace.switcher().search().set_text("clear me");
    workspace.switcher().cancel();
    assert!(workspace.is_visible());
    assert!(workspace.switcher().search().text().is_empty());
    workspace.switcher().cancel();
    assert!(!workspace.is_visible());
    assert_eq!(workspace.mode(), WorkspaceMode::FocusWorkspace);

    // A disconnected backend keeps input and the surface available for retry.
    let disconnected: WindowManager = glib::Object::new();
    let failed_workspace = WorkspaceSwitcher::new(disconnected.clone())?;
    let failed_output = OutputSwitcher::new(disconnected.clone())?;
    let failed_rename = RenameSwitcher::new(disconnected)?;
    failed_workspace.show(WorkspaceMode::MoveWindow);
    failed_workspace
        .switcher()
        .search()
        .set_text("retry exact name");
    failed_workspace.switcher().activate(true);
    assert!(failed_workspace.is_visible());
    assert!(
        failed_workspace
            .switcher()
            .search()
            .tooltip_text()
            .is_some()
    );
    failed_workspace.hide();
    failed_output.show();
    failed_output.switcher().search().set_text("DP-1");
    failed_output.switcher().activate(false);
    assert!(failed_output.is_visible());
    failed_output.hide();
    failed_rename.show();
    failed_rename.switcher().search().set_text("still here");
    failed_rename.switcher().activate(false);
    assert!(failed_rename.is_visible());
    drop((failed_workspace, failed_output, failed_rename));

    workspace.show(WorkspaceMode::FocusWorkspace);
    let retained = workspace.switcher().clone();
    let weak = Rc::downgrade(&workspace);
    drop(workspace);
    assert!(weak.upgrade().is_none());
    assert!(!retained.is_visible());
    retained.show();
    retained.activate(true);
    assert!(!retained.is_visible());
    output.show();
    let retained_output = output.switcher().clone();
    drop(output);
    assert!(!retained_output.is_visible());
    rename.show();
    let retained_rename = rename.switcher().clone();
    drop(rename);
    assert!(!retained_rename.is_visible());
    app.close();
    target_window.close();
    let weak_manager = manager.downgrade();
    drop(manager);
    assert!(weak_manager.upgrade().is_none());
    println!("workspace/raw movement/rename/output/error recovery/owned lifetime checks passed");
    Ok(())
}
