//! Move the current workspace to an output selected by its canonical name.

use super::switcher::{Switcher, SwitcherAction, SwitcherEntry};
use crate::services::wm::WindowManager;
use gtk::prelude::*;
use std::{
    cell::{Cell, OnceCell},
    rc::Rc,
};
use way_shell_core::wm::{Action, Output};

pub struct OutputSwitcher {
    manager: WindowManager,
    view: OnceCell<Rc<Switcher>>,
    handler: Cell<Option<glib::SignalHandlerId>>,
}

impl OutputSwitcher {
    pub fn new(manager: WindowManager) -> Result<Rc<Self>, String> {
        let this = Rc::new(Self {
            manager,
            view: OnceCell::new(),
            handler: Cell::new(None),
        });
        let weak = Rc::downgrade(&this);
        let view = Switcher::new(true, false, move |action| {
            if let Some(this) = weak.upgrade() {
                this.action(action);
            }
        })?;
        this.view.get_or_init(|| view);
        let weak = Rc::downgrade(&this);
        this.handler.set(Some(this.manager.connect_local(
            "outputs-changed",
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
        self.view.get().expect("output switcher initialized")
    }
    pub fn is_visible(&self) -> bool {
        self.switcher().is_visible()
    }
    pub fn show(&self) {
        self.switcher().search().set_tooltip_text(None);
        self.refresh();
        self.switcher().show();
    }
    pub fn hide(&self) {
        self.switcher().hide();
    }
    pub fn toggle(&self) {
        if self.is_visible() {
            self.hide();
        } else {
            self.show();
        }
    }

    fn refresh(&self) {
        self.switcher().set_entries(
            self.manager
                .outputs()
                .into_iter()
                .map(|output| SwitcherEntry {
                    id: output.name.clone(),
                    label: output.name,
                })
                .collect(),
        );
    }

    fn action(&self, action: SwitcherAction) {
        if let SwitcherAction::Activate { selected, text, .. } = action {
            let result = output_action(&self.manager.outputs(), selected.as_deref(), &text)
                .and_then(|action| self.manager.perform(&action));
            match result {
                Ok(()) => self.hide(),
                Err(error) => {
                    self.switcher().search().set_tooltip_text(Some(&error));
                    glib::g_message!("way-shell", "Output command failed: {error}");
                }
            }
        }
    }
}

impl Drop for OutputSwitcher {
    fn drop(&mut self) {
        if let Some(handler) = self.handler.take() {
            self.manager.disconnect(handler);
        }
        if let Some(view) = self.view.get() {
            view.close();
        }
    }
}

fn output_action(outputs: &[Output], selected: Option<&str>, text: &str) -> Result<Action, String> {
    let name = if let Some(selected) = selected {
        outputs
            .iter()
            .find(|output| output.name == selected)
            .ok_or("The selected output is no longer available")?
            .name
            .clone()
    } else {
        text.to_owned()
    };
    Ok(Action::MoveWorkspaceToOutput(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_output_uses_identity_and_missing_selection_is_rejected() {
        let mut outputs = vec![
            Output {
                name: "DP-2".into(),
                ..Output::default()
            },
            Output {
                name: "eDP-1".into(),
                ..Output::default()
            },
        ];
        assert_eq!(
            output_action(&outputs, Some("DP-2"), "eDP"),
            Ok(Action::MoveWorkspaceToOutput("DP-2".into()))
        );
        outputs.reverse();
        assert_eq!(
            output_action(&outputs, Some("DP-2"), "eDP"),
            Ok(Action::MoveWorkspaceToOutput("DP-2".into()))
        );
        assert!(output_action(&outputs, Some("gone"), "DP-2").is_err());
        assert!(output_action(&[], Some("DP-2"), "DP-2").is_err());
    }

    #[test]
    fn unmatched_output_keeps_exact_user_supplied_name() {
        let name = "  Some Display; 日本語  ";
        assert_eq!(
            output_action(&[], None, name),
            Ok(Action::MoveWorkspaceToOutput(name.into()))
        );
    }
}
