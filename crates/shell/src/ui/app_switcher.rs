//! Application groups and instances backed by owned Wayland toplevel snapshots.
use super::{
    switcher::next_index,
    window::{LayerWindow, VisibilityController, WindowRole},
};
use crate::services::wayland::{Toplevel, ToplevelAction, WaylandService};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};

#[derive(Clone, Default)]
struct Group {
    app_id: String,
    instances: Vec<Toplevel>,
}

#[derive(Default)]
struct Selection {
    groups: Vec<Group>,
    selected: Option<u64>,
    last_active: Option<(String, u64)>,
    alternative_app: bool,
    preview: bool,
}

impl Selection {
    fn group_index(&self) -> Option<usize> {
        self.groups.iter().position(|group| {
            group
                .instances
                .iter()
                .any(|top| Some(top.id) == self.selected)
        })
    }

    fn change(&mut self, top: Toplevel) {
        let Some(app_id) = top.app_id.as_deref().filter(|id| !id.is_empty()) else {
            self.remove(top.id);
            return;
        };
        // A window may acquire or change its app ID after its first protocol batch.
        if self.groups.iter().any(|group| {
            group.app_id != app_id && group.instances.iter().any(|old| old.id == top.id)
        }) {
            let selected = self.selected == Some(top.id);
            self.remove(top.id);
            if selected {
                self.selected = Some(top.id);
            }
        }
        let group_index = self
            .groups
            .iter()
            .position(|group| group.app_id == app_id)
            .unwrap_or_else(|| {
                self.groups.push(Group {
                    app_id: app_id.to_owned(),
                    instances: Vec::new(),
                });
                self.groups.len() - 1
            });
        let group = &mut self.groups[group_index];
        if let Some(old) = group.instances.iter_mut().find(|old| old.id == top.id) {
            *old = top.clone();
        } else {
            group.instances.push(top.clone());
        }
        if top.active && !self.preview {
            let index = group
                .instances
                .iter()
                .position(|old| old.id == top.id)
                .unwrap();
            let instance = group.instances.remove(index);
            group.instances.insert(0, instance);
            let group = self.groups.remove(group_index);
            self.groups.insert(0, group);
            // Title/metadata batches on the already-active instance must not
            // change the next invocation's app-versus-instance choice.
            if self.last_active.as_ref().map(|(_, id)| *id) != Some(top.id) {
                self.alternative_app = self
                    .last_active
                    .as_ref()
                    .is_some_and(|(old, _)| old != app_id);
                self.last_active = Some((app_id.to_owned(), top.id));
            } else if let Some((name, _)) = &mut self.last_active {
                *name = app_id.to_owned();
            }
        }
    }

    fn remove(&mut self, id: u64) {
        let old_group = self.group_index();
        for group in &mut self.groups {
            group.instances.retain(|top| top.id != id);
        }
        self.groups.retain(|group| !group.instances.is_empty());
        if self.selected == Some(id) {
            self.selected = old_group
                .and_then(|index| {
                    self.groups
                        .get(index.min(self.groups.len().saturating_sub(1)))
                })
                .and_then(|group| group.instances.first())
                .map(|top| top.id);
        }
        if self.groups.is_empty() {
            self.last_active = None;
            self.alternative_app = false;
        }
    }

    fn focus_group(&mut self, index: usize) {
        self.selected = self.groups.get(index).and_then(|group| {
            let previous = self
                .last_active
                .as_ref()
                .is_some_and(|(app, _)| app == &group.app_id)
                && group.instances.len() > 1;
            group.instances.get(usize::from(previous)).map(|top| top.id)
        });
    }

    fn show(&mut self) -> bool {
        if self.groups.is_empty() {
            self.selected = None;
            return false;
        }
        self.focus_group(usize::from(self.alternative_app && self.groups.len() > 1));
        self.preview = true;
        true
    }

    fn next_app(&mut self, previous: bool) {
        if let Some(index) = next_index(self.groups.len(), self.group_index(), previous) {
            self.focus_group(index);
        }
    }

    fn next_instance(&mut self, previous: bool) -> Option<u64> {
        let group = self.groups.get(self.group_index()?)?;
        if group.instances.len() <= 1 {
            return None;
        }
        let current = group
            .instances
            .iter()
            .position(|top| Some(top.id) == self.selected);
        self.selected = next_index(group.instances.len(), current, previous)
            .map(|index| group.instances[index].id);
        self.selected
    }
}

pub struct AppSwitcher {
    service: WaylandService,
    visibility: VisibilityController,
    instances: LayerWindow,
    apps_box: gtk::Box,
    instances_box: gtk::Box,
    selection: RefCell<Selection>,
    handlers: RefCell<Vec<glib::SignalHandlerId>>,
    inhibited: Cell<bool>,
    icons: RefCell<BTreeMap<String, Option<gio::Icon>>>,
    rendering: Cell<bool>,
    render_pending: Cell<bool>,
    visibility_generation: Cell<u64>,
}

impl Drop for AppSwitcher {
    fn drop(&mut self) {
        for handler in self.handlers.get_mut().drain(..) {
            self.service.disconnect(handler);
        }
        if self.inhibited.replace(false) {
            self.service.restore_shortcuts();
        }
        self.instances.close();
        self.visibility.close();
    }
}

impl AppSwitcher {
    pub fn new(service: WaylandService) -> Result<Rc<Self>, String> {
        let main = LayerWindow::new(WindowRole::AppSwitcher, None)?;
        main.window().set_widget_name("app-switcher");
        main.window().set_hexpand(true);
        main.window().set_vexpand(true);
        let apps_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        apps_box.set_widget_name("app-switcher-list");
        let scroll = scrolled(&apps_box, 1400);
        main.window().set_content(Some(&scroll));
        let instances = LayerWindow::new(WindowRole::AppInstances, None)?;
        instances.window().set_widget_name("app-switcher");
        let instances_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        instances
            .window()
            .set_content(Some(&scrolled(&instances_box, 1200)));
        let this = Rc::new(Self {
            service,
            visibility: VisibilityController::new(main, None),
            instances,
            apps_box,
            instances_box,
            selection: RefCell::new(Selection::default()),
            handlers: RefCell::new(Vec::new()),
            inhibited: Cell::new(false),
            icons: RefCell::new(BTreeMap::new()),
            rendering: Cell::new(false),
            render_pending: Cell::new(false),
            visibility_generation: Cell::new(0),
        });
        for top in this.service.toplevels() {
            this.selection.borrow_mut().change(top);
        }
        for (signal, removed) in [("toplevel-changed", false), ("toplevel-removed", true)] {
            let weak = Rc::downgrade(&this);
            let handler = this.service.connect_local(signal, false, move |values| {
                if let Some(this) = weak.upgrade() {
                    let top = values[1].get::<Toplevel>().expect("Wayland signal type");
                    if removed {
                        this.selection.borrow_mut().remove(top.id);
                    } else {
                        this.selection.borrow_mut().change(top);
                    }
                    if this.selection.borrow().groups.is_empty() {
                        this.hide();
                    }
                    this.render();
                }
                None
            });
            this.handlers.borrow_mut().push(handler);
        }
        let weak = Rc::downgrade(&this);
        let handler = this.service.connect_local("failed", false, move |_| {
            if let Some(this) = weak.upgrade() {
                this.hide();
            }
            None
        });
        this.handlers.borrow_mut().push(handler);
        let weak = Rc::downgrade(&this);
        this.window().connect_map(move |_| {
            if let Some(this) = weak.upgrade() {
                this.inhibit();
            }
        });
        let weak = Rc::downgrade(&this);
        this.visibility.main().on_closed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.hide();
                this.instances.close();
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
        let weak = Rc::downgrade(&this);
        keys.connect_key_released(move |_, key, _, _| {
            if (key == gtk::gdk::Key::Super_L || key == gtk::gdk::Key::Super_R)
                && let Some(this) = weak.upgrade()
                && this.is_visible()
            {
                this.activate_selected();
            }
        });
        this.window().add_controller(keys);
        this.render();
        Ok(this)
    }

    pub fn window(&self) -> &adw::Window {
        self.visibility.main().window()
    }
    pub fn instances_window(&self) -> &adw::Window {
        self.instances.window()
    }
    pub fn is_visible(&self) -> bool {
        self.window().is_visible() && !self.visibility.main().is_closed()
    }
    pub fn selected_id(&self) -> Option<u64> {
        self.selection.borrow().selected
    }
    pub fn close(&self) {
        self.hide();
        self.visibility.close();
        self.instances.close();
    }

    pub fn show(self: &Rc<Self>) {
        let generation = self.next_visibility_generation();
        if self.visibility.main().is_closed() || !self.selection.borrow_mut().show() {
            self.hide();
            return;
        }
        if let Some(transition) = self.visibility.begin_show() {
            self.visibility.finish_transition(&transition);
        }
        if self.visibility_generation.get() != generation {
            return;
        }
        // An observer may reopen during the brief native unmap used for preview.
        self.visibility.main().present();
        if self.visibility_generation.get() != generation {
            return;
        }
        self.render();
        if self.visibility_generation.get() == generation {
            self.inhibit();
        }
    }

    pub fn hide(&self) {
        let generation = self.next_visibility_generation();
        {
            let mut selection = self.selection.borrow_mut();
            selection.preview = false;
            selection.selected = None;
            // Preview activation events were intentionally not used to reorder.
            // Refresh once after closing so subsequent invocations see the
            // compositor's actual selected application, including after Escape.
            for top in self.service.toplevels() {
                selection.change(top);
            }
        }
        if self.inhibited.replace(false) {
            self.service.restore_shortcuts();
        }
        if self.visibility_generation.get() != generation {
            return;
        }
        self.instances.hide();
        if self.visibility_generation.get() != generation {
            return;
        }
        self.visibility.hide_now();
    }

    fn next_visibility_generation(&self) -> u64 {
        let generation = self.visibility_generation.get().wrapping_add(1);
        self.visibility_generation.set(generation);
        generation
    }

    pub fn toggle(self: &Rc<Self>) {
        if self.is_visible() {
            self.hide();
        } else {
            self.show();
        }
    }

    pub fn next_app(self: &Rc<Self>, previous: bool) {
        if !self.is_visible() {
            return;
        }
        self.selection.borrow_mut().next_app(previous);
        self.render();
    }

    pub fn next_instance(self: &Rc<Self>, previous: bool) {
        if !self.is_visible() {
            return;
        }
        let generation = self.visibility_generation.get();
        let id = self.selection.borrow_mut().next_instance(previous);
        self.render();
        if let Some(id) = id
            && self.is_visible()
            && self.visibility_generation.get() == generation
            && self.selected_id() == Some(id)
        {
            self.activate(id);
            // Preserve Sway's focused-border preview behavior. A remapped GTK
            // surface needs a new inhibitor; never retain the old surface lease.
            if self.inhibited.replace(false) {
                self.service.restore_shortcuts();
            }
            self.instances.hide();
            if self.visibility_generation.get() != generation {
                return;
            }
            self.visibility.hide_now();
            if self.visibility_generation.get() != generation {
                return;
            }
            if let Some(transition) = self.visibility.begin_show() {
                self.visibility.finish_transition(&transition);
            }
            if self.visibility_generation.get() != generation {
                return;
            }
            self.render();
            if self.visibility_generation.get() == generation {
                self.inhibit();
            }
        }
    }

    pub fn activate_selected(&self) {
        if !self.is_visible() {
            return;
        }
        let id = self.selected_id();
        if let Some(id) = id {
            self.activate(id);
        }
        self.hide();
    }

    fn activate(&self, id: u64) {
        if let Err(error) = self.service.action(id, ToplevelAction::Activate) {
            glib::g_warning!("way-shell", "Cannot activate switcher window {id}: {error}");
        }
    }

    fn inhibit(&self) {
        if !self.inhibited.get()
            && self.is_visible()
            && self.window().is_mapped()
            && self.service.has_shortcut_inhibition()
        {
            match self.service.inhibit_shortcuts(self.window()) {
                Ok(()) => self.inhibited.set(true),
                Err(error) => glib::g_warning!(
                    "way-shell",
                    "Cannot inhibit app-switcher shortcuts: {error}"
                ),
            }
        }
    }

    fn render(self: &Rc<Self>) {
        self.render_pending.set(true);
        if self.rendering.replace(true) {
            return;
        }
        while self.render_pending.replace(false) && !self.visibility.main().is_closed() {
            self.render_once();
        }
        self.rendering.set(false);
    }

    fn render_once(self: &Rc<Self>) {
        // Copy state before GTK calls: widget observers may reenter the service.
        let (groups, selected) = {
            let selection = self.selection.borrow();
            (selection.groups.clone(), selection.selected)
        };
        clear_box(&self.apps_box);
        clear_box(&self.instances_box);
        self.icons
            .borrow_mut()
            .retain(|app_id, _| groups.iter().any(|group| &group.app_id == app_id));
        let selected_group = groups
            .iter()
            .find(|group| group.instances.iter().any(|top| Some(top.id) == selected));
        for group in &groups {
            let icon = self.icon(&group.app_id);
            let active = selected_group.is_some_and(|selected| selected.app_id == group.app_id);
            let (contents, button) = app_button(&group.app_id, icon.as_ref(), false);
            let arrow = gtk::Image::from_icon_name("pan-down-symbolic");
            arrow.add_css_class("app-switcher-app-widget-expand-arrow");
            arrow.set_pixel_size(12);
            arrow.set_visible(group.instances.len() > 1);
            contents.append(&arrow);
            if active {
                button.add_css_class("selected");
            }
            let app_id = group.app_id.clone();
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade()
                    && this.is_visible()
                {
                    let index = this
                        .selection
                        .borrow()
                        .groups
                        .iter()
                        .position(|group| group.app_id == app_id);
                    if let Some(index) = index {
                        if this.selection.borrow().group_index() != Some(index) {
                            this.selection.borrow_mut().focus_group(index);
                        }
                        this.activate_selected();
                    }
                }
            });
            self.apps_box.append(&contents);
            if active {
                button.grab_focus();
            }
        }
        if let Some(group) = selected_group.filter(|group| group.instances.len() > 1) {
            let icon = self.icon(&group.app_id);
            for top in &group.instances {
                let title = top.title.as_deref().unwrap_or(&group.app_id);
                let (contents, button) = app_button(title, icon.as_ref(), true);
                button.set_tooltip_text(Some(title));
                if selected == Some(top.id) {
                    button.add_css_class("selected");
                }
                let id = top.id;
                let weak = Rc::downgrade(self);
                button.connect_clicked(move |_| {
                    if let Some(this) = weak.upgrade()
                        && this.is_visible()
                    {
                        this.activate(id);
                        this.hide();
                    }
                });
                self.instances_box.append(&contents);
            }
            if self.is_visible() {
                self.instances.present();
            }
        } else {
            self.instances.hide();
        }
    }

    fn key_pressed(
        self: &Rc<Self>,
        key: gtk::gdk::Key,
        modifiers: gtk::gdk::ModifierType,
    ) -> glib::Propagation {
        if !self.is_visible() {
            return glib::Propagation::Proceed;
        }
        match key_action(key, modifiers) {
            Some(KeyAction::PreviousApp) => self.next_app(true),
            Some(KeyAction::NextApp) => self.next_app(false),
            Some(KeyAction::PreviousInstance) => self.next_instance(true),
            Some(KeyAction::NextInstance) => self.next_instance(false),
            Some(KeyAction::Cancel) => self.hide(),
            None => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    fn icon(&self, app_id: &str) -> Option<gio::Icon> {
        if let Some(icon) = self.icons.borrow().get(app_id) {
            return icon.clone();
        }
        let icon = desktop_icon(&gio::AppInfo::all(), app_id);
        self.icons
            .borrow_mut()
            .insert(app_id.to_owned(), icon.clone());
        icon
    }
}

#[derive(Debug, PartialEq, Eq)]
enum KeyAction {
    PreviousApp,
    NextApp,
    PreviousInstance,
    NextInstance,
    Cancel,
}

fn key_action(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> Option<KeyAction> {
    use gtk::gdk::{Key, ModifierType as Mods};
    if !modifiers.contains(Mods::SUPER_MASK) {
        return None;
    }
    let shift = modifiers.contains(Mods::SHIFT_MASK);
    Some(if key == Key::ISO_Left_Tab && shift {
        KeyAction::PreviousApp
    } else if key == Key::Tab {
        KeyAction::NextApp
    } else if (key == Key::asciitilde || key == Key::G) && shift {
        KeyAction::PreviousInstance
    } else if key == Key::grave || key == Key::g {
        KeyAction::NextInstance
    } else if key == Key::Escape {
        KeyAction::Cancel
    } else {
        return None;
    })
}

fn scrolled(child: &gtk::Box, max_width: i32) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .max_content_width(max_width)
        .max_content_height(-1)
        .propagate_natural_width(true)
        .propagate_natural_height(true)
        .child(child)
        .build()
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn desktop_icon(apps: &[gio::AppInfo], app_id: &str) -> Option<gio::Icon> {
    let query = glib::casefold(app_id);
    apps.iter()
        .find(|app| {
            app.id()
                .is_some_and(|id| glib::casefold(&id).contains(query.as_str()))
        })
        .and_then(|app| app.icon())
}

fn app_button(text: &str, icon: Option<&gio::Icon>, instance: bool) -> (gtk::Box, gtk::Button) {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    container.add_css_class("app-switcher-app-widget");
    let button = gtk::Button::new();
    button.add_css_class("app-switcher-app-widget-button");
    if instance {
        button.add_css_class("instance");
    }
    let contents = gtk::Box::new(gtk::Orientation::Vertical, 0);
    contents.set_halign(gtk::Align::Center);
    contents.set_valign(gtk::Align::Center);
    let image = gtk::Image::from_icon_name("application-x-executable");
    image.set_pixel_size(if instance { 48 } else { 64 });
    if let Some(icon) = icon {
        image.set_from_gicon(icon);
    }
    let label = gtk::Label::new(Some(text));
    label.add_css_class("app-switcher-app-widget-label");
    label.set_width_chars(if instance { 14 } else { 15 });
    label.set_max_width_chars(if instance { 14 } else { 15 });
    label.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    label.set_xalign(0.5);
    contents.append(&image);
    contents.append(&label);
    button.set_child(Some(&contents));
    container.append(&button);
    (container, button)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn top(id: u64, app: &str, active: bool) -> Toplevel {
        Toplevel {
            id,
            app_id: Some(app.into()),
            title: Some(format!("window {id}")),
            active,
            ..Default::default()
        }
    }
    #[test]
    fn keys_preserve_super_tab_instance_aliases_and_cancel() {
        use gtk::gdk::{Key, ModifierType as Mods};
        let super_key = Mods::SUPER_MASK;
        let shift = super_key | Mods::SHIFT_MASK;
        assert_eq!(key_action(Key::Tab, super_key), Some(KeyAction::NextApp));
        assert_eq!(
            key_action(Key::ISO_Left_Tab, shift),
            Some(KeyAction::PreviousApp)
        );
        for key in [Key::grave, Key::g] {
            assert_eq!(key_action(key, super_key), Some(KeyAction::NextInstance));
        }
        for key in [Key::asciitilde, Key::G] {
            assert_eq!(key_action(key, shift), Some(KeyAction::PreviousInstance));
        }
        assert_eq!(key_action(Key::Escape, super_key), Some(KeyAction::Cancel));
        assert_eq!(key_action(Key::Escape, Mods::empty()), None);
        assert_eq!(key_action(Key::Tab, Mods::empty()), None);
    }
    #[test]
    fn activation_distinguishes_previous_app_and_previous_instance() {
        let mut selection = Selection::default();
        selection.change(top(1, "editor", true));
        selection.change(top(2, "editor", false));
        selection.change(top(3, "browser", true));
        assert!(selection.show());
        assert_eq!(selection.selected, Some(1));
        selection.preview = false;
        selection.change(top(2, "editor", true));
        selection.change(top(1, "editor", true));
        selection.change(top(1, "editor", true)); // metadata must not reset the choice
        assert!(selection.show());
        assert_eq!(selection.selected, Some(2));
        assert_eq!(selection.next_instance(false), Some(1));
        assert_eq!(selection.next_instance(false), Some(2));
        assert_eq!(selection.next_instance(true), Some(1));
    }
    #[test]
    fn preview_updates_titles_without_reordering_and_removal_repairs_selection() {
        let mut selection = Selection::default();
        selection.change(top(1, "editor", true));
        selection.change(top(2, "editor", false));
        selection.change(top(3, "browser", true));
        selection.show();
        selection.change(top(2, "editor", true));
        assert_eq!(selection.groups[0].app_id, "browser");
        assert_eq!(selection.groups[1].instances[0].id, 1);
        selection.remove(1);
        assert_eq!(selection.selected, Some(2));
        selection.remove(2);
        assert_eq!(selection.selected, Some(3));
        selection.remove(3);
        assert!(!selection.show());
        assert_eq!(selection.selected, None);
    }
    #[test]
    fn changed_app_id_never_leaves_a_duplicate_or_null_activation_target() {
        let mut selection = Selection::default();
        selection.change(top(1, "editor", true));
        selection.change(top(2, "browser", true));
        selection.show();
        assert_eq!(selection.selected, Some(1));
        selection.change(top(1, "browser", false));
        assert_eq!(selection.groups.len(), 1);
        assert_eq!(selection.selected, Some(1));
        selection.show();
        assert_eq!(selection.selected, Some(1));
        selection.next_app(false);
        assert_eq!(selection.selected, Some(1));
        selection.change(Toplevel {
            id: 1,
            ..Default::default()
        });
        assert_eq!(selection.selected, Some(2));
    }
}
