//! Application carousel, search and launching on an owned layer surface.
use crate::{
    services::apps::{AppCatalog, Application, matches},
    ui::window::{LayerWindow, Transition, Visibility, VisibilityController, WindowRole},
};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    rc::Rc,
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivitiesEvent {
    WillShow,
    Visible,
    WillHide,
    Hidden,
}
type Observer = Rc<dyn Fn(ActivitiesEvent)>;
struct Inner {
    visibility: VisibilityController,
    catalog: AppCatalog,
    catalog_handler: RefCell<Option<glib::SignalHandlerId>>,
    handlers: RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>,
    revealer: gtk::Revealer,
    search: gtk::SearchEntry,
    results: gtk::FlowBox,
    scrolled: gtk::ScrolledWindow,
    carousel: adw::Carousel,
    dots: adw::CarouselIndicatorDots,
    pages: RefCell<Vec<gtk::Grid>>,
    apps: RefCell<HashMap<gtk::FlowBoxChild, Application>>,
    inventory_revision: Cell<u64>,
    rebuilding: Cell<bool>,
    pending_rebuild: Cell<bool>,
    updating_search: Cell<bool>,
    pending_search: Cell<bool>,
    desired: Cell<bool>,
    announced: Cell<bool>,
    request_revision: Cell<u64>,
    transition: RefCell<Option<(Transition, bool)>>,
    finish_task: RefCell<Option<glib::JoinHandle<()>>>,
    closed: Cell<bool>,
    observers: RefCell<Vec<Observer>>,
    events: RefCell<VecDeque<ActivitiesEvent>>,
    publishing: Cell<bool>,
}
#[derive(Clone)]
pub struct Activities(Rc<Inner>);
impl Activities {
    pub fn new(catalog: AppCatalog) -> Result<Self, String> {
        let surface = LayerWindow::new(WindowRole::Activities, None)?;
        surface.window().set_widget_name("activities");
        surface.window().set_hexpand(true);
        surface.window().set_vexpand(true);
        let visibility = VisibilityController::new(surface, None);
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.set_widget_name("activities-container");
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        revealer.set_transition_duration(300);
        revealer.set_hexpand(true);
        revealer.set_vexpand(true);
        revealer.set_child(Some(&container));
        let search_container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        search_container.set_widget_name("search-container");
        search_container.set_hexpand(true);
        let search = gtk::SearchEntry::new();
        search.set_widget_name("search-entry");
        search.set_halign(gtk::Align::Center);
        search.set_size_request(480, 120);
        search.set_placeholder_text(Some(
            "Search Applications...ctrl-g for next, ctrl-s-g for prev",
        ));
        search_container.append(&search);
        container.append(&search_container);
        let results = gtk::FlowBox::new();
        results.set_column_spacing(20);
        results.set_row_spacing(20);
        results.set_halign(gtk::Align::Center);
        results.set_valign(gtk::Align::Start);
        results.set_homogeneous(true);
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_child(Some(&results));
        scrolled.set_vexpand(true);
        scrolled.set_visible(false);
        container.append(&scrolled);
        let carousel = adw::Carousel::new();
        let dots = adw::CarouselIndicatorDots::new();
        dots.set_carousel(Some(&carousel));
        container.append(&carousel);
        container.append(&dots);
        visibility.main().window().set_content(Some(&revealer));
        let inner = Rc::new(Inner {
            visibility,
            catalog,
            catalog_handler: RefCell::new(None),
            handlers: RefCell::new(Vec::new()),
            revealer,
            search,
            results,
            scrolled,
            carousel,
            dots,
            pages: RefCell::new(Vec::new()),
            apps: RefCell::new(HashMap::new()),
            inventory_revision: Cell::new(0),
            rebuilding: Cell::new(false),
            pending_rebuild: Cell::new(false),
            updating_search: Cell::new(false),
            pending_search: Cell::new(false),
            desired: Cell::new(false),
            announced: Cell::new(false),
            request_revision: Cell::new(0),
            transition: RefCell::new(None),
            finish_task: RefCell::new(None),
            closed: Cell::new(false),
            observers: RefCell::new(Vec::new()),
            events: RefCell::new(VecDeque::new()),
            publishing: Cell::new(false),
        });
        inner.connect();
        inner.rebuild();
        Ok(Self(inner))
    }
    pub fn show(&self) -> bool {
        self.0.show()
    }
    pub fn hide(&self) -> bool {
        self.0.hide()
    }
    pub fn toggle(&self) {
        if self.0.desired.get() {
            self.hide();
        } else {
            self.show();
        }
    }
    pub fn window(&self) -> &adw::Window {
        self.0.visibility.main().window()
    }
    pub fn visibility_controller(&self) -> &VisibilityController {
        &self.0.visibility
    }
    /// Capture other UI owners weakly when subscribing to lifecycle events.
    pub fn on_event(&self, observer: impl Fn(ActivitiesEvent) + 'static) {
        self.0.observers.borrow_mut().push(Rc::new(observer));
    }
    pub fn close(&self) {
        self.0.visibility.close();
    }
}
impl Inner {
    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.visibility.main().on_closed(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.on_closed();
            }
        });
        let weak = Rc::downgrade(self);
        let handler = self.catalog.connect_local("changed", false, move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.rebuild();
            }
            None
        });
        self.catalog_handler.replace(Some(handler));
        let weak = Rc::downgrade(self);
        let handler = self.search.connect_search_changed(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.update_search();
            }
        });
        self.handlers
            .borrow_mut()
            .push((self.search.clone().upcast(), handler));
        let weak = Rc::downgrade(self);
        let handler = self.search.connect_stop_search(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.clear_search();
            }
        });
        self.handlers
            .borrow_mut()
            .push((self.search.clone().upcast(), handler));
        let weak = Rc::downgrade(self);
        let handler = self.search.connect_activate(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.launch_selected();
            }
        });
        self.handlers
            .borrow_mut()
            .push((self.search.clone().upcast(), handler));
        let weak = Rc::downgrade(self);
        let handler = self.search.connect_next_match(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.move_selection(1);
            }
        });
        self.handlers
            .borrow_mut()
            .push((self.search.clone().upcast(), handler));
        let weak = Rc::downgrade(self);
        let handler = self.search.connect_previous_match(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.move_selection(-1);
            }
        });
        self.handlers
            .borrow_mut()
            .push((self.search.clone().upcast(), handler));
        let weak = Rc::downgrade(self);
        self.results.set_filter_func(move |child| {
            let Some(inner) = weak.upgrade().filter(|inner| !inner.closed.get()) else {
                return false;
            };
            let query = inner.search.text();
            inner
                .apps
                .borrow()
                .get(child)
                .is_some_and(|app| query.is_empty() || matches(&query, &app.name))
        });
        let controller = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        let handler = controller.connect_key_pressed(move |_, key, _, modifiers| {
            if let Some(direction) = navigation(key, modifiers)
                && let Some(inner) = weak.upgrade().filter(|inner| !inner.closed.get())
            {
                inner.move_selection(direction);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.handlers
            .borrow_mut()
            .push((controller.clone().upcast(), handler));
        self.visibility.main().window().add_controller(controller);
        let weak = Rc::downgrade(self);
        let handler = self.revealer.connect_child_revealed_notify(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.finish_if_ready();
            }
        });
        self.handlers
            .borrow_mut()
            .push((self.revealer.clone().upcast(), handler));
    }
    fn publish(&self, event: ActivitiesEvent) {
        self.events.borrow_mut().push_back(event);
        if self.publishing.replace(true) {
            return;
        }
        loop {
            let event = self.events.borrow_mut().pop_front();
            let Some(event) = event else {
                break;
            };
            let observers = self.observers.borrow().clone();
            for observer in observers {
                observer(event);
            }
        }
        self.publishing.set(false);
    }
    fn show(self: &Rc<Self>) -> bool {
        if self.closed.get() || self.desired.get() {
            return false;
        }
        self.desired.set(true);
        self.announced.set(true);
        let revision = self.request_revision.get().wrapping_add(1);
        self.request_revision.set(revision);
        self.publish(ActivitiesEvent::WillShow);
        if self.closed.get() || self.request_revision.get() != revision {
            return false;
        }
        if self.catalog.is_dirty() {
            self.catalog.refresh();
        }
        let Some(transition) = self.visibility.begin_show() else {
            return false;
        };
        if self.closed.get() || self.request_revision.get() != revision {
            return false;
        }
        self.start_transition(transition, true);
        self.revealer.set_reveal_child(true);
        if self.desired.get() && !self.closed.get() {
            self.search.grab_focus();
        }
        self.finish_if_ready();
        true
    }
    fn hide(self: &Rc<Self>) -> bool {
        if self.closed.get() || !self.desired.get() {
            return false;
        }
        self.desired.set(false);
        let revision = self.request_revision.get().wrapping_add(1);
        self.request_revision.set(revision);
        self.publish(ActivitiesEvent::WillHide);
        if self.closed.get() || self.request_revision.get() != revision {
            return false;
        }
        self.clear_search();
        if self.closed.get() || self.request_revision.get() != revision {
            return false;
        }
        if let Some(transition) = self.visibility.begin_hide() {
            self.start_transition(transition, false);
            self.revealer.set_reveal_child(false);
            self.finish_if_ready();
        } else if self.visibility.visibility() == Visibility::Hidden {
            self.announced.set(false);
            self.publish(ActivitiesEvent::Hidden);
        }
        true
    }
    fn start_transition(self: &Rc<Self>, transition: Transition, showing: bool) {
        self.cancel_timer();
        self.transition.replace(Some((transition, showing)));
        let revision = self.request_revision.get();
        let weak = Rc::downgrade(self);
        self.finish_task
            .replace(Some(glib::MainContext::ref_thread_default().spawn_local(
                async move {
                    glib::timeout_future(Duration::from_millis(350)).await;
                    if let Some(inner) = weak.upgrade().filter(|inner| {
                        !inner.closed.get() && inner.request_revision.get() == revision
                    }) {
                        inner.finish_task.borrow_mut().take();
                        inner.finish();
                    }
                },
            )));
    }
    fn finish_if_ready(&self) {
        let showing = self
            .transition
            .borrow()
            .as_ref()
            .map(|(_, showing)| *showing);
        if showing.is_some_and(|showing| {
            self.revealer.reveals_child() == showing && self.revealer.is_child_revealed() == showing
        }) {
            self.finish();
        }
    }
    fn finish(&self) {
        let revision = self.request_revision.get();
        let pending = self.transition.borrow_mut().take();
        let Some((transition, showing)) = pending else {
            return;
        };
        self.cancel_timer();
        if self.visibility.finish_transition(&transition)
            && self.request_revision.get() == revision
            && !self.closed.get()
        {
            if !showing {
                self.announced.set(false);
            }
            self.publish(if showing {
                ActivitiesEvent::Visible
            } else {
                ActivitiesEvent::Hidden
            });
        }
    }
    fn cancel_timer(&self) {
        if let Some(task) = self.finish_task.borrow_mut().take() {
            task.abort();
        }
    }
    fn clear_search(&self) {
        self.search.set_text("");
        self.update_search();
    }
    fn update_search(&self) {
        if self.closed.get() {
            return;
        }
        self.pending_search.set(true);
        if self.updating_search.replace(true) {
            return;
        }
        while self.pending_search.replace(false) && !self.closed.get() {
            let searching = !self.search.text().is_empty();
            self.carousel.set_visible(!searching);
            self.dots.set_visible(!searching);
            self.results.unselect_all();
            self.results.invalidate_filter();
            if searching {
                let mut child = self.results.first_child();
                while let Some(next) = child {
                    child = next.next_sibling();
                    if next.is_child_visible() {
                        if let Ok(child) = next.downcast::<gtk::FlowBoxChild>() {
                            self.results.select_child(&child);
                        }
                        break;
                    }
                }
            }
            self.scrolled.set_visible(searching);
        }
        self.updating_search.set(false);
    }
    fn move_selection(&self, direction: i32) {
        if self.closed.get() {
            return;
        }
        let Some(selected) = self.results.selected_children().first().cloned() else {
            return;
        };
        let mut next = if direction > 0 {
            selected.next_sibling()
        } else {
            selected.prev_sibling()
        };
        while let Some(child) = next {
            next = if direction > 0 {
                child.next_sibling()
            } else {
                child.prev_sibling()
            };
            if child.is_child_visible() {
                if let Ok(child) = child.downcast::<gtk::FlowBoxChild>() {
                    self.results.select_child(&child);
                }
                break;
            }
        }
    }
    fn launch_selected(self: &Rc<Self>) {
        let selected = self.results.selected_children().first().cloned();
        let app = selected.and_then(|child| self.apps.borrow().get(&child).cloned());
        if let Some(app) = app {
            self.launch(&app.id, self.inventory_revision.get());
        }
    }
    fn launch(self: &Rc<Self>, id: &str, revision: u64) {
        if self.closed.get() || self.inventory_revision.get() != revision {
            return;
        }
        self.catalog.launch(id, |result| {
            if let Err(error) = result {
                glib::g_message!("way-shell", "Failed to launch application: {error}");
            }
        });
        self.hide();
    }
    fn app_widget(self: &Rc<Self>, app: &Application, revision: u64) -> gtk::Box {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("activities-app-widget");
        root.set_halign(gtk::Align::Center);
        root.set_valign(gtk::Align::Center);
        let button = gtk::Button::new();
        button.set_size_request(192, 192);
        let contents = gtk::Box::new(gtk::Orientation::Vertical, 0);
        contents.set_halign(gtk::Align::Center);
        contents.set_valign(gtk::Align::Center);
        let icon = gtk::Image::from_icon_name("image-missing");
        icon.add_css_class("activities-app-widget-icon");
        icon.set_pixel_size(78);
        if let Some(serialized) = &app.icon
            && let Ok(value) = gio::Icon::for_string(serialized)
        {
            icon.set_from_gicon(&value);
        }
        let label = gtk::Label::new(Some(&app.name));
        label.add_css_class("activities-app-widget-display-name");
        contents.append(&icon);
        contents.append(&label);
        button.set_child(Some(&contents));
        root.append(&button);
        let id = app.id.clone();
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.launch(&id, revision);
            }
        });
        root
    }
    fn rebuild(self: &Rc<Self>) {
        if self.closed.get() {
            return;
        }
        self.pending_rebuild.set(true);
        if self.rebuilding.replace(true) {
            return;
        }
        while self.pending_rebuild.replace(false) && !self.closed.get() {
            let revision = self.inventory_revision.get().wrapping_add(1);
            self.inventory_revision.set(revision);
            self.apps.borrow_mut().clear();
            while let Some(child) = self.results.first_child() {
                self.results.remove(&child);
            }
            for page in self.pages.take() {
                self.carousel.remove(&page);
            }
            let mut pages = Vec::new();
            for (index, app) in self.catalog.entries().into_iter().enumerate() {
                if self.closed.get() {
                    break;
                }
                let search_widget = self.app_widget(&app, revision);
                let child = gtk::FlowBoxChild::new();
                child.set_child(Some(&search_widget));
                self.apps.borrow_mut().insert(child.clone(), app.clone());
                self.results.insert(&child, -1);
                if index % 24 == 0 {
                    let page = gtk::Grid::new();
                    page.set_column_spacing(20);
                    page.set_row_spacing(20);
                    page.set_halign(gtk::Align::Center);
                    page.set_valign(gtk::Align::Start);
                    page.set_hexpand(true);
                    page.set_vexpand(true);
                    pages.push(page);
                }
                let slot = index % 24;
                pages.last().unwrap().attach(
                    &self.app_widget(&app, revision),
                    (slot % 7) as i32,
                    (slot / 7) as i32,
                    1,
                    1,
                );
            }
            if self.closed.get() {
                break;
            }
            for page in &pages {
                self.carousel.append(page);
            }
            self.pages.replace(pages);
            self.update_search();
        }
        self.rebuilding.set(false);
    }
    fn on_closed(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.request_revision
            .set(self.request_revision.get().wrapping_add(1));
        self.cancel_timer();
        self.transition.borrow_mut().take();
        if self.announced.replace(false) {
            if self.desired.replace(false) {
                self.publish(ActivitiesEvent::WillHide);
            }
            self.publish(ActivitiesEvent::Hidden);
        }
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.closed.set(true);
        self.cancel_timer();
        if let Some(handler) = self.catalog_handler.get_mut().take() {
            self.catalog.disconnect(handler);
        }
        for (source, handler) in self.handlers.get_mut().drain(..) {
            source.disconnect(handler);
        }
        self.results.unset_filter_func();
        self.visibility.close();
    }
}
fn navigation(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> Option<i32> {
    use gtk::gdk::{Key, ModifierType};
    if key == Key::Tab || key == Key::n && modifiers.contains(ModifierType::CONTROL_MASK) {
        Some(1)
    } else if key == Key::ISO_Left_Tab && modifiers.contains(ModifierType::SHIFT_MASK)
        || key == Key::p && modifiers.contains(ModifierType::CONTROL_MASK)
    {
        Some(-1)
    } else {
        None
    }
}
/// Internal controls documented by the shortcut sheet.
pub const NAVIGATION: &[(&str, &str)] = &[
    ("Type", "Search applications"),
    ("Tab / Ctrl+N", "Next result"),
    ("Shift+Tab / Ctrl+P", "Previous result"),
    ("Enter", "Launch selected result"),
    ("Escape", "Clear search"),
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_preserves_tab_and_control_keys_without_claiming_escape() {
        use gtk::gdk::{Key, ModifierType};
        assert_eq!(navigation(Key::Tab, ModifierType::empty()), Some(1));
        assert_eq!(
            navigation(Key::ISO_Left_Tab, ModifierType::SHIFT_MASK),
            Some(-1)
        );
        assert_eq!(navigation(Key::n, ModifierType::CONTROL_MASK), Some(1));
        assert_eq!(navigation(Key::p, ModifierType::CONTROL_MASK), Some(-1));
        assert_eq!(navigation(Key::n, ModifierType::empty()), None);
        assert_eq!(navigation(Key::Escape, ModifierType::empty()), None);
    }
}
