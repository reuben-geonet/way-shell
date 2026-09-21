//! Workspace buttons retain stable compositor identifiers through inventory changes.
use crate::services::wm::WindowManager;
use gio::prelude::*;
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use way_shell_core::wm::{Action, Workspace, WorkspaceTarget};

struct Row {
    button: gtk::Button,
    target: RefCell<WorkspaceTarget>,
}
struct Inner {
    root: gtk::Box,
    list: gtk::Box,
    monitor: gtk::gdk::Monitor,
    manager: WindowManager,
    rows: RefCell<Vec<Rc<Row>>>,
    handlers: RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>,
    updating: Cell<bool>,
    pending: Cell<bool>,
}
impl Inner {
    fn update(self: &Rc<Self>) {
        self.pending.set(true);
        if self.updating.replace(true) {
            return;
        }
        while self.pending.replace(false) {
            self.update_once();
        }
        self.updating.set(false);
    }
    fn update_once(self: &Rc<Self>) {
        let connector = self.monitor.connector();
        let workspaces = for_output(&self.manager.workspaces(), connector.as_deref());
        // Keep native callbacks independent of this mutable collection. Retained
        // buttons use weak row/controller references and become inert on removal.
        let mut rows = self.rows.take();
        while rows.len() > workspaces.len() {
            self.list.remove(&rows.pop().unwrap().button);
        }
        for (index, workspace) in workspaces.iter().enumerate() {
            if index == rows.len() {
                let row = Rc::new(Row {
                    button: gtk::Button::new(),
                    target: RefCell::new(WorkspaceTarget::from(workspace)),
                });
                row.button.add_css_class("panel-button");
                let weak = Rc::downgrade(self);
                let weak_row = Rc::downgrade(&row);
                row.button.connect_clicked(move |_| {
                    if let (Some(inner), Some(row)) = (weak.upgrade(), weak_row.upgrade()) {
                        let action = Action::FocusWorkspace(row.target.borrow().clone());
                        if let Err(error) = inner.manager.perform(&action) {
                            glib::g_message!("way-shell", "Could not focus workspace: {error}");
                        }
                    }
                });
                self.list.append(&row.button);
                rows.push(row);
            }
            let row = &rows[index];
            row.target.replace(WorkspaceTarget::from(workspace));
            row.button.set_label(&workspace.name);
            for (class, active) in [
                ("panel-button-toggled", workspace.focused),
                (
                    "panel-button-urgent",
                    workspace.urgent && !workspace.focused,
                ),
            ] {
                if active {
                    row.button.add_css_class(class);
                } else {
                    row.button.remove_css_class(class);
                }
            }
        }
        let focused = workspaces
            .iter()
            .position(|workspace| workspace.focused)
            .map(|index| rows[index].button.clone());
        self.rows.replace(rows);
        if let Some(button) = focused {
            button.grab_focus();
        }
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        for (source, handler) in self.handlers.get_mut().drain(..) {
            source.disconnect(handler);
        }
    }
}

#[derive(Clone)]
pub struct WorkspacesBar(Rc<Inner>);
impl WorkspacesBar {
    pub fn new(manager: WindowManager, monitor: &gtk::gdk::Monitor) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        // Preserve the actual legacy root ID used by custom stylesheets.
        root.set_widget_name("workspaces-bar-list");
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_max_content_width(800);
        scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        scroll.set_propagate_natural_width(true);
        let list = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        scroll.set_child(Some(&list));
        root.append(&scroll);
        let inner = Rc::new(Inner {
            root,
            list,
            monitor: monitor.clone(),
            manager,
            rows: RefCell::new(Vec::new()),
            handlers: RefCell::new(Vec::new()),
            updating: Cell::new(false),
            pending: Cell::new(false),
        });
        let weak = Rc::downgrade(&inner);
        let handler = inner
            .manager
            .connect_local("workspaces-changed", false, move |_| {
                if let Some(inner) = weak.upgrade() {
                    inner.update();
                }
                None
            });
        inner
            .handlers
            .borrow_mut()
            .push((inner.manager.clone().upcast(), handler));
        let weak = Rc::downgrade(&inner);
        let handler = monitor.connect_notify_local(Some("connector"), move |_, _| {
            if let Some(inner) = weak.upgrade() {
                inner.update();
            }
        });
        inner
            .handlers
            .borrow_mut()
            .push((monitor.clone().upcast(), handler));
        inner.update();
        Self(inner)
    }
    pub fn widget(&self) -> &gtk::Box {
        &self.0.root
    }
}

fn for_output(workspaces: &[Workspace], connector: Option<&str>) -> Vec<Workspace> {
    workspaces
        .iter()
        .filter(|workspace| workspace.output.as_deref() == connector)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn monitor_filter_preserves_order_names_and_full_width_workspace_ids() {
        let first = Workspace {
            id: u64::MAX - 1,
            name: "1: 'quoted' workspace".into(),
            output: Some("HEADLESS-1".into()),
            focused: true,
            ..Default::default()
        };
        let second = Workspace {
            id: 1,
            output: Some("HEADLESS-2".into()),
            ..first.clone()
        };
        let third = Workspace {
            id: 2,
            ..first.clone()
        };
        let selected = for_output(&[first.clone(), second, third.clone()], Some("HEADLESS-1"));
        assert_eq!(selected, [first.clone(), third]);
        assert_eq!(WorkspaceTarget::from(&selected[0]).id, Some(first.id));
        assert!(for_output(&selected, Some("removed")).is_empty());
    }
}
