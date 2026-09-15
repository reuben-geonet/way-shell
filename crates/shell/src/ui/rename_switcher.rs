//! Rename the current workspace without borrowing compositor state.

use super::switcher::{Switcher, SwitcherAction};
use crate::services::wm::WindowManager;
use gtk::prelude::*;
use std::{cell::OnceCell, rc::Rc};
use way_shell_core::wm::Action;

pub struct RenameSwitcher {
    manager: WindowManager,
    view: OnceCell<Rc<Switcher>>,
}

impl RenameSwitcher {
    pub fn new(manager: WindowManager) -> Result<Rc<Self>, String> {
        let this = Rc::new(Self {
            manager,
            view: OnceCell::new(),
        });
        let weak = Rc::downgrade(&this);
        let view = Switcher::new(false, false, move |action| {
            if let Some(this) = weak.upgrade() {
                this.action(action);
            }
        })?;
        this.view.get_or_init(|| view);
        Ok(this)
    }

    pub fn switcher(&self) -> &Rc<Switcher> {
        self.view.get().expect("rename switcher initialized")
    }
    pub fn is_visible(&self) -> bool {
        self.switcher().is_visible()
    }
    pub fn show(&self) {
        self.switcher().search().set_tooltip_text(None);
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

    fn action(&self, action: SwitcherAction) {
        if let SwitcherAction::Activate { text, .. } = action
            && let Some(action) = rename_action(&text)
        {
            match self.manager.perform(&action) {
                Ok(()) => self.hide(),
                Err(error) => {
                    self.switcher().search().set_tooltip_text(Some(&error));
                    glib::g_message!("way-shell", "Rename command failed: {error}");
                }
            }
        }
    }
}

impl Drop for RenameSwitcher {
    fn drop(&mut self) {
        if let Some(view) = self.view.get() {
            view.close();
        }
    }
}

fn rename_action(text: &str) -> Option<Action> {
    (!text.is_empty()).then(|| Action::RenameWorkspace(text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_rename_is_ignored_and_nonempty_text_is_not_trimmed() {
        assert_eq!(rename_action(""), None);
        for name in [" ", " 日本語; \\\"name\\\" ", "$mod \\ exact"] {
            assert_eq!(
                rename_action(name),
                Some(Action::RenameWorkspace(name.into()))
            );
        }
    }
}
