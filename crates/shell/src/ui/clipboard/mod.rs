//! Copyous-style clipboard cards on a monitor-bound layer surface.
mod actions;
mod cards;
#[cfg(test)]
mod live_tests;
mod preferences;
use super::{
    progress::Progress,
    window::{LayerWindow, UnderlaySet, WindowRole},
};
use crate::services::{
    clipboard::ClipboardService, shortcuts::ShortcutsService, wayland::Toplevel, wm::WindowManager,
};
use gtk4::{gdk, gio};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use libadwaita::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use way_shell_clipboard::{Entry, Filter, Kind, TAGS};
type VisibilityCallback = Rc<dyn Fn(bool)>;

pub struct ClipboardDialog {
    pub service: Rc<ClipboardService>,
    manager: WindowManager,
    shortcuts: Rc<ShortcutsService>,
    surface: LayerWindow,
    underlays: UnderlaySet,
    root: gtk4::Fixed,
    panel: gtk4::Box,
    header: gtk4::CenterBox,
    footer: gtk4::Box,
    controls: gtk4::Box,
    clear: gtk4::Button,
    search_box: gtk4::Box,
    search: gtk4::SearchEntry,
    pinned: gtk4::ToggleButton,
    filter_menu: gtk4::MenuButton,
    filter_icon: gtk4::Image,
    kind: gtk4::SingleSelection,
    tag: gtk4::SingleSelection,
    model: gtk4::StringList,
    selection: gtk4::SingleSelection,
    list: gtk4::ListView,
    scroller: gtk4::ScrolledWindow,
    empty: gtk4::Box,
    empty_icon: gtk4::Image,
    empty_title: gtk4::Label,
    status: gtk4::Label,
    spinner: gtk4::Spinner,
    retry: gtk4::Button,
    progress: Progress,
    visible: Cell<bool>,
    stopped: Cell<bool>,
    subscription: Cell<Option<u64>>,
    monitor: RefCell<Option<(gdk::Monitor, glib::SignalHandlerId)>>,
    callbacks: RefCell<Vec<VisibilityCallback>>,
    target: RefCell<Option<Toplevel>>,
    generation: Cell<u64>,
    pending_paste: RefCell<Option<glib::JoinHandle<()>>>,
    processes: RefCell<Vec<gio::Subprocess>>,
    temporary_images: RefCell<Vec<std::path::PathBuf>>,
    preview_cancel: RefCell<Vec<Arc<AtomicBool>>>,
    preferences: RefCell<Option<libadwaita::PreferencesWindow>>,
    style: gtk4::CssProvider,
    style_manager: libadwaita::StyleManager,
    style_signature: RefCell<String>,
    style_handler: RefCell<Option<glib::SignalHandlerId>>,
    last_revision: Cell<u64>,
    pointer_pending: Cell<bool>,
    pointer: Cell<Option<(f64, f64)>>,
}
impl ClipboardDialog {
    pub fn new(
        service: Rc<ClipboardService>,
        manager: WindowManager,
        shortcuts: Rc<ShortcutsService>,
    ) -> Result<Rc<Self>, String> {
        crate::resources::get();
        gtk4::IconTheme::for_display(
            &gdk::Display::default().ok_or("No GTK display is available")?,
        )
        .add_resource_path("/org/ldelossa/way-shell/data/clipboard/icons");
        let surface = LayerWindow::new(WindowRole::Clipboard, None)?;
        surface.window().set_title(Some("Clipboard history"));
        surface.window().add_css_class("clipboard-surface");
        let root = gtk4::Fixed::new();
        root.set_hexpand(true);
        root.set_vexpand(true);
        let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        panel.add_css_class("clipboard-dialog");
        root.put(&panel, 0.0, 0.0);
        surface.window().set_content(Some(&root));
        let header = gtk4::CenterBox::new();
        header.add_css_class("clipboard-header");
        let controls = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        controls.set_valign(gtk4::Align::Center);
        let settings = icon_button("way-shell-settings-symbolic", "Clipboard preferences");
        let incognito = gtk4::ToggleButton::new();
        incognito.set_icon_name("way-shell-incognito-symbolic");
        incognito.set_tooltip_text(Some("Incognito: pause clipboard history"));
        incognito.add_css_class("circular");
        controls.append(&settings);
        controls.append(&incognito);
        header.set_start_widget(Some(&controls));
        let search_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        search_box.add_css_class("clipboard-search");
        search_box.set_hexpand(false);
        search_box.set_halign(gtk4::Align::Center);
        search_box.set_valign(gtk4::Align::Center);
        let kind_names: Vec<_> = std::iter::once("All types")
            .chain(Kind::ALL.iter().map(|kind| kind.name()))
            .collect();
        let kind = gtk4::SingleSelection::new(Some(gtk4::StringList::new(&kind_names)));
        let tag_names: Vec<_> = std::iter::once("All tags").chain(TAGS).collect();
        let tag = gtk4::SingleSelection::new(Some(gtk4::StringList::new(&tag_names)));
        let filter_menu = gtk4::MenuButton::new();
        let filter_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
        let filter_icon = gtk4::Image::from_icon_name("system-search-symbolic");
        filter_content.append(&filter_icon);
        let arrow = gtk4::Image::from_icon_name("pan-down-symbolic");
        arrow.set_pixel_size(8);
        filter_content.append(&arrow);
        filter_menu.set_child(Some(&filter_content));
        filter_menu.add_css_class("flat");
        filter_menu.set_tooltip_text(Some("Filter clipboard history by type or tag"));
        let filters = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        filters.set_margin_top(10);
        filters.set_margin_bottom(10);
        filters.set_margin_start(10);
        filters.set_margin_end(10);
        filters.append(&tag_picker(&tag));
        filters.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
        let type_factory = gtk4::SignalListItemFactory::new();
        type_factory.connect_bind(|_, object| {
            let Some(item) = object.downcast_ref::<gtk4::ListItem>() else {
                return;
            };
            let Some(name) = item.item().and_downcast::<gtk4::StringObject>() else {
                return;
            };
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            row.append(&gtk4::Image::from_icon_name("object-select-symbolic"));
            let label = gtk4::Label::new(Some(&name.string()));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            row.append(&label);
            item.set_child(Some(&row));
        });
        let types = gtk4::ListView::new(Some(kind.clone()), Some(type_factory));
        types.add_css_class("clipboard-filter-types");
        types.set_single_click_activate(true);
        filters.append(&types);
        let popover = gtk4::Popover::new();
        popover.add_css_class("clipboard-popover");
        popover.set_child(Some(&filters));
        let weak_popover = popover.downgrade();
        types.connect_activate(move |_, _| {
            if let Some(popover) = weak_popover.upgrade() {
                popover.popdown();
            }
        });
        let weak_popover = popover.downgrade();
        tag.connect_selected_notify(move |_| {
            if let Some(popover) = weak_popover.upgrade() {
                popover.popdown();
            }
        });
        filter_menu.set_popover(Some(&popover));
        let search = gtk4::SearchEntry::new();
        search.set_placeholder_text(Some("Type to search"));
        search.set_hexpand(true);
        search.set_width_request(100);
        let pinned = gtk4::ToggleButton::new();
        pinned.set_icon_name("way-shell-pin-symbolic");
        pinned.set_tooltip_text(Some("Search pinned items"));
        pinned.add_css_class("clipboard-pinned");
        pinned.set_valign(gtk4::Align::Center);
        search_box.append(&filter_menu);
        search_box.append(&search);
        search_box.append(&pinned);
        header.set_center_widget(Some(&search_box));
        let clear = gtk4::Button::with_label("Clear");
        clear.add_css_class("pill");
        header.set_end_widget(Some(&clear));
        panel.append(&header);
        let model = gtk4::StringList::new(&[]);
        let selection = gtk4::SingleSelection::new(Some(model.clone()));
        let list =
            gtk4::ListView::new(Some(selection.clone()), None::<gtk4::SignalListItemFactory>);
        list.add_css_class("clipboard-items");
        list.set_orientation(gtk4::Orientation::Horizontal);
        let scroller = gtk4::ScrolledWindow::new();
        scroller.set_overlay_scrolling(false);
        scroller.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Never);
        scroller.set_hexpand(true);
        scroller.set_vexpand(true);
        scroller.set_child(Some(&list));
        for scrollbar in [scroller.hscrollbar(), scroller.vscrollbar()] {
            let list = list.downgrade();
            let scroller = scroller.downgrade();
            scrollbar.connect_visible_notify(move |_| {
                if let (Some(list), Some(scroller)) = (list.upgrade(), scroller.upgrade()) {
                    if scroller.hscrollbar().is_visible() || scroller.vscrollbar().is_visible() {
                        list.add_css_class("clipboard-scrollbars");
                    } else {
                        list.remove_css_class("clipboard-scrollbars");
                    }
                }
            });
        }
        let content = gtk4::Overlay::new();
        content.set_hexpand(true);
        content.set_vexpand(true);
        content.set_child(Some(&scroller));
        let empty = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
        empty.add_css_class("clipboard-empty");
        empty.set_halign(gtk4::Align::Center);
        empty.set_valign(gtk4::Align::Center);
        empty.set_can_target(false);
        empty.set_visible(false);
        let empty_icon = gtk4::Image::from_icon_name("way-shell-clipboard-symbolic");
        empty_icon.set_pixel_size(64);
        let empty_title = gtk4::Label::new(Some("Clipboard is Empty"));
        empty_title.add_css_class("clipboard-empty-title");
        empty.append(&empty_icon);
        empty.append(&empty_title);
        content.add_overlay(&empty);
        panel.append(&content);
        let status_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        status_box.add_css_class("clipboard-status");
        status_box.set_margin_start(15);
        status_box.set_margin_end(15);
        let spinner = gtk4::Spinner::new();
        spinner.set_size_request(16, 16);
        spinner.set_opacity(0.0);
        let status = gtk4::Label::new(None);
        status.set_xalign(0.0);
        status.set_hexpand(true);
        status.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        status_box.append(&spinner);
        status_box.append(&status);
        let retry = icon_button("view-refresh-symbolic", "Retry opening clipboard history");
        retry.set_opacity(0.0);
        retry.set_sensitive(false);
        status_box.append(&retry);
        status_box.set_valign(gtk4::Align::End);
        status_box.set_can_target(false);
        content.add_overlay(&status_box);
        let footer = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        footer.add_css_class("clipboard-footer");
        panel.append(&footer);
        let underlays = UnderlaySet::new(WindowRole::PanelPopupUnderlay)?;
        let style = gtk4::CssProvider::new();
        gtk4::style_context_add_provider_for_display(
            &gtk4::prelude::WidgetExt::display(surface.window()),
            &style,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        let style_manager = libadwaita::StyleManager::for_display(
            &gtk4::prelude::WidgetExt::display(surface.window()),
        );
        let this = Rc::new_cyclic(|weak: &std::rc::Weak<Self>| Self {
            service,
            manager,
            shortcuts,
            surface,
            underlays,
            root,
            panel,
            header,
            footer,
            controls,
            clear,
            search_box,
            search,
            pinned,
            filter_menu,
            filter_icon,
            kind,
            tag,
            model,
            selection,
            list,
            scroller,
            empty,
            empty_icon,
            empty_title,
            status,
            spinner,
            retry,
            progress: Progress::new({
                let weak = weak.clone();
                move |visible| {
                    if let Some(this) = weak.upgrade() {
                        this.spinner.set_opacity(if visible { 1.0 } else { 0.0 });
                        this.spinner.set_spinning(visible);
                    }
                }
            }),
            visible: Cell::new(false),
            stopped: Cell::new(false),
            subscription: Cell::new(None),
            monitor: RefCell::new(None),
            callbacks: RefCell::new(Vec::new()),
            target: RefCell::new(None),
            generation: Cell::new(0),
            pending_paste: RefCell::new(None),
            processes: RefCell::new(Vec::new()),
            temporary_images: RefCell::new(Vec::new()),
            preview_cancel: RefCell::new(Vec::new()),
            preferences: RefCell::new(None),
            style,
            style_manager,
            style_signature: RefCell::new(String::new()),
            style_handler: RefCell::new(None),
            last_revision: Cell::new(u64::MAX),
            pointer_pending: Cell::new(false),
            pointer: Cell::new(None),
        });
        let weak = Rc::downgrade(&this);
        let id = this.service.on_changed(move || {
            if let Some(this) = weak.upgrade() {
                incognito.set_active(this.service.settings.boolean("incognito"));
                if this.visible.get() {
                    this.render();
                }
            }
        });
        this.subscription.set(Some(id));
        let weak = Rc::downgrade(&this);
        this.retry.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.service.retry();
            }
        });
        // The state switch remains immediate and independent of presentation timers.
        let incognito = this
            .controls
            .last_child()
            .and_downcast::<gtk4::ToggleButton>()
            .unwrap();
        incognito.set_active(this.service.settings.boolean("incognito"));
        let weak = Rc::downgrade(&this);
        incognito.connect_toggled(move |button| {
            if let Some(this) = weak.upgrade() {
                let _ = this.service.incognito(button.is_active());
            }
        });
        let weak = Rc::downgrade(&this);
        settings.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.open_preferences();
            }
        });
        let weak = Rc::downgrade(&this);
        this.clear.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.confirm_clear();
            }
        });
        let weak = Rc::downgrade(&this);
        this.search.connect_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.place(None);
                this.filter();
            }
        });
        let weak = Rc::downgrade(&this);
        this.pinned.connect_toggled(move |_| {
            if let Some(this) = weak.upgrade() {
                this.filter();
            }
        });
        for dropdown in [&this.kind, &this.tag] {
            let weak = Rc::downgrade(&this);
            dropdown.connect_selected_notify(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.filter();
                }
            });
        }
        let factory = gtk4::SignalListItemFactory::new();
        let weak = Rc::downgrade(&this);
        factory.connect_bind(move |_, object| {
            if let Some(this) = weak.upgrade()
                && let Some(item) = object.downcast_ref::<gtk4::ListItem>()
                && let Some(id) = item
                    .item()
                    .and_downcast::<gtk4::StringObject>()
                    .and_then(|value| value.string().parse::<i64>().ok())
                && let Some(entry) = this.service.entry(id)
            {
                item.set_child(Some(&this.card(&entry)));
            }
        });
        factory.connect_unbind(|_, object| {
            if let Some(item) = object.downcast_ref::<gtk4::ListItem>() {
                item.set_child(None::<&gtk4::Widget>);
            }
        });
        this.list.set_factory(Some(&factory));
        let list = this.list.downgrade();
        this.selection.connect_selected_notify(move |selection| {
            if selection.selected() != gtk4::INVALID_LIST_POSITION
                && let Some(list) = list.upgrade()
            {
                let _ = list.activate_action(
                    "list.scroll-to-item",
                    Some(&selection.selected().to_variant()),
                );
            }
        });
        let weak = Rc::downgrade(&this);
        this.list.connect_activate(move |_, position| {
            if let Some(this) = weak.upgrade()
                && let Some(id) = this.model.string(position).and_then(|id| id.parse().ok())
            {
                this.activate(id, this.service.settings.boolean("swap-copy-shortcut"));
            }
        });
        let weak = Rc::downgrade(&this);
        this.underlays.on_dismiss(move || {
            if let Some(this) = weak.upgrade() {
                this.hide();
            }
        });
        let weak = Rc::downgrade(&this);
        this.underlays.on_pointer(move |monitor, x, y| {
            if let Some(this) = weak.upgrade().filter(|this| this.visible.get())
                && this.pointer_pending.replace(false)
            {
                this.set_monitor(monitor);
                this.place(Some((x, y)));
            }
        });
        let weak = Rc::downgrade(&this);
        this.surface.on_closed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.close();
            }
        });
        let click = gtk4::GestureClick::new();
        let weak = Rc::downgrade(&this);
        click.connect_released(move |_, _, x, y| {
            if let Some(this) = weak.upgrade()
                && this
                    .root
                    .pick(x, y, gtk4::PickFlags::DEFAULT)
                    .is_none_or(|picked| !picked.is_ancestor(&this.panel) && picked != this.panel)
            {
                this.hide();
            }
        });
        this.root.add_controller(click);
        let motion = gtk4::EventControllerMotion::new();
        let weak = Rc::downgrade(&this);
        motion.connect_enter(move |_, x, y| {
            if let Some(this) = weak.upgrade()
                && this.pointer_pending.replace(false)
            {
                this.place(Some((x, y)));
            }
        });
        this.root.add_controller(motion);
        let scroll = gtk4::EventControllerScroll::new(
            gtk4::EventControllerScrollFlags::BOTH_AXES
                | gtk4::EventControllerScrollFlags::DISCRETE,
        );
        let weak = Rc::downgrade(&this);
        scroll.connect_scroll(move |controller, dx, dy| {
            if let Some(this) = weak.upgrade() {
                let reverse = dx + dy < 0.0;
                let ctrl = controller
                    .current_event_state()
                    .contains(gdk::ModifierType::CONTROL_MASK);
                if ctrl ^ this.service.settings.boolean("swap-scroll-shortcut") {
                    this.tag
                        .set_selected(cycle(this.tag.selected(), 10, reverse));
                } else {
                    this.kind
                        .set_selected(cycle(this.kind.selected(), 9, reverse));
                }
            }
            glib::Propagation::Stop
        });
        this.search_box.add_controller(scroll);
        let scroll = gtk4::EventControllerScroll::new(
            gtk4::EventControllerScrollFlags::BOTH_AXES
                | gtk4::EventControllerScrollFlags::DISCRETE,
        );
        let weak = Rc::downgrade(&this);
        scroll.connect_scroll(move |_, dx, dy| {
            if let Some(this) = weak.upgrade() {
                let vertical = this.list.orientation() == gtk4::Orientation::Vertical;
                let adjustment = if vertical {
                    this.scroller.vadjustment()
                } else {
                    this.scroller.hadjustment()
                };
                let step = this.service.settings.int(if vertical {
                    "item-height"
                } else {
                    "item-width"
                }) + 10;
                adjustment.set_value((adjustment.value() + (dx + dy) * f64::from(step)).clamp(
                    adjustment.lower(),
                    (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()),
                ));
            }
            glib::Propagation::Stop
        });
        this.list.add_controller(scroll);
        this.install_keys();
        let weak = Rc::downgrade(&this);
        let handler = this.style_manager.connect_dark_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.last_revision.set(u64::MAX);
                this.render();
            }
        });
        this.style_handler.replace(Some(handler));
        Ok(this)
    }
    pub fn preferences(self: &Rc<Self>) {
        self.open_preferences();
    }
    pub fn on_visible(&self, callback: impl Fn(bool) + 'static) {
        self.callbacks.borrow_mut().push(Rc::new(callback));
    }
    pub fn is_visible(&self) -> bool {
        self.visible.get()
    }
    pub fn window(&self) -> &libadwaita::Window {
        self.surface.window()
    }
    pub fn toggle(self: &Rc<Self>, monitor: Option<&gdk::Monitor>) -> Result<(), String> {
        if self.visible.get() {
            self.hide();
            Ok(())
        } else {
            self.show(monitor)
        }
    }
    pub fn open(self: &Rc<Self>) -> Result<(), String> {
        if self.visible.get()
            && self
                .service
                .settings
                .string("open-clipboard-dialog-behavior")
                == "open-or-select-next"
        {
            self.navigate(1);
            Ok(())
        } else {
            self.toggle(None)
        }
    }
    pub fn show(self: &Rc<Self>, monitor: Option<&gdk::Monitor>) -> Result<(), String> {
        if self.stopped.get() {
            return Err("Clipboard surface has stopped".into());
        }
        if self.visible.get() {
            return Ok(());
        }
        self.cancel_paste();
        self.generation.set(self.generation.get().wrapping_add(1));
        self.target.replace(self.service.wayland.focused_toplevel());
        let display = gtk4::prelude::WidgetExt::display(self.window());
        let monitors = display.monitors();
        let output = self
            .manager
            .workspaces()
            .into_iter()
            .find(|workspace| workspace.focused)
            .and_then(|workspace| workspace.output);
        let monitor = monitor
            .cloned()
            .or_else(|| {
                (0..monitors.n_items())
                    .filter_map(|i| monitors.item(i).and_downcast::<gdk::Monitor>())
                    .find(|monitor| monitor.connector().as_deref() == output.as_deref())
            })
            .or_else(|| monitors.item(0).and_downcast::<gdk::Monitor>())
            .ok_or("No display outputs are available")?;
        self.set_monitor(&monitor);
        self.visible.set(true);
        for callback in self.callbacks.borrow().clone() {
            callback(true);
        }
        if !self.visible.get() {
            return Ok(());
        }
        if !self.service.settings.boolean("remember-search") {
            self.search.set_text("");
            self.pinned.set_active(false);
            self.kind.set_selected(0);
            self.tag.set_selected(0);
        }
        self.pointer_pending
            .set(self.service.settings.boolean("show-at-pointer"));
        self.pointer.set(None);
        self.place(None);
        self.last_revision.set(u64::MAX);
        self.render();
        if !self.service.settings.boolean("remember-search") {
            self.selection.set_selected(0);
        }
        self.service.prune();
        self.underlays.present();
        self.window().set_keyboard_mode(KeyboardMode::Exclusive);
        self.surface.present();
        self.list.grab_focus();
        Ok(())
    }
    fn set_monitor(self: &Rc<Self>, monitor: &gdk::Monitor) {
        if let Some((old, handler)) = self.monitor.borrow_mut().take() {
            old.disconnect(handler);
        }
        let weak = Rc::downgrade(self);
        let handler = monitor.connect_invalidate(move |_| {
            if let Some(this) = weak.upgrade() {
                this.hide();
            }
        });
        self.monitor.replace(Some((monitor.clone(), handler)));
        self.window().set_monitor(Some(monitor));
        let geometry = monitor.geometry();
        self.window()
            .set_default_size(geometry.width(), geometry.height());
    }
    pub fn hide(&self) {
        if !self.visible.replace(false) {
            return;
        }
        self.progress.clear();
        self.filter_menu.popdown();
        self.pointer_pending.set(false);
        self.generation.set(self.generation.get().wrapping_add(1));
        for cancel in self.preview_cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        self.underlays.hide();
        self.window().set_keyboard_mode(KeyboardMode::None);
        self.surface.hide();
        for callback in self.callbacks.borrow().clone() {
            callback(false);
        }
    }
    fn cancel_paste(&self) {
        if let Some(task) = self.pending_paste.borrow_mut().take() {
            task.abort();
        }
    }
    pub fn close(&self) {
        if self.stopped.replace(true) {
            return;
        }
        self.hide();
        self.cancel_paste();
        for process in self.processes.take() {
            process.force_exit();
        }
        for path in self.temporary_images.take() {
            let _ = std::fs::remove_file(&path);
            if let Some(parent) = path.parent() {
                let _ = std::fs::remove_dir(parent);
            }
        }
        if let Some(id) = self.subscription.take() {
            self.service.disconnect(id);
        }
        if let Some(handler) = self.style_handler.borrow_mut().take() {
            self.style_manager.disconnect(handler);
        }
        if let Some((monitor, handler)) = self.monitor.borrow_mut().take() {
            monitor.disconnect(handler);
        }
        if let Some(preferences) = self.preferences.borrow_mut().take() {
            preferences.destroy();
        }
        self.underlays.close();
        self.surface.close();
        gtk4::style_context_remove_provider_for_display(
            &gtk4::prelude::WidgetExt::display(self.window()),
            &self.style,
        );
    }
    fn place(&self, pointer: Option<(f64, f64)>) {
        if pointer.is_some() {
            self.pointer.set(pointer);
        }
        let monitor = self.monitor.borrow();
        let Some((monitor, _)) = monitor.as_ref() else {
            return;
        };
        let geometry = monitor.geometry();
        let settings = &self.service.settings;
        let vertical = settings.string("clipboard-orientation") == "vertical";
        let margins = [
            settings.int("clipboard-margin-top"),
            settings.int("clipboard-margin-right"),
            settings.int("clipboard-margin-bottom"),
            settings.int("clipboard-margin-left"),
        ];
        let horizontal = settings.string("clipboard-position-horizontal");
        let position = settings.string("clipboard-position-vertical");
        let available_width = (geometry.width() - margins[1] - margins[3]).max(180);
        let available_height = (geometry.height() - margins[0] - margins[2]).max(150);
        let width = if vertical {
            settings.int("item-width") + 30
        } else if horizontal == "fill" && !settings.boolean("show-at-pointer") {
            available_width
        } else {
            settings.int("clipboard-size")
        }
        .min(available_width);
        let header_height = self.header.measure(gtk4::Orientation::Vertical, -1).1;
        let height = if vertical {
            if position == "fill" && !settings.boolean("show-at-pointer") {
                available_height
            } else {
                settings.int("clipboard-size")
            }
        } else {
            settings.int("item-height") + header_height + 32
        }
        .min(available_height);
        let search_width = if vertical {
            width - 30
        } else {
            width
                - 42
                - self.controls.measure(gtk4::Orientation::Horizontal, -1).1
                - self.clear.measure(gtk4::Orientation::Horizontal, -1).1
        };
        self.search_box
            .set_width_request(search_width.clamp(180, 350));
        self.panel.set_size_request(width, height);
        let x = match horizontal.as_str() {
            "center" => (available_width - width) / 2,
            "bottom" => available_width - width,
            _ => 0,
        } + margins[3];
        let y = match position.as_str() {
            "center" => (available_height - height) / 2,
            "bottom" => available_height - height,
            _ => 0,
        } + margins[0];
        let (x, y) = self
            .pointer
            .get()
            .filter(|_| settings.boolean("show-at-pointer"))
            .map(|(x, y)| {
                (
                    x.clamp(0.0, f64::from((geometry.width() - width).max(0))),
                    y.clamp(0.0, f64::from((geometry.height() - height).max(0))),
                )
            })
            .unwrap_or((f64::from(x), f64::from(y)));
        self.root.move_(&self.panel, x, y);
        self.list.set_orientation(if vertical {
            gtk4::Orientation::Vertical
        } else {
            gtk4::Orientation::Horizontal
        });
        self.list
            .remove_css_class(if vertical { "horizontal" } else { "vertical" });
        self.list
            .add_css_class(if vertical { "vertical" } else { "horizontal" });
        let scrolling = if settings.boolean("show-scrollbar") {
            gtk4::PolicyType::Automatic
        } else {
            gtk4::PolicyType::External
        };
        self.scroller.set_policy(
            if vertical {
                gtk4::PolicyType::Never
            } else {
                scrolling
            },
            if vertical {
                scrolling
            } else {
                gtk4::PolicyType::Never
            },
        );
        self.footer.set_visible(vertical);
        if vertical && self.controls.parent().as_ref() != Some(self.footer.upcast_ref()) {
            self.header.set_start_widget(None::<&gtk4::Widget>);
            self.header.set_end_widget(None::<&gtk4::Widget>);
            self.controls.set_hexpand(true);
            self.footer.append(&self.controls);
            self.footer.append(&self.clear);
        } else if !vertical && self.controls.parent().as_ref() == Some(self.footer.upcast_ref()) {
            self.footer.remove(&self.controls);
            self.footer.remove(&self.clear);
            self.controls.set_hexpand(false);
            self.header.set_start_widget(Some(&self.controls));
            self.header.set_end_widget(Some(&self.clear));
        }
        let show_header = !settings.boolean("auto-hide-search")
            || self.search.has_focus()
            || !self.search.text().is_empty();
        self.header.set_visible(show_header);
        if show_header {
            self.panel.add_css_class("clipboard-show-header");
        } else {
            self.panel.remove_css_class("clipboard-show-header");
        }
    }
    fn render(self: &Rc<Self>) {
        if !self.visible.get() {
            return;
        }
        let error = self.service.error();
        self.retry
            .set_opacity(if error.is_some() { 1.0 } else { 0.0 });
        self.retry.set_sensitive(error.is_some());
        if let Some(error) = error {
            self.progress.clear();
            self.status.set_text(&error);
        } else if !self.service.ready() {
            self.progress.begin();
            self.status.set_text("");
        } else {
            self.progress.finish(|| {});
            self.status.set_text("");
        }
        self.place(None);
        self.apply_style();
        if self.last_revision.replace(self.service.revision()) != self.service.revision() {
            self.filter();
        }
        self.empty.set_visible(
            self.service.ready() && self.service.error().is_none() && self.model.n_items() == 0,
        );
    }
    fn filter(&self) {
        let selected = self.selected_id();
        let settings = &self.service.settings;
        let filter = Filter {
            query: self.search.text().to_string(),
            kind: self
                .kind
                .selected()
                .checked_sub(1)
                .and_then(|i| Kind::ALL.get(i as usize).copied()),
            pinned: self.pinned.is_active(),
            tag: self
                .tag
                .selected()
                .checked_sub(1)
                .and_then(|i| TAGS.get(i as usize).map(|tag| (*tag).into())),
            exclude_pinned: settings.boolean("exclude-pinned"),
            exclude_tagged: settings.boolean("exclude-tagged"),
        };
        self.filter_icon.set_icon_name(Some(
            filter
                .kind
                .map_or("system-search-symbolic", |kind| kind.icon()),
        ));
        for tag in TAGS {
            let class = format!("clipboard-tag-{tag}");
            if filter.tag.as_deref() == Some(tag) {
                self.filter_menu.add_css_class(&class);
            } else {
                self.filter_menu.remove_css_class(&class);
            }
        }
        let ids = self
            .service
            .entries()
            .iter()
            .filter(|entry| filter.matches(entry))
            .map(|entry| entry.id.to_string())
            .collect::<Vec<_>>();
        let refs = ids.iter().map(String::as_str).collect::<Vec<_>>();
        self.model.splice(0, self.model.n_items(), &refs);
        if let Some(position) =
            selected.and_then(|id| ids.iter().position(|value| value == &id.to_string()))
        {
            self.selection.set_selected(position as u32);
        }
        if self.service.ready() && self.service.error().is_none() {
            self.status.set_text("");
        }
        self.empty
            .set_visible(self.service.ready() && self.service.error().is_none() && ids.is_empty());
        let history_empty = self.service.entries().is_empty();
        self.empty_icon.set_icon_name(Some(if history_empty {
            "way-shell-clipboard-symbolic"
        } else {
            "way-shell-search-clipboard-symbolic"
        }));
        self.empty_title.set_text(if history_empty {
            "Clipboard is Empty"
        } else {
            "No Items Found"
        });
    }
    fn selected_id(&self) -> Option<i64> {
        self.selection
            .selected_item()
            .and_downcast::<gtk4::StringObject>()?
            .string()
            .parse()
            .ok()
    }
    fn navigate(&self, delta: i32) {
        let length = self.model.n_items();
        if length == 0 {
            return;
        }
        let current = self.selection.selected().min(length - 1) as i32;
        self.selection
            .set_selected((current + delta).rem_euclid(length as i32) as u32);
        self.list.grab_focus();
    }
    fn activate(self: &Rc<Self>, id: i64, copy_only: bool) {
        match self.service.restore(id) {
            Ok(_) => {
                let target = self.target.borrow().clone();
                self.hide();
                if !copy_only {
                    self.schedule_paste(target);
                }
            }
            Err(error) => self.service.report(error),
        }
    }
    fn schedule_paste(self: &Rc<Self>, target: Option<Toplevel>) {
        self.cancel_paste();
        let Some(target) = target else {
            self.service.report(
                "Copied to clipboard; the previous application is no longer available".into(),
            );
            return;
        };
        let generation = self.generation.get();
        let weak = Rc::downgrade(self);
        let task = glib::spawn_future_local(async move {
            // Wait for unmapping and focus restoration rather than racing a fixed paste delay.
            for _ in 0..50 {
                glib::timeout_future(Duration::from_millis(10)).await;
                let Some(this) = weak.upgrade().filter(|this| {
                    !this.stopped.get()
                        && !this.visible.get()
                        && this.generation.get() == generation
                }) else {
                    return;
                };
                if let Some(focused) = this.service.wayland.focused_toplevel() {
                    if focused.id != target.id {
                        return;
                    }
                    if let Err(error) = this.service.wayland.paste(this.service.terminal(&target)) {
                        this.service.report(format!("Copied to clipboard; {error}"));
                    }
                    return;
                }
            }
        });
        self.pending_paste.replace(Some(task));
    }
    fn confirm_clear(self: &Rc<Self>) -> libadwaita::MessageDialog {
        let dialog = libadwaita::MessageDialog::new(
            Some(self.window()),
            Some("Clear clipboard history?"),
            Some("Pinned and tagged items are kept unless you include them below."),
        );
        let all = gtk4::CheckButton::with_label("Include pinned and tagged items");
        dialog.set_extra_child(Some(&all));
        dialog.add_responses(&[("cancel", "Cancel"), ("clear", "Clear")]);
        dialog.set_response_appearance("clear", libadwaita::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let service = self.service.clone();
        dialog.connect_response(None, move |dialog, response| {
            if response == "clear" {
                service.clear(all.is_active());
            }
            dialog.close();
        });
        self.present_dialog(&dialog);
        dialog
    }
    fn present_dialog(&self, dialog: &libadwaita::MessageDialog) {
        // A normal toplevel sits below the clipboard's full-output overlay and
        // cannot receive pointer input. Keep child dialogs on that same layer.
        dialog.init_layer_shell();
        dialog.set_namespace(Some("way-shell-clipboard-dialog"));
        dialog.set_layer(Layer::Overlay);
        dialog.set_exclusive_zone(0);
        dialog.set_keyboard_mode(KeyboardMode::Exclusive);
        if let Some((monitor, _)) = self.monitor.borrow().as_ref() {
            dialog.set_monitor(Some(monitor));
        }
        dialog.present();
    }
    fn install_keys(self: &Rc<Self>) {
        let keys = gtk4::EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(this) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
            if key == gdk::Key::Escape {
                if !this.search.text().is_empty() {
                    this.search.set_text("");
                } else {
                    this.hide();
                }
                return glib::Propagation::Stop;
            }
            if ctrl && key == gdk::Key::Tab {
                this.kind
                    .set_selected(cycle(this.kind.selected(), 9, shift));
                return glib::Propagation::Stop;
            }
            if ctrl && matches!(key, gdk::Key::grave | gdk::Key::asciitilde) {
                this.tag.set_selected(cycle(this.tag.selected(), 10, shift));
                return glib::Propagation::Stop;
            }
            if shortcut(
                &this.service.settings,
                "toggle-pinned-search-shortcut",
                key,
                state,
            ) {
                this.pinned.set_active(!this.pinned.is_active());
                return glib::Propagation::Stop;
            }
            if !this.visible.get() {
                return glib::Propagation::Proceed;
            }
            let Some(id) = this.selected_id() else {
                return glib::Propagation::Proceed;
            };
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                || (key == gdk::Key::space && !this.search.has_focus())
            {
                if ctrl {
                    this.default_action(id);
                } else {
                    this.activate(
                        id,
                        shift ^ this.service.settings.boolean("swap-copy-shortcut"),
                    );
                }
                return glib::Propagation::Stop;
            }
            if !this.search.has_focus() {
                if shortcut(&this.service.settings, "pin-item-shortcut", key, state) {
                    if let Some(mut entry) = this.service.entry(id) {
                        entry.pinned = !entry.pinned;
                        this.service.update(entry);
                    }
                    return glib::Propagation::Stop;
                }
                if shortcut(
                    &this.service.settings,
                    "delete-item-shortcut",
                    key,
                    state & !gdk::ModifierType::SHIFT_MASK,
                ) {
                    this.service.delete(id, shift);
                    return glib::Propagation::Stop;
                }
                if shortcut(&this.service.settings, "edit-item-shortcut", key, state) {
                    this.edit(id, false);
                    return glib::Propagation::Stop;
                }
                if shortcut(&this.service.settings, "edit-title-shortcut", key, state) {
                    this.edit(id, true);
                    return glib::Propagation::Stop;
                }
                if shortcut(&this.service.settings, "open-menu-shortcut", key, state) {
                    this.show_item_menu(id);
                    return glib::Propagation::Stop;
                }
            }
            for action in this
                .service
                .actions()
                .actions
                .iter()
                .flat_map(flatten_actions)
            {
                if action
                    .shortcut
                    .iter()
                    .any(|value| accelerator(value, key, state))
                {
                    this.run_action(id, action.clone());
                    return glib::Propagation::Stop;
                }
            }
            if ctrl && let Some(number) = key.to_unicode().and_then(|c| c.to_digit(10)) {
                let index = if number == 0 { 9 } else { number - 1 };
                if index < this.model.n_items() {
                    this.selection.set_selected(index);
                }
                return glib::Propagation::Stop;
            }
            match key {
                gdk::Key::Left | gdk::Key::Up if !this.search.has_focus() => this.navigate(-1),
                gdk::Key::Right | gdk::Key::Down => this.navigate(1),
                gdk::Key::Tab => this.navigate(if shift { -1 } else { 1 }),
                gdk::Key::Home if !this.search.has_focus() => this.selection.set_selected(0),
                gdk::Key::End if !this.search.has_focus() => this
                    .selection
                    .set_selected(this.model.n_items().saturating_sub(1)),
                _ => {
                    if !ctrl
                        && !state.contains(gdk::ModifierType::ALT_MASK)
                        && !this.search.has_focus()
                        && let Some(character) = key
                            .to_unicode()
                            .filter(|c| !c.is_control() && !c.is_whitespace())
                    {
                        this.search
                            .set_text(&format!("{}{character}", this.search.text()));
                        this.search.grab_focus();
                        this.search.set_position(-1);
                    } else {
                        return glib::Propagation::Proceed;
                    }
                }
            }
            glib::Propagation::Stop
        });
        self.window().add_controller(keys);
    }
    fn apply_style(&self) {
        let settings = &self.service.settings;
        let theme = settings.child("theme");
        let signature = format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}",
            theme.string("theme"),
            theme.string("color-scheme"),
            theme.string("custom-color-scheme"),
            theme.string("custom-bg-color"),
            theme.string("custom-fg-color"),
            theme.string("custom-card-bg-color"),
            theme.string("custom-search-bg-color"),
            settings.string("custom-css"),
            self.style_manager.is_dark()
        );
        if *self.style_signature.borrow() == signature {
            return;
        }
        self.style_signature.replace(signature);
        let mut css = String::new();
        let scheme = theme.string("color-scheme");
        let light = self.light_scheme();
        if light {
            css.push_str(".clipboard-dialog,.clipboard-popover > contents {background:#fafafb;color:#222226;}.clipboard-card,.clipboard-search {background:#fff;color:#222226;}.clipboard-card:hover {background:#f2f2f4;}.clipboard-time {color:#777;}");
        } else {
            css.push_str(".clipboard-dialog,.clipboard-popover > contents {background:#36363a;color:#fff;}.clipboard-card,.clipboard-search {background:#47474c;color:#fff;}.clipboard-card:hover {background:mix(#47474c,#fff,.05);}");
        }
        if theme.string("theme") == "yaru" {
            css.push_str(".clipboard-items row:selected .clipboard-card{outline-color:#e95420;}.clipboard-dialog{border-radius:16px;}");
        }
        if scheme == "high-contrast" {
            css.push_str(".clipboard-dialog{background:#000;color:#fff;border:2px solid #fff;}.clipboard-card{background:#000;border:2px solid #fff;}.clipboard-items row:selected .clipboard-card{outline:3px solid #fff;}");
        }
        if theme.string("theme") == "custom" {
            for (key, selector, property) in [
                (
                    "custom-bg-color",
                    ".clipboard-dialog,.clipboard-popover > contents",
                    "background",
                ),
                (
                    "custom-fg-color",
                    ".clipboard-dialog,.clipboard-popover > contents",
                    "color",
                ),
                ("custom-card-bg-color", ".clipboard-card", "background"),
                ("custom-search-bg-color", ".clipboard-search", "background"),
            ] {
                let value = theme.string(key);
                if !value.is_empty() && gdk::RGBA::parse(&value).is_ok() {
                    css.push_str(&format!("{selector}{{{property}:{value};}}"));
                }
            }
        }
        if theme.string("theme") == "custom" {
            let path = settings.string("custom-css");
            if let Ok(custom) = std::fs::read_to_string(path.as_str()) {
                css.push_str(&custom);
            }
        }
        self.style.load_from_data(&css);
    }
    fn light_scheme(&self) -> bool {
        let theme = self.service.settings.child("theme");
        if theme.string("theme") == "custom" {
            return theme.string("custom-color-scheme") == "light";
        }
        let scheme = theme.string("color-scheme");
        scheme == "light" || (scheme == "system" && !self.style_manager.is_dark())
    }
}
impl Drop for ClipboardDialog {
    fn drop(&mut self) {
        self.close();
    }
}
fn icon_button(icon: &str, tooltip: &str) -> gtk4::Button {
    let button = gtk4::Button::from_icon_name(icon);
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("circular");
    button.update_property(&[gtk4::accessible::Property::Label(tooltip)]);
    button
}
fn tag_picker(selection: &gtk4::SingleSelection) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    row.add_css_class("clipboard-tags");
    let tags = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    for (index, tag) in std::iter::once(None)
        .chain(TAGS.into_iter().map(Some))
        .enumerate()
    {
        let button = gtk4::ToggleButton::new();
        button.add_css_class("clipboard-tag-button");
        if let Some(tag) = tag {
            button.add_css_class(&format!("clipboard-tag-{tag}"));
            let swatch = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            swatch.add_css_class("clipboard-tag-swatch");
            button.set_child(Some(&swatch));
        } else {
            button.set_icon_name("edit-clear-symbolic");
        }
        let label = format!("{} ({index})", tag.unwrap_or("No tag"));
        button.set_tooltip_text(Some(&label));
        button.update_property(&[gtk4::accessible::Property::Label(&label)]);
        button.set_active(selection.selected() == index as u32);
        let selected = selection.clone();
        button.connect_clicked(move |button| {
            selected.set_selected(if selected.selected() == index as u32 {
                0
            } else {
                index as u32
            });
            button.set_active(selected.selected() == index as u32);
        });
        let weak = button.downgrade();
        selection.connect_selected_notify(move |selection| {
            if let Some(button) = weak.upgrade() {
                button.set_active(selection.selected() == index as u32);
            }
        });
        tags.append(&button);
    }
    let scroller = gtk4::ScrolledWindow::new();
    scroller.set_policy(gtk4::PolicyType::External, gtk4::PolicyType::Never);
    scroller.set_min_content_width(170);
    scroller.set_max_content_width(170);
    scroller.set_propagate_natural_width(true);
    scroller.set_child(Some(&tags));
    let previous = gtk4::Button::from_icon_name("pan-start-symbolic");
    let next = gtk4::Button::from_icon_name("pan-end-symbolic");
    for (button, direction) in [(&previous, -1.0), (&next, 1.0)] {
        let adjustment = scroller.hadjustment();
        button.connect_clicked(move |_| {
            adjustment.set_value((adjustment.value() + direction * 170.0).clamp(
                adjustment.lower(),
                (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()),
            ));
        });
    }
    row.append(&previous);
    row.append(&scroller);
    row.append(&next);
    row
}
fn cycle(current: u32, length: u32, reverse: bool) -> u32 {
    if reverse {
        (current + length - 1) % length
    } else {
        (current + 1) % length
    }
}
fn accelerator(value: &str, key: gdk::Key, state: gdk::ModifierType) -> bool {
    gtk4::accelerator_parse(value).is_some_and(|(expected, modifiers)| {
        expected.to_lower() == key.to_lower()
            && state & gtk4::accelerator_get_default_mod_mask() == modifiers
    })
}
fn shortcut(settings: &gio::Settings, name: &str, key: gdk::Key, state: gdk::ModifierType) -> bool {
    settings.strv(name).iter().any(|value| {
        accelerator(value, key, state)
            || (value.as_str() == "Alt" && matches!(key, gdk::Key::Alt_L | gdk::Key::Alt_R))
    })
}
fn flatten_actions(
    action: &way_shell_clipboard::actions::Action,
) -> Vec<&way_shell_clipboard::actions::Action> {
    if !action.is_group() {
        vec![action]
    } else {
        action.actions.iter().flat_map(flatten_actions).collect()
    }
}
