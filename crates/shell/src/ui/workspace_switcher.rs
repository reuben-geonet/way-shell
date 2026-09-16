//! Workspace focus and application movement share one owned selector.

use super::switcher::{Switcher, SwitcherAction, SwitcherEntry};
use crate::services::wm::WindowManager;
use gtk::prelude::*;
use std::{
    cell::{Cell, OnceCell},
    rc::Rc,
};
use way_shell_core::wm::{Action, Workspace, WorkspaceTarget};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceMode {
    #[default]
    FocusWorkspace,
    MoveWindow,
}

pub struct WorkspaceSwitcher {
    manager: WindowManager,
    view: OnceCell<Rc<Switcher>>,
    handler: Cell<Option<glib::SignalHandlerId>>,
    mode: Cell<WorkspaceMode>,
}

impl WorkspaceSwitcher {
    pub fn new(manager: WindowManager) -> Result<Rc<Self>, String> {
        let this = Rc::new(Self {
            manager,
            view: OnceCell::new(),
            handler: Cell::new(None),
            mode: Cell::new(WorkspaceMode::FocusWorkspace),
        });
        let weak = Rc::downgrade(&this);
        let view = Switcher::new(true, true, move |action| {
            if let Some(this) = weak.upgrade() {
                this.action(action);
            }
        })?;
        this.view.get_or_init(|| view);
        let weak = Rc::downgrade(&this);
        this.handler.set(Some(this.manager.connect_local(
            "workspaces-changed",
            false,
            move |_| {
                if let Some(this) = weak.upgrade() {
                    this.refresh();
                }
                None
            },
        )));
        this.refresh();
        Ok(this)
    }

    pub fn switcher(&self) -> &Rc<Switcher> {
        self.view.get().expect("workspace switcher initialized")
    }

    pub fn mode(&self) -> WorkspaceMode {
        self.mode.get()
    }

    pub fn is_visible(&self) -> bool {
        self.switcher().is_visible()
    }

    pub fn show(&self, mode: WorkspaceMode) {
        self.mode.set(mode);
        self.switcher().search().set_tooltip_text(None);
        self.refresh();
        self.switcher().show();
    }

    pub fn toggle(&self, mode: WorkspaceMode) {
        if self.is_visible() {
            self.hide();
        } else {
            self.show(mode);
        }
    }

    pub fn hide(&self) {
        self.mode.set(WorkspaceMode::FocusWorkspace);
        self.switcher().hide();
    }

    fn refresh(&self) {
        // The service owns configured sorting and compositor identity.
        let entries = self
            .manager
            .workspaces()
            .into_iter()
            .map(|workspace| SwitcherEntry {
                id: workspace.id.to_string(),
                label: workspace.name,
            })
            .collect();
        self.switcher().set_entries(entries);
    }

    fn action(&self, action: SwitcherAction) {
        match action {
            SwitcherAction::Hidden => self.mode.set(WorkspaceMode::FocusWorkspace),
            SwitcherAction::Activate {
                selected,
                text,
                raw,
            } => {
                let result = workspace_action(
                    &self.manager.workspaces(),
                    self.mode.get(),
                    selected.as_deref(),
                    &text,
                    raw,
                )
                .and_then(|action| self.manager.perform(&action));
                match result {
                    Ok(()) => self.hide(),
                    Err(error) => {
                        self.switcher().search().set_tooltip_text(Some(&error));
                        glib::g_message!("way-shell", "Workspace command failed: {error}");
                    }
                }
            }
        }
    }
}

impl Drop for WorkspaceSwitcher {
    fn drop(&mut self) {
        if let Some(handler) = self.handler.take() {
            self.manager.disconnect(handler);
        }
        if let Some(view) = self.view.get() {
            view.close();
        }
    }
}

fn workspace_action(
    workspaces: &[Workspace],
    mode: WorkspaceMode,
    selected: Option<&str>,
    text: &str,
    raw: bool,
) -> Result<Action, String> {
    let target = if let Some(selected) = selected.filter(|_| !raw) {
        let id = selected
            .parse::<u64>()
            .map_err(|_| "Invalid workspace identifier")?;
        let workspace = workspaces
            .iter()
            .find(|workspace| workspace.id == id)
            .ok_or("The selected workspace is no longer available")?;
        WorkspaceTarget::from(workspace)
    } else {
        WorkspaceTarget {
            id: None,
            number: -1,
            name: text.to_owned(),
        }
    };
    Ok(match mode {
        WorkspaceMode::FocusWorkspace => Action::FocusWorkspace(target),
        WorkspaceMode::MoveWindow => Action::MoveWindowToWorkspace(target),
    })
}

/// Internal controls documented by the shortcut sheet.
pub const NAVIGATION: &[(&str, &str)] = &[("Ctrl+Enter", "Use the typed workspace name")];

#[cfg(test)]
mod tests {
    use super::*;

    fn workspaces() -> Vec<Workspace> {
        vec![
            Workspace {
                id: u64::MAX,
                num: 3,
                name: "Work".into(),
                ..Workspace::default()
            },
            Workspace {
                id: 4294967313,
                num: 7,
                name: "Work".into(),
                ..Workspace::default()
            },
        ]
    }

    #[test]
    fn selection_retains_full_identifiers_despite_duplicate_names_and_reordering() {
        let mut values = workspaces();
        let id = 4294967313_u64.to_string();
        let expected = WorkspaceTarget::from(&values[1]);
        assert_eq!(
            workspace_action(
                &values,
                WorkspaceMode::FocusWorkspace,
                Some(&id),
                "ignored",
                false
            ),
            Ok(Action::FocusWorkspace(expected.clone()))
        );
        values.reverse();
        assert_eq!(
            workspace_action(
                &values,
                WorkspaceMode::MoveWindow,
                Some(&id),
                "ignored",
                false
            ),
            Ok(Action::MoveWindowToWorkspace(expected))
        );
        let id = u64::MAX.to_string();
        assert_eq!(
            workspace_action(
                &values,
                WorkspaceMode::FocusWorkspace,
                Some(&id),
                "ignored",
                false
            ),
            Ok(Action::FocusWorkspace(WorkspaceTarget::from(&values[1])))
        );
    }

    #[test]
    fn raw_return_obeys_mode_and_preserves_exact_text() {
        let values = workspaces();
        let name = "  Work; 日本語 \\\"  ";
        let target = WorkspaceTarget {
            id: None,
            number: -1,
            name: name.into(),
        };
        let selected = values[0].id.to_string();
        assert_eq!(
            workspace_action(
                &values,
                WorkspaceMode::FocusWorkspace,
                Some(&selected),
                name,
                true
            ),
            Ok(Action::FocusWorkspace(target.clone()))
        );
        assert_eq!(
            workspace_action(
                &values,
                WorkspaceMode::MoveWindow,
                Some(&selected),
                name,
                true
            ),
            Ok(Action::MoveWindowToWorkspace(target.clone()))
        );
        assert_eq!(
            workspace_action(&values, WorkspaceMode::MoveWindow, None, name, false),
            Ok(Action::MoveWindowToWorkspace(target))
        );
    }

    #[test]
    fn removed_or_invalid_selection_never_targets_another_workspace() {
        let values = workspaces();
        for id in ["missing", "0", "18446744073709551616"] {
            assert!(
                workspace_action(&values, WorkspaceMode::MoveWindow, Some(id), "Work", false)
                    .is_err()
            );
        }
        let id = values[0].id.to_string();
        assert!(
            workspace_action(&[], WorkspaceMode::FocusWorkspace, Some(&id), "Work", false).is_err()
        );
    }

    #[test]
    fn selected_rename_uses_updated_snapshot_with_unchanged_identity() {
        let mut values = workspaces();
        let selected = values[0].id.to_string();
        values[0].name = "Renamed 日本語".into();
        assert_eq!(
            workspace_action(
                &values,
                WorkspaceMode::MoveWindow,
                Some(&selected),
                "Work",
                false
            ),
            Ok(Action::MoveWindowToWorkspace(WorkspaceTarget::from(
                &values[0]
            )))
        );
    }
}
