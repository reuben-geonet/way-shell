//! Session actions retain inline confirmation and the separate confirmation dialog.
use super::{controls::Subscription, menu::Menu};
use crate::services::logind::{LoginState, LogindService, PowerAction};
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemAction {
    Suspend,
    Restart,
    PowerOff,
    Logout,
}
impl SystemAction {
    const ALL: [Self; 4] = [Self::Suspend, Self::Restart, Self::PowerOff, Self::Logout];
    pub fn title(self) -> &'static str {
        match self {
            Self::Suspend => "Suspend?",
            Self::Restart => "Restart?",
            Self::PowerOff => "Power Off?",
            Self::Logout => "Log Out?",
        }
    }
    pub fn body(self) -> &'static str {
        match self {
            Self::Suspend => "Are you sure you want to suspend?",
            Self::Restart => "Are you sure you want to restart?",
            Self::PowerOff => "Are you sure you want to power off?",
            Self::Logout => "Are you sure you want to log out?",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Suspend => "Suspend",
            Self::Restart => "Restart",
            Self::PowerOff => "Power Off",
            Self::Logout => "Log Out...",
        }
    }
}
pub struct Confirmation {
    pub action: SystemAction,
    /// The coordinator hides quick settings, presents the existing dialog, then responds once.
    pub respond: Box<dyn FnOnce(bool)>,
}

pub fn suspend_action(preference: &str, state: &LoginState) -> Option<PowerAction> {
    let preferred = match preference {
        "suspend" => Some(PowerAction::Suspend),
        "hibernate" => Some(PowerAction::Hibernate),
        "hybrid-sleep" => Some(PowerAction::HybridSleep),
        "suspend-then-hibernate" => Some(PowerAction::SuspendThenHibernate),
        _ => None,
    };
    preferred
        .into_iter()
        .chain([
            PowerAction::Suspend,
            PowerAction::HybridSleep,
            PowerAction::SuspendThenHibernate,
            PowerAction::Hibernate,
        ])
        .find(|action| state.capabilities[*action as usize])
}
struct Row {
    button: gtk::Button,
    confirm: gtk::Button,
    revealer: gtk::Revealer,
}
pub struct PowerMenu {
    menu: Menu,
    rows: Vec<Row>,
    logind: LogindService,
    settings: gio::Settings,
    subscriptions: RefCell<Vec<Subscription>>,
    request: Box<dyn Fn(Confirmation)>,
    generation: Cell<u64>,
    operation: RefCell<Option<gio::Cancellable>>,
    stopped: Cell<bool>,
    updating: Cell<bool>,
    pending_row: Cell<Option<Option<usize>>>,
    refreshing: Cell<bool>,
    refresh_pending: Cell<bool>,
}
impl PowerMenu {
    pub fn new(
        logind: LogindService,
        settings: gio::Settings,
        request: impl Fn(Confirmation) + 'static,
    ) -> Rc<Self> {
        let menu = Menu::new("Power Off", "system-shutdown-symbolic", false);
        let rows = SystemAction::ALL
            .into_iter()
            .map(|action| {
                let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
                let button = gtk::Button::with_label(action.label());
                button.set_hexpand(true);
                if let Some(label) = button.child().and_downcast::<gtk::Label>() {
                    label.set_xalign(0.0);
                }
                root.append(&button);
                let revealer = gtk::Revealer::builder()
                    .transition_type(gtk::RevealerTransitionType::SwingDown)
                    .transition_duration(250)
                    .build();
                let confirm = gtk::Button::with_label("Confirm");
                confirm.set_hexpand(true);
                confirm.add_css_class("confirm-button");
                revealer.set_child(Some(&confirm));
                root.append(&revealer);
                menu.options().append(&root);
                Row {
                    button,
                    confirm,
                    revealer,
                }
            })
            .collect();
        let this = Rc::new(Self {
            menu,
            rows,
            logind,
            settings,
            subscriptions: RefCell::new(Vec::new()),
            request: Box::new(request),
            generation: Cell::new(0),
            operation: RefCell::new(None),
            stopped: Cell::new(false),
            updating: Cell::new(false),
            pending_row: Cell::new(None),
            refreshing: Cell::new(false),
            refresh_pending: Cell::new(false),
        });
        for (index, row) in this.rows.iter().enumerate() {
            let weak = Rc::downgrade(&this);
            row.button.connect_clicked(move |button| {
                if let Some(this) = weak.upgrade()
                    && button.is_sensitive()
                    && !this.stopped.get()
                {
                    this.reveal((!this.rows[index].revealer.reveals_child()).then_some(index));
                }
            });
            let weak = Rc::downgrade(&this);
            row.confirm.connect_clicked(move |button| {
                if let Some(this) = weak.upgrade()
                    && button.is_sensitive()
                    && !this.stopped.get()
                    && this.rows[index].revealer.reveals_child()
                {
                    this.request_confirmation(SystemAction::ALL[index]);
                }
            });
        }
        let weak = Rc::downgrade(&this);
        let handler = this.logind.connect_local("changed", false, move |_| {
            if let Some(this) = weak.upgrade() {
                this.refresh();
            }
            None
        });
        this.subscriptions
            .borrow_mut()
            .push(Subscription::new(&this.logind, handler));
        this.refresh();
        this
    }
    pub fn widget(&self) -> &gtk::Box {
        self.menu.widget()
    }
    pub fn action_button(&self, action: SystemAction) -> &gtk::Button {
        &self.rows[action as usize].button
    }
    pub fn confirmation_button(&self, action: SystemAction) -> &gtk::Button {
        &self.rows[action as usize].confirm
    }
    pub fn hide_confirmations(&self) {
        self.reveal(None);
    }
    pub fn stop(&self) {
        if self.stopped.replace(true) {
            return;
        }
        self.subscriptions.borrow_mut().clear();
        self.generation.set(self.generation.get().wrapping_add(1));
        let operation = self.operation.borrow_mut().take();
        if let Some(operation) = operation {
            operation.cancel();
        }
        self.reveal(None);
        self.menu.widget().set_sensitive(false);
    }
    fn reveal(&self, index: Option<usize>) {
        self.pending_row.set(Some(index));
        if self.updating.replace(true) {
            return;
        }
        while let Some(index) = self.pending_row.take() {
            for (current, row) in self.rows.iter().enumerate() {
                row.revealer
                    .set_reveal_child(!self.stopped.get() && index == Some(current));
            }
        }
        self.updating.set(false);
    }
    fn refresh(&self) {
        if self.stopped.get() {
            return;
        }
        self.refresh_pending.set(true);
        if self.refreshing.replace(true) {
            return;
        }
        while self.refresh_pending.replace(false) && !self.stopped.get() {
            let state = self.logind.state();
            for (index, enabled) in [
                suspend_action(&self.settings.string("suspend-method"), &state).is_some(),
                state.capabilities[PowerAction::Reboot as usize],
                state.capabilities[PowerAction::PowerOff as usize],
                state.session.is_some(),
            ]
            .into_iter()
            .enumerate()
            {
                self.rows[index].button.set_sensitive(enabled);
                self.rows[index].confirm.set_sensitive(enabled);
                if !enabled {
                    self.rows[index].revealer.set_reveal_child(false);
                }
            }
        }
        self.refreshing.set(false);
    }
    fn request_confirmation(self: &Rc<Self>, action: SystemAction) {
        self.hide_confirmations();
        if self.stopped.get() {
            return;
        }
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let weak = Rc::downgrade(self);
        (self.request)(Confirmation {
            action,
            respond: Box::new(move |confirmed| {
                if confirmed
                    && let Some(this) = weak.upgrade()
                    && !this.stopped.get()
                    && this.generation.get() == generation
                {
                    this.perform(action, generation);
                }
            }),
        });
    }
    fn perform(self: &Rc<Self>, action: SystemAction, generation: u64) {
        let old = self.operation.borrow_mut().take();
        if let Some(old) = old {
            old.cancel();
        }
        let completed = Rc::new(Cell::new(false));
        let done = completed.clone();
        let weak = Rc::downgrade(self);
        let callback = move |result: Result<(), glib::Error>| {
            done.set(true);
            if let Some(this) = weak.upgrade()
                && !this.stopped.get()
                && this.generation.get() == generation
            {
                this.operation.borrow_mut().take();
                this.menu
                    .widget()
                    .set_tooltip_text(result.err().map(|error| error.to_string()).as_deref());
            }
        };
        let operation = match action {
            SystemAction::Logout => self.logind.terminate_session(callback),
            action => {
                let power = match action {
                    SystemAction::Suspend => suspend_action(
                        &self.settings.string("suspend-method"),
                        &self.logind.state(),
                    ),
                    SystemAction::Restart => Some(PowerAction::Reboot),
                    SystemAction::PowerOff => Some(PowerAction::PowerOff),
                    SystemAction::Logout => unreachable!(),
                };
                let Some(power) = power else {
                    self.menu
                        .widget()
                        .set_tooltip_text(Some("No supported suspend method is available"));
                    return;
                };
                self.logind.perform(power, callback)
            }
        };
        if !completed.get() && !self.stopped.get() && self.generation.get() == generation {
            self.operation.replace(Some(operation));
        } else {
            operation.cancel();
        }
    }
}
impl Drop for PowerMenu {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sleep_preference_uses_existing_fallback_priority() {
        let mut state = LoginState::default();
        assert_eq!(suspend_action("suspend", &state), None);
        for action in [
            PowerAction::Hibernate,
            PowerAction::SuspendThenHibernate,
            PowerAction::HybridSleep,
            PowerAction::Suspend,
        ] {
            state.capabilities[action as usize] = true;
            assert_eq!(suspend_action("unknown", &state), Some(action));
        }
        assert_eq!(
            suspend_action("hibernate", &state),
            Some(PowerAction::Hibernate)
        );
        assert_eq!(
            suspend_action("suspend-then-hibernate", &state),
            Some(PowerAction::SuspendThenHibernate)
        );
    }
}
