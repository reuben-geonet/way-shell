//! Grouped, scrollable shortcut sheet, inspired by nwg-shell-config's help window.
use super::panel_popup::{PanelPopup, PopupEvent};
use crate::services::{
    shortcuts::{ShortcutsService, Status},
    wm::WindowManager,
};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub const NAVIGATION: &[(&str, &str)] = &[
    ("Escape", "Close the shortcut sheet"),
    (
        "↑ / ↓ / Page Up / Page Down",
        "Scroll the sheet when the scrolling area has focus",
    ),
    ("Tab / Shift+Tab", "Move focus between controls"),
];
pub struct ShortcutsSheet {
    popup: Rc<PanelPopup>,
    service: Rc<ShortcutsService>,
    manager: WindowManager,
    rows: gtk::Box,
    scroller: gtk::ScrolledWindow,
    subscription: Cell<Option<u64>>,
    scroll_restore: RefCell<Option<glib::SourceId>>,
    stopped: Cell<bool>,
}
impl ShortcutsSheet {
    pub fn new(service: Rc<ShortcutsService>, manager: WindowManager) -> Result<Rc<Self>, String> {
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 12);
        rows.add_css_class("shortcuts-sections");
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_vexpand(true);
        scroller.set_hexpand(true);
        scroller.set_child(Some(&rows));
        let popup = PanelPopup::new(
            "help-about-symbolic",
            "Keyboard shortcuts",
            &scroller,
            680,
            680,
        )?;
        popup.window().add_css_class("shortcuts-sheet");
        let this = Rc::new(Self {
            popup,
            service,
            manager,
            rows,
            scroller,
            subscription: Cell::new(None),
            scroll_restore: RefCell::new(None),
            stopped: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        let id = this.service.on_changed(move || {
            if let Some(this) = weak.upgrade() {
                this.render();
            }
        });
        this.subscription.set(Some(id));
        let weak = Rc::downgrade(&this);
        this.popup.on_event(move |event| {
            if let Some(this) = weak.upgrade() {
                match event {
                    PopupEvent::Visible => this.service.set_visible(true),
                    PopupEvent::Hidden => this.service.set_visible(false),
                    PopupEvent::Dismissed => (),
                }
            }
        });
        this.render();
        Ok(this)
    }
    pub fn popup(&self) -> &Rc<PanelPopup> {
        &self.popup
    }
    pub fn show(&self) -> Result<(), String> {
        self.popup.show(&self.focused_monitor()?)
    }
    pub fn toggle(&self) -> Result<(), String> {
        if self.popup.is_visible() {
            self.popup.hide();
            Ok(())
        } else {
            self.show()
        }
    }
    pub fn hide(&self) {
        self.popup.hide();
    }
    pub fn toggle_on(&self, monitor: &gtk::gdk::Monitor) -> Result<(), String> {
        self.popup.toggle(monitor)
    }
    fn focused_monitor(&self) -> Result<gtk::gdk::Monitor, String> {
        let display = gtk::gdk::Display::default().ok_or("No display is available")?;
        let monitors = display.monitors();
        let output = self
            .manager
            .workspaces()
            .into_iter()
            .find(|w| w.focused)
            .and_then(|w| w.output);
        let valid: Vec<_> = (0..monitors.n_items())
            .filter_map(|i| monitors.item(i).and_downcast::<gtk::gdk::Monitor>())
            .filter(|m| m.is_valid())
            .collect();
        valid
            .iter()
            .find(|m| m.connector().as_deref() == output.as_deref())
            .or_else(|| valid.first())
            .cloned()
            .ok_or_else(|| "No display outputs are available".into())
    }
    fn render(self: &Rc<Self>) {
        if self.stopped.get() {
            return;
        }
        let scroll = self.scroller.vadjustment().value();
        if let Some(source) = self.scroll_restore.borrow_mut().take() {
            source.remove();
        }
        while let Some(child) = self.rows.first_child() {
            self.rows.remove(&child);
        }
        let data = self.service.data();
        let explanation = label(
            "Saved configuration. Sway edits may need a compositor reload before they take effect.",
        );
        explanation.add_css_class("dim-label");
        self.rows.append(&explanation);
        let status = match data.status {
            Status::Loading => Some("Loading configured shortcuts…"),
            Status::Unavailable => {
                Some("Configuration is unavailable. Internal navigation is still available below.")
            }
            Status::Stale => Some(
                "Saved configuration could not be loaded. Showing the previous complete shortcuts for this file.",
            ),
            Status::Incomplete => {
                Some("Configuration contains errors. These shortcuts are incomplete.")
            }
            Status::Current if data.rows.is_empty() => {
                Some("No compositor or Way-Shell shortcuts found.")
            }
            Status::Current => None,
        };
        if let Some(status) = status {
            let widget = label(status);
            widget.add_css_class("shortcuts-status");
            self.rows.append(&widget);
        }
        if data.reload_failed {
            self.rows.append(&label(
                "Niri rejected the configuration reload. Saved shortcuts may not be active.",
            ));
        }
        if data.snapshot.mod_legend {
            self.rows.append(&label("Mod: the compositor's configured modifier. Desktop/nested context could not be established."));
        }
        if matches!(
            data.status,
            Status::Unavailable | Status::Stale | Status::Incomplete
        ) {
            let retry = gtk::Button::with_label("Retry");
            retry.set_halign(gtk::Align::Start);
            let weak = Rc::downgrade(self);
            retry.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade().filter(|this| !this.stopped.get()) {
                    this.service.retry();
                }
            });
            self.rows.append(&retry);
        }
        let mut section = String::new();
        let mut grid = gtk::Grid::new();
        let mut index = 0;
        for row in data.rows {
            if section != row.section {
                section = row.section.clone();
                grid = self.section(&section);
                index = 0;
            }
            let key = label(&row.keys.join(" / "));
            key.add_css_class("shortcut-key");
            key.set_hexpand(false);
            key.set_width_chars(24);
            key.set_max_width_chars(24);
            grid.attach(&key, 0, index, 1, 1);
            let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
            content.set_hexpand(true);
            content.append(&label(&row.description));
            let details = gtk::Expander::new(Some("Details"));
            details.add_css_class("shortcuts-details");
            let source = label(&row.details.join("\n\n"));
            source.set_selectable(true);
            details.set_child(Some(&source));
            content.append(&details);
            grid.attach(&content, 1, index, 1, 1);
            index += 1;
        }
        let navigation = self.section("Inside Way-Shell");
        let contexts = [
            ("Activities", super::activities::NAVIGATION),
            (
                "Workspace and output selectors",
                super::switcher::NAVIGATION,
            ),
            ("Workspace selectors", super::workspace_switcher::NAVIGATION),
            ("Rename selector", super::rename_switcher::NAVIGATION),
            ("App switcher — hold Super", super::app_switcher::NAVIGATION),
            ("Shortcut sheet", NAVIGATION),
        ];
        let mut row = 0;
        for (context, entries) in contexts {
            let heading = label(context);
            heading.add_css_class("heading");
            navigation.attach(&heading, 0, row, 2, 1);
            row += 1;
            for (key, description) in entries {
                let key = label(key);
                key.add_css_class("shortcut-key");
                key.set_hexpand(false);
                key.set_width_chars(24);
                key.set_max_width_chars(24);
                navigation.attach(&key, 0, row, 1, 1);
                navigation.attach(&label(description), 1, row, 1, 1);
                row += 1;
            }
        }
        self.rows.append(&label("The app switcher's internal controls always require Super, even when its opening binding uses another modifier."));
        let mut diagnostics = data.messages;
        diagnostics.extend(data.snapshot.diagnostics.iter().map(|d| {
            format!(
                "{}:{}: {}",
                d.source.path.display(),
                d.source.line,
                d.message
            )
        }));
        if let Some(path) = data.source {
            diagnostics.insert(0, format!("Source: {}", path.display()));
        }
        if !diagnostics.is_empty() {
            let details = gtk::Expander::new(Some("Configuration details"));
            details.add_css_class("shortcuts-details");
            let text = label(&diagnostics.join("\n"));
            text.set_selectable(true);
            details.set_child(Some(&text));
            self.rows.append(&details);
        }
        let weak = Rc::downgrade(self);
        self.scroll_restore
            .replace(Some(glib::idle_add_local_once(move || {
                if let Some(this) = weak.upgrade().filter(|this| !this.stopped.get()) {
                    this.scroll_restore.borrow_mut().take();
                    let adjustment = this.scroller.vadjustment();
                    adjustment.set_value(
                        scroll.min((adjustment.upper() - adjustment.page_size()).max(0.0)),
                    );
                }
            })));
    }
    fn section(&self, name: &str) -> gtk::Grid {
        self.rows
            .append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let heading = label(name);
        heading.add_css_class("heading");
        heading.add_css_class("shortcuts-heading");
        self.rows.append(&heading);
        let grid = gtk::Grid::new();
        grid.set_column_spacing(18);
        grid.set_row_spacing(10);
        grid.set_hexpand(true);
        self.rows.append(&grid);
        grid
    }
    pub fn close(&self) {
        if self.stopped.replace(true) {
            return;
        }
        if let Some(id) = self.subscription.take() {
            self.service.disconnect(id);
        }
        if let Some(source) = self.scroll_restore.borrow_mut().take() {
            source.remove();
        }
        self.service.set_visible(false);
        self.popup.shutdown();
    }
}
impl Drop for ShortcutsSheet {
    fn drop(&mut self) {
        self.close();
    }
}
fn label(text: &str) -> gtk::Label {
    let truncated;
    let text = if text.chars().count() > 8192 {
        truncated = format!(
            "{}… (see source file for the full text)",
            text.chars().take(8192).collect::<String>()
        );
        &truncated
    } else {
        text
    };
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_valign(gtk::Align::Start);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_hexpand(true);
    label
}
