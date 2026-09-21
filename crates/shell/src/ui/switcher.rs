//! Shared search/list surface used by workspace, output and rename views.
use super::window::{LayerWindow, Visibility, VisibilityController, WindowRole};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitcherEntry {
    /// Stable application-owned identity; labels are never used as identifiers.
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwitcherAction {
    Activate {
        selected: Option<String>,
        text: String,
        raw: bool,
    },
    Hidden,
}

enum Update {
    Entries(Vec<SwitcherEntry>),
    Filter,
    Navigate(bool),
    Show(u64),
}

pub struct Switcher {
    visibility: VisibilityController,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    rows: RefCell<Vec<(SwitcherEntry, gtk::ListBoxRow)>>,
    has_list: bool,
    allow_raw: bool,
    action: Box<dyn Fn(SwitcherAction)>,
    updates: RefCell<VecDeque<Update>>,
    updating: Cell<bool>,
    visibility_generation: Cell<u64>,
}

impl Switcher {
    pub fn new(
        has_list: bool,
        allow_raw: bool,
        action: impl Fn(SwitcherAction) + 'static,
    ) -> Result<Rc<Self>, String> {
        let surface = LayerWindow::new(WindowRole::Switcher, None)?;
        surface.window().set_widget_name("switcher");
        surface.window().set_hexpand(true);
        surface.window().set_vexpand(true);
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        let search_container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        search_container.set_widget_name("search-container");
        search_container.set_hexpand(true);
        let search = gtk::SearchEntry::new();
        search.set_widget_name("search-entry");
        search_container.append(&search);
        container.append(&search_container);
        let list = gtk::ListBox::new();
        list.set_widget_name("list");
        list.set_hexpand(true);
        list.set_vexpand(true);
        if has_list {
            let scrolled = gtk::ScrolledWindow::builder()
                .max_content_height(400)
                .propagate_natural_height(true)
                .propagate_natural_width(true)
                .child(&list)
                .build();
            container.append(&scrolled);
        } else {
            container.add_css_class("no_list");
        }
        surface.window().set_content(Some(&container));
        let this = Rc::new(Self {
            visibility: VisibilityController::new(surface, None),
            search,
            list,
            rows: RefCell::new(Vec::new()),
            has_list,
            allow_raw,
            action: Box::new(action),
            updates: RefCell::new(VecDeque::new()),
            updating: Cell::new(false),
            visibility_generation: Cell::new(0),
        });
        let weak = Rc::downgrade(&this);
        this.search.connect_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.filter();
            }
        });
        let weak = Rc::downgrade(&this);
        this.search.connect_activate(move |_| {
            if let Some(this) = weak.upgrade() {
                this.activate(false);
            }
        });
        let weak = Rc::downgrade(&this);
        this.search.connect_stop_search(move |_| {
            if let Some(this) = weak.upgrade() {
                this.cancel();
            }
        });
        let weak = Rc::downgrade(&this);
        this.search.connect_next_match(move |_| {
            if let Some(this) = weak.upgrade() {
                this.navigate(false);
            }
        });
        let weak = Rc::downgrade(&this);
        this.search.connect_previous_match(move |_| {
            if let Some(this) = weak.upgrade() {
                this.navigate(true);
            }
        });
        let weak = Rc::downgrade(&this);
        this.list.connect_row_activated(move |list, row| {
            list.select_row(Some(row));
            if let Some(this) = weak.upgrade() {
                this.activate(false);
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_name(Some("way-shell-switcher-keys"));
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&this);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            weak.upgrade().map_or(glib::Propagation::Proceed, |this| {
                this.key_pressed(key, modifiers)
            })
        });
        this.window().add_controller(keys);
        Ok(this)
    }

    pub fn window(&self) -> &adw::Window {
        self.visibility.main().window()
    }
    pub fn search(&self) -> &gtk::SearchEntry {
        &self.search
    }
    pub fn list(&self) -> &gtk::ListBox {
        &self.list
    }
    pub fn is_visible(&self) -> bool {
        matches!(
            self.visibility.visibility(),
            Visibility::Showing | Visibility::Visible
        )
    }
    pub fn close(&self) {
        self.next_visibility_generation();
        self.updates.borrow_mut().clear();
        self.visibility.close();
    }

    pub fn set_entries(&self, entries: Vec<SwitcherEntry>) {
        self.update(Update::Entries(entries));
    }

    fn update(&self, update: Update) {
        if self.visibility.main().is_closed() {
            return;
        }
        self.updates.borrow_mut().push_back(update);
        if self.updating.replace(true) {
            return;
        }
        loop {
            let update = self.updates.borrow_mut().pop_front();
            let Some(update) = update else {
                break;
            };
            if self.visibility.main().is_closed() {
                self.updates.borrow_mut().clear();
                break;
            }
            // GTK row/visibility notifications may request another update. Keep
            // native list mutation on this stack and drain owned requests after
            // it returns, so a nested inventory never gets overwritten.
            match update {
                Update::Entries(entries) => self.replace_entries(entries),
                Update::Filter => self.apply_filter(),
                Update::Navigate(previous) => self.apply_navigation(previous),
                Update::Show(generation) => self.finish_show(generation),
            }
        }
        self.updating.set(false);
    }

    fn replace_entries(&self, entries: Vec<SwitcherEntry>) {
        let unchanged = {
            let rows = self.rows.borrow();
            rows.len() == entries.len() && rows.iter().map(|(entry, _)| entry).eq(entries.iter())
        };
        if unchanged {
            return;
        }
        let selected = self.selected_id();
        let old_rows = self.rows.take();
        drop(old_rows);
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut rows = Vec::with_capacity(entries.len());
        for entry in entries {
            let contents = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            contents.add_css_class("switcher-entry");
            contents.append(&gtk::Label::new(Some(&entry.label)));
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&contents));
            self.list.append(&row);
            rows.push((entry, row));
        }
        self.rows.replace(rows);
        self.apply_filter();
        let previous = self
            .rows
            .borrow()
            .iter()
            .find(|(entry, row)| Some(&entry.id) == selected.as_ref() && row.is_visible())
            .map(|(_, row)| row.clone());
        if let Some(row) = previous {
            self.list.select_row(Some(&row));
        }
    }

    fn filter(&self) {
        self.update(Update::Filter);
    }

    fn apply_filter(&self) {
        let text = self.search.text();
        let previous = self.list.selected_row();
        let rows = self.rows.borrow().clone();
        for (entry, row) in &rows {
            row.set_visible(matches(&entry.label, &text));
        }
        let keep_previous =
            text.is_empty() && previous.as_ref().is_some_and(|row| row.is_visible());
        self.list.select_row(if keep_previous {
            previous.as_ref()
        } else {
            rows.iter()
                .find(|(_, row)| row.is_visible())
                .map(|(_, row)| row)
        });
    }

    pub fn selected_id(&self) -> Option<String> {
        let selected = self.list.selected_row()?;
        self.rows
            .borrow()
            .iter()
            .find(|(_, row)| row == &selected && row.is_visible())
            .map(|(entry, _)| entry.id.clone())
    }

    pub fn navigate(&self, previous: bool) {
        self.update(Update::Navigate(previous));
    }

    fn apply_navigation(&self, previous: bool) {
        let rows: Vec<_> = self
            .rows
            .borrow()
            .iter()
            .filter(|(_, row)| row.is_visible())
            .map(|(_, row)| row.clone())
            .collect();
        let selected = self.list.selected_row();
        let current = rows.iter().position(|row| Some(row) == selected.as_ref());
        if let Some(index) = next_index(rows.len(), current, previous) {
            self.list.select_row(Some(&rows[index]));
            rows[index].grab_focus();
        }
    }

    pub fn show(&self) {
        if self.visibility.main().is_closed() {
            return;
        }
        let generation = self.next_visibility_generation();
        if let Some(transition) = self.visibility.begin_show() {
            self.visibility.finish_transition(&transition);
        }
        self.update(Update::Show(generation));
    }

    fn finish_show(&self, generation: u64) {
        if !self.current_show(generation) {
            return;
        }
        self.apply_filter();
        if !self.current_show(generation) {
            return;
        }
        let first = self
            .rows
            .borrow()
            .iter()
            .find(|(_, row)| row.is_visible())
            .map(|(_, row)| row.clone());
        self.list.select_row(first.as_ref());
        if self.current_show(generation) {
            self.search.grab_focus();
        }
    }

    pub fn hide(&self) {
        let was_visible = self.is_visible();
        let generation = self.next_visibility_generation();
        self.visibility.hide_now();
        if self.visibility_generation.get() != generation {
            return;
        }
        self.search.set_text("");
        if was_visible && self.visibility_generation.get() == generation {
            (self.action)(SwitcherAction::Hidden);
        }
    }

    fn next_visibility_generation(&self) -> u64 {
        let generation = self.visibility_generation.get().wrapping_add(1);
        self.visibility_generation.set(generation);
        generation
    }

    fn current_show(&self, generation: u64) -> bool {
        self.visibility_generation.get() == generation && self.is_visible()
    }

    pub fn toggle(&self) {
        if self.is_visible() {
            self.hide();
        } else {
            self.show();
        }
    }

    pub fn cancel(&self) {
        if self.search.text().is_empty() {
            self.hide();
        } else {
            self.search.set_text("");
        }
    }

    pub fn activate(&self, raw: bool) {
        if !self.is_visible() {
            return;
        }
        (self.action)(SwitcherAction::Activate {
            selected: if raw { None } else { self.selected_id() },
            text: self.search.text().to_string(),
            raw,
        });
    }

    fn key_pressed(
        &self,
        key: gtk::gdk::Key,
        modifiers: gtk::gdk::ModifierType,
    ) -> glib::Propagation {
        use gtk::gdk::{Key, ModifierType as Mods};
        if key == Key::Escape {
            self.cancel();
        } else if self.has_list
            && (key == Key::Down
                || key == Key::Tab
                || key == Key::n && modifiers.contains(Mods::CONTROL_MASK))
        {
            self.navigate(false);
        } else if self.has_list
            && (key == Key::Up
                || key == Key::ISO_Left_Tab && modifiers.contains(Mods::SHIFT_MASK)
                || key == Key::p && modifiers.contains(Mods::CONTROL_MASK))
        {
            self.navigate(true);
        } else if key == Key::Return && modifiers.contains(Mods::CONTROL_MASK) && self.allow_raw {
            self.activate(true);
        } else {
            return glib::Propagation::Proceed;
        }
        glib::Propagation::Stop
    }
}

fn matches(label: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let (Ok(label), Ok(query)) = (std::ffi::CString::new(label), std::ffi::CString::new(query))
    else {
        return false;
    };
    // GLib owns no input memory here. The high-level crate does not expose this
    // token/alternate matcher; keep the exact existing function in one wrapper.
    unsafe { glib::ffi::g_str_match_string(query.as_ptr(), label.as_ptr(), 1) != 0 }
}

pub(super) fn next_index(len: usize, current: Option<usize>, previous: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match current.filter(|index| *index < len) {
        None => {
            if previous {
                len - 1
            } else {
                0
            }
        }
        Some(0) if previous => len - 1,
        Some(index) if previous => index - 1,
        Some(index) => (index + 1) % len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matching_keeps_glib_token_prefix_and_alternate_semantics() {
        assert!(matches("Work Café", "caf"));
        assert!(matches("Work Café", "work cafe"));
        assert!(matches("日本語", "日本"));
        assert!(!matches("Work Café", "ork"));
        assert!(matches("Work Café", ""));
    }
    #[test]
    fn navigation_handles_empty_filtered_and_removed_selection() {
        assert_eq!(next_index(0, None, true), None);
        assert_eq!(next_index(2, Some(1), false), Some(0));
        assert_eq!(next_index(2, Some(0), true), Some(1));
        assert_eq!(next_index(2, Some(9), false), Some(0));
        assert_eq!(next_index(2, None, true), Some(1));
    }
}
