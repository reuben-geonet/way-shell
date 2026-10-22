//! GTK tray indicators driven by owned status-notifier and menu services.
use crate::{
    services::tray::{
        ItemCommand, ItemKey, TrayEvent, TrayItem, TrayService,
        menu::{MenuService, MenuState},
    },
    ui::tray_support::{load_theme_icon, menu_model, rgba_pixbuf},
};
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
    time::Duration,
};

/// Keep this controller alive while its widget belongs to a panel. Dropping the
/// final controller disconnects the service and makes retained widgets inert.
#[derive(Clone)]
pub struct TrayBar(Rc<Bar>);
struct Bar {
    widget: gtk::Box,
    list: gtk::Box,
    service: TrayService,
    handler: RefCell<Option<glib::SignalHandlerId>>,
    rows: RefCell<BTreeMap<String, Rc<Indicator>>>,
}
impl TrayBar {
    pub fn new(service: TrayService) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        widget.set_widget_name("panel-indicator-bar");
        let list = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        list.add_css_class("panel-indicator-bar-list");
        widget.append(&list);
        let bar = Rc::new(Bar {
            widget,
            list,
            service,
            handler: RefCell::new(None),
            rows: RefCell::new(BTreeMap::new()),
        });
        let weak = Rc::downgrade(&bar);
        let handler = bar.service.connect_local("event", false, move |values| {
            if let Some(bar) = weak.upgrade() {
                bar.apply(values[1].get::<TrayEvent>().expect("typed tray event"));
            }
            None
        });
        bar.handler.replace(Some(handler));
        for item in bar.service.state().items {
            bar.apply(TrayEvent::Added(item));
        }
        Self(bar)
    }
    pub fn widget(&self) -> &gtk::Box {
        &self.0.widget
    }
}
impl Bar {
    fn apply(&self, event: TrayEvent) {
        match event {
            TrayEvent::Added(item) | TrayEvent::Changed(item) => {
                let key = item.key.registration();
                let current = self.rows.borrow().get(&key).cloned();
                if let Some(row) = current {
                    row.update(item);
                } else {
                    let row = Indicator::new(self.service.clone(), item);
                    self.rows.borrow_mut().insert(key, row.clone());
                    self.list.append(&row.widget);
                    row.start();
                }
            }
            TrayEvent::Removed(item) => {
                let row = self.rows.borrow_mut().remove(&item.key.registration());
                if let Some(row) = row {
                    row.stop();
                    self.list.remove(&row.widget);
                }
            }
        }
    }
}
impl Drop for Bar {
    fn drop(&mut self) {
        if let Some(handler) = self.handler.get_mut().take() {
            self.service.disconnect(handler);
        }
        for row in std::mem::take(self.rows.get_mut()).into_values() {
            row.stop();
            self.list.remove(&row.widget);
        }
    }
}
struct MenuBinding {
    service: MenuService,
    handler: Option<glib::SignalHandlerId>,
    actions: gio::SimpleActionGroup,
}
impl Drop for MenuBinding {
    fn drop(&mut self) {
        if let Some(handler) = self.handler.take() {
            self.service.disconnect(handler);
        }
        for name in ["item-clicked", "about-to-show"] {
            self.actions
                .lookup_action(name)
                .unwrap()
                .downcast::<gio::SimpleAction>()
                .unwrap()
                .set_enabled(false);
        }
        self.service.stop();
    }
}
struct Indicator {
    widget: gtk::Box,
    button: gtk::Button,
    icon: gtk::Image,
    service: TrayService,
    key: ItemKey,
    item: RefCell<TrayItem>,
    active: Cell<bool>,
    click_handler: RefCell<Option<glib::SignalHandlerId>>,
    popup: RefCell<Option<gtk::PopoverMenu>>,
    menu: RefCell<Option<MenuBinding>>,
    menu_revision: Cell<u64>,
    icon_revision: Cell<u64>,
    icon_task: RefCell<Option<glib::JoinHandle<()>>>,
    displayed: Cell<bool>,
}
impl Indicator {
    fn new(service: TrayService, item: TrayItem) -> Rc<Self> {
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        widget.set_widget_name("panel-indicator-bar-widget");
        let inner = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        inner.add_css_class("panel-indicator-bar-widget-box");
        let icon = gtk::Image::new();
        icon.set_pixel_size(16);
        icon.set_size_request(16, 16);
        let button = gtk::Button::new();
        button.set_child(Some(&icon));
        inner.append(&button);
        widget.append(&inner);
        let row = Rc::new(Self {
            widget,
            button,
            icon,
            service,
            key: item.key.clone(),
            item: RefCell::new(item),
            active: Cell::new(true),
            click_handler: RefCell::new(None),
            popup: RefCell::new(None),
            menu: RefCell::new(None),
            menu_revision: Cell::new(0),
            icon_revision: Cell::new(0),
            icon_task: RefCell::new(None),
            displayed: Cell::new(false),
        });
        let weak = Rc::downgrade(&row);
        let handler = row.button.connect_clicked(move |_| {
            if let Some(row) = weak.upgrade().filter(|row| row.active.get()) {
                row.click();
            }
        });
        row.click_handler.replace(Some(handler));
        row
    }
    fn start(self: &Rc<Self>) {
        self.update_icon();
        self.start_menu();
    }
    fn update(self: &Rc<Self>, item: TrayItem) {
        if !self.active.get() {
            return;
        }
        let old = self.item.replace(item.clone());
        if old.icon != item.icon || old.icon_theme_path != item.icon_theme_path {
            self.update_icon();
        }
        if old.menu_path != item.menu_path {
            self.stop_menu();
        }
        if self.active.get() {
            self.start_menu();
        }
    }
    fn click(&self) {
        let popup = self.popup.borrow().clone();
        if let Some(popup) = popup {
            popup.popup();
            let service = self
                .menu
                .borrow()
                .as_ref()
                .map(|binding| binding.service.clone());
            if self.active.get()
                && let Some(service) = service
            {
                service.about_to_show(0, report_menu_reply);
            }
        } else {
            self.service.command(
                &self.key,
                ItemCommand::Activate { x: 0, y: 0 },
                report_reply,
            );
        }
    }
    fn update_icon(self: &Rc<Self>) {
        self.cancel_icon();
        if !self.active.get() {
            return;
        }
        let item = self.item.borrow().clone();
        let asynchronous = !item.icon.name.is_empty() && !item.icon_theme_path.is_empty();
        if !asynchronous {
            if !self.show_supplied_icon(&item) {
                self.missing_icon();
            }
            return;
        }
        if !self.displayed.get() {
            self.show_supplied_icon(&item);
        }
        if !self.active.get() {
            return;
        }
        let revision = self.icon_revision.get();
        let weak = Rc::downgrade(self);
        let task = glib::MainContext::ref_thread_default().spawn_local(async move {
            let result = glib::future_with_timeout(
                Duration::from_secs(2),
                load_theme_icon(&item.icon_theme_path, &item.icon.name),
            )
            .await;
            if let Some(row) = weak.upgrade() {
                row.finish_icon(
                    revision,
                    result.unwrap_or_else(|_| {
                        Err(glib::Error::new(
                            gio::IOErrorEnum::TimedOut,
                            "Tray icon loading timed out",
                        ))
                    }),
                );
            }
        });
        self.icon_task.replace(Some(task));
    }
    fn show_supplied_icon(&self, item: &TrayItem) -> bool {
        if let Some(image) = item.icon.pixmap.as_ref().and_then(rgba_pixbuf) {
            self.icon
                .set_paintable(Some(&gtk::gdk::Texture::for_pixbuf(&image)));
        } else if !item.icon.name.is_empty()
            && gtk::IconTheme::for_display(&self.icon.display()).has_icon(&item.icon.name)
        {
            self.icon.set_icon_name(Some(&item.icon.name));
        } else {
            return false;
        }
        self.displayed.set(true);
        true
    }
    fn missing_icon(&self) {
        self.icon.set_icon_name(Some("image-missing"));
        self.displayed.set(false);
    }
    fn finish_icon(&self, revision: u64, result: Result<gtk::gdk_pixbuf::Pixbuf, glib::Error>) {
        if !self.active.get() || self.icon_revision.get() != revision {
            return;
        }
        self.icon_task.borrow_mut().take();
        match result {
            Ok(image) => {
                self.icon
                    .set_paintable(Some(&gtk::gdk::Texture::for_pixbuf(&image)));
                self.displayed.set(true);
            }
            Err(error) => {
                let item = self.item.borrow().clone();
                glib::g_message!(
                    "way-shell",
                    "Tray icon {} could not be loaded: {error}",
                    item.icon.name
                );
                if !self.show_supplied_icon(&item) && !self.displayed.get() {
                    self.missing_icon();
                }
            }
        }
    }
    fn cancel_icon(&self) {
        self.icon_revision
            .set(self.icon_revision.get().wrapping_add(1));
        if let Some(task) = self.icon_task.borrow_mut().take() {
            task.abort();
        }
    }
    fn start_menu(self: &Rc<Self>) {
        if !self.active.get() || self.menu.borrow().is_some() {
            return;
        }
        let service = match self.service.menu(&self.key) {
            Ok(Some(service)) => service,
            Ok(None) => return,
            Err(error) => {
                glib::g_message!(
                    "way-shell",
                    "Cannot initialize tray menu {}: {error}",
                    self.key.registration()
                );
                return;
            }
        };
        let revision = self.menu_revision.get();
        let weak = Rc::downgrade(self);
        let handler = service.connect_local("snapshot", false, move |values| {
            if let Some(row) = weak.upgrade().filter(|row| row.menu_current(revision)) {
                row.apply_menu(
                    values[1].get::<MenuState>().expect("typed menu snapshot"),
                    revision,
                );
            }
            None
        });
        let actions = self.menu_actions(&service, revision);
        self.menu.replace(Some(MenuBinding {
            service: service.clone(),
            handler: Some(handler),
            actions,
        }));
        self.apply_menu(service.state(), revision);
    }
    fn menu_current(&self, revision: u64) -> bool {
        self.active.get() && self.menu_revision.get() == revision
    }
    fn menu_actions(
        self: &Rc<Self>,
        service: &MenuService,
        revision: u64,
    ) -> gio::SimpleActionGroup {
        let group = gio::SimpleActionGroup::new();
        for name in ["item-clicked", "about-to-show"] {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::new("(si)").unwrap()));
            action.set_enabled(false);
            let weak = Rc::downgrade(self);
            let weak_service = service.downgrade();
            action.connect_activate(move |_, parameter| {
                let Some((key, id)) = parameter.and_then(|value| value.get::<(String, i32)>())
                else {
                    return;
                };
                let Some(row) = weak
                    .upgrade()
                    .filter(|row| row.menu_current(revision) && row.key.registration() == key)
                else {
                    return;
                };
                let Some(service) = weak_service.upgrade() else {
                    return;
                };
                if !row.active.get() {
                    return;
                }
                if name == "item-clicked" {
                    service.activate(id, (glib::real_time() / 1_000_000) as u32, report_reply);
                } else {
                    service.about_to_show(id, report_menu_reply);
                }
            });
            group.add_action(&action);
        }
        group
    }
    fn apply_menu(&self, state: MenuState, revision: u64) {
        if !self.menu_current(revision) {
            return;
        }
        let actions = self
            .menu
            .borrow()
            .as_ref()
            .map(|binding| binding.actions.clone());
        let Some(actions) = actions else {
            return;
        };
        for name in ["item-clicked", "about-to-show"] {
            if !self.menu_current(revision) {
                return;
            }
            actions
                .lookup_action(name)
                .unwrap()
                .downcast::<gio::SimpleAction>()
                .unwrap()
                .set_enabled(state.available);
        }
        if !self.menu_current(revision) {
            return;
        }
        let Some(root) = state.root else {
            self.clear_popup();
            return;
        };
        let model = menu_model(&root, &self.key.registration());
        let current = self.popup.borrow().clone();
        let popup = if let Some(popup) = current {
            popup
        } else {
            let popup = gtk::PopoverMenu::from_model_full(&model, gtk::PopoverMenuFlags::NESTED);
            self.popup.replace(Some(popup.clone()));
            popup.set_parent(&self.button);
            popup
        };
        if !self.menu_current(revision) {
            return;
        }
        popup.insert_action_group("sni", Some(&actions));
        if !self.menu_current(revision) {
            return;
        }
        popup.set_menu_model(Some(&model));
    }
    fn clear_popup(&self) {
        let popup = self.popup.borrow_mut().take();
        if let Some(popup) = popup {
            popup.insert_action_group("sni", gio::ActionGroup::NONE);
            popup.popdown();
            if popup.parent().is_some() {
                popup.unparent();
            }
        }
    }
    fn stop_menu(&self) {
        self.menu_revision
            .set(self.menu_revision.get().wrapping_add(1));
        let menu = self.menu.borrow_mut().take();
        drop(menu);
        self.clear_popup();
    }
    fn stop(&self) {
        if !self.active.replace(false) {
            return;
        }
        if let Some(handler) = self.click_handler.borrow_mut().take() {
            self.button.disconnect(handler);
        }
        self.cancel_icon();
        self.stop_menu();
    }
}
impl Drop for Indicator {
    fn drop(&mut self) {
        self.stop();
    }
}
fn report_reply(result: Result<(), glib::Error>) {
    if let Err(error) = result {
        glib::g_message!("way-shell", "Tray operation failed: {error}");
    }
}
fn report_menu_reply(result: Result<bool, glib::Error>) {
    if let Err(error) = result {
        glib::g_message!("way-shell", "Tray menu operation failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::tray::{Icon, RgbaImage, Tooltip};
    fn item(name: &str, directory: &str, pixmap: Option<RgbaImage>) -> TrayItem {
        TrayItem {
            key: ItemKey {
                owner: ":1.2".into(),
                path: "/Item".into(),
            },
            category: "ApplicationStatus".into(),
            id: "fixture".into(),
            title: "Fixture".into(),
            status: "Active".into(),
            window_id: 0,
            icon_theme_path: directory.into(),
            icon: Icon {
                name: name.into(),
                pixmap,
            },
            overlay: Icon::default(),
            attention: Icon::default(),
            attention_movie_name: String::new(),
            tooltip: Tooltip::default(),
            item_is_menu: false,
            menu_path: None,
            label: String::new(),
        }
    }
    #[test]
    #[ignore = "requires an isolated Wayland compositor"]
    fn tray_icons_reserve_space_retain_replacements_and_reject_stale_results() {
        crate::ui::init_test_display();
        let context = glib::MainContext::default();
        let directory = std::env::temp_dir().join(format!(
            "way-shell-tray-presentation-{}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("fixture.png"),
            include_bytes!("../../../../../tests/fixtures/tray-icon.png"),
        )
        .unwrap();
        let directory_name = directory.to_str().unwrap();
        let service: TrayService = glib::Object::new();
        let row = Indicator::new(service.clone(), item("fixture.png", directory_name, None));
        row.update_icon();
        assert!(row.icon.paintable().is_none());
        assert!(row.icon.icon_name().is_none());
        let width = row.icon.measure(gtk::Orientation::Horizontal, -1);
        let wait = |row: &Rc<Indicator>| {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while row.icon_task.borrow().is_some() {
                assert!(std::time::Instant::now() < deadline);
                context.block_on(glib::timeout_future(Duration::from_millis(5)));
            }
        };
        wait(&row);
        assert!(row.displayed.get());
        let displayed = row.icon.paintable();
        assert_eq!(row.icon.measure(gtk::Orientation::Horizontal, -1), width);
        row.item.replace(item("missing.png", directory_name, None));
        row.update_icon();
        let old_revision = row.icon_revision.get();
        assert_eq!(row.icon.paintable(), displayed);
        row.item.replace(item("fixture.png", directory_name, None));
        row.update_icon();
        let pixel = RgbaImage {
            width: 1,
            height: 1,
            pixels: vec![1, 2, 3, 255],
        };
        let decoded = rgba_pixbuf(&pixel).unwrap();
        row.finish_icon(old_revision, Ok(decoded));
        assert_eq!(row.icon.paintable(), displayed);
        wait(&row);
        assert!(row.displayed.get());
        assert_ne!(row.icon.paintable(), displayed);
        let displayed = row.icon.paintable();
        row.item.replace(item("missing.png", directory_name, None));
        row.update_icon();
        assert_eq!(row.icon.paintable(), displayed);
        wait(&row);
        assert_eq!(row.icon.paintable(), displayed);
        let first = Indicator::new(service.clone(), item("missing.png", directory_name, None));
        first.update_icon();
        assert!(first.icon.icon_name().is_none());
        wait(&first);
        assert_eq!(first.icon.icon_name().as_deref(), Some("image-missing"));
        let immediate = Indicator::new(
            service.clone(),
            item("fixture.png", directory_name, Some(pixel)),
        );
        immediate.update_icon();
        assert!(immediate.icon.paintable().is_some());
        immediate.stop();
        let themed = Indicator::new(service.clone(), item("folder-symbolic", "", None));
        themed.update_icon();
        assert!(themed.displayed.get());
        assert_eq!(themed.icon.icon_name().as_deref(), Some("folder-symbolic"));
        let timeout = Indicator::new(service, item("fixture.png", directory_name, None));
        timeout.update_icon();
        assert!(timeout.icon.icon_name().is_none());
        timeout.cancel_icon();
        timeout.finish_icon(
            timeout.icon_revision.get(),
            Err(glib::Error::new(
                gio::IOErrorEnum::TimedOut,
                "fixture timeout",
            )),
        );
        assert_eq!(timeout.icon.icon_name().as_deref(), Some("image-missing"));
        row.stop();
        first.stop();
        themed.stop();
        timeout.stop();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
