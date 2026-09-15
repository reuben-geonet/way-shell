//! Temporary borrowed tray records and owned Rust menus for the remaining C panel.
use gio::prelude::*;
use glib::{ffi, subclass::prelude::*, translate::*};
use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
use std::{
    cell::{Cell, RefCell, UnsafeCell},
    collections::{BTreeMap, VecDeque},
    ffi::{CString, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    rc::{Rc, Weak},
    sync::OnceLock,
    time::Duration,
};
use way_shell::services::tray::{
    ItemCommand, ItemKey, Orientation, RgbaImage, TrayEvent, TrayItem, TrayService,
    menu::{MenuNode, MenuService, MenuState},
};

/// The existing C layout remains borrowed. Rust owns the menu and action group;
/// both obsolete proxy fields stay null.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ItemRecord {
    proxy: *mut c_void,
    menu_proxy: *mut c_void,
    action_group: *mut c_void,
    menu_model: *mut c_void,
    bus_name: *const c_char,
    obj_name: *const c_char,
    register_service_name: *const c_char,
    category: *const c_char,
    id: *const c_char,
    title: *const c_char,
    status: *const c_char,
    window_id: u32,
    icon_pixmap_from_theme: *mut gtk::gdk_pixbuf::ffi::GdkPixbuf,
    icon_theme_path: *const c_char,
    icon_name: *const c_char,
    icon_pixmap: *mut gtk::gdk_pixbuf::ffi::GdkPixbuf,
    overlay_icon_name: *const c_char,
    overlay_icon_pixmap: *mut gtk::gdk_pixbuf::ffi::GdkPixbuf,
    attention_icon_name: *const c_char,
    attention_icon_pixmap: *mut gtk::gdk_pixbuf::ffi::GdkPixbuf,
    attention_movie_name: *const c_char,
}

fn string(value: &str) -> CString {
    // D-Bus excludes NUL. Manually supplied test snapshots are defensive too.
    CString::new(value).unwrap_or_default()
}
fn pixbuf(image: &RgbaImage) -> Option<Pixbuf> {
    let width = i32::try_from(image.width).ok().filter(|width| *width > 0)?;
    let height = i32::try_from(image.height)
        .ok()
        .filter(|height| *height > 0)?;
    let stride = width.checked_mul(4)?;
    let length = (stride as usize).checked_mul(height as usize)?;
    if image.pixels.len() != length {
        return None;
    }
    // GBytes retains these bytes even after the tray record or source variant
    // disappears, including when a GTK texture still retains the pixbuf.
    Some(Pixbuf::from_bytes(
        &glib::Bytes::from_owned(image.pixels.clone()),
        Colorspace::Rgb,
        true,
        8,
        width,
        height,
        stride,
    ))
}
fn pixbuf_pointer(image: &Option<Pixbuf>) -> *mut gtk::gdk_pixbuf::ffi::GdkPixbuf {
    image
        .as_ref()
        .map_or(std::ptr::null_mut(), ObjectType::as_ptr)
}
struct Metadata {
    strings: [CString; 9],
    menu: CString,
    images: [Option<Pixbuf>; 3],
    theme: Option<Pixbuf>,
}

struct OwnedMenu {
    service: MenuService,
    handler: Option<glib::SignalHandlerId>,
    actions: gio::SimpleActionGroup,
    model: Option<gio::Menu>,
}
impl Drop for OwnedMenu {
    fn drop(&mut self) {
        if let Some(handler) = self.handler.take() {
            self.service.disconnect(handler);
        }
        for name in ["item-clicked", "about-to-show"] {
            if let Some(action) = self.actions.lookup_action(name) {
                action
                    .downcast::<gio::SimpleAction>()
                    .unwrap()
                    .set_enabled(false);
            }
        }
        self.service.stop();
    }
}

fn menu_actions(
    record: &Rc<Record>,
    service: &MenuService,
    revision: u64,
) -> gio::SimpleActionGroup {
    let actions = gio::SimpleActionGroup::new();
    for name in ["item-clicked", "about-to-show"] {
        let action = gio::SimpleAction::new(name, Some(glib::VariantTy::new("(si)").unwrap()));
        action.set_enabled(false);
        let weak_record = Rc::downgrade(record);
        let weak_service = service.downgrade();
        action.connect_activate(move |_, parameter| {
            let Some((key, id)) = parameter.and_then(|value| value.get::<(String, i32)>()) else {
                return;
            };
            let Some(record) = weak_record.upgrade().filter(|record| {
                record.menu_revision.get() == revision && record.key.registration() == key
            }) else {
                return;
            };
            let Some(adapter) = record
                .adapter
                .upgrade()
                .filter(|adapter| adapter.contains(&record))
            else {
                return;
            };
            let Some(service) = weak_service.upgrade() else {
                return;
            };
            // Keep owners alive throughout the synchronous dispatch. The D-Bus
            // operation itself owns only its cancellable and weak subscriptions.
            let _adapter = adapter;
            if name == "item-clicked" {
                let timestamp = (glib::real_time() / 1_000_000) as u32;
                service.activate(id, timestamp, |result| {
                    if let Err(error) = result {
                        glib::g_message!("way-shell", "Tray menu activation failed: {error}");
                    }
                });
            } else {
                service.about_to_show(id, |result| {
                    if let Err(error) = result {
                        glib::g_message!("way-shell", "Tray menu AboutToShow failed: {error}");
                    }
                });
            }
        });
        actions.add_action(&action);
    }
    actions
}

fn build_menu(root: &MenuNode, key: &str) -> gio::Menu {
    let menu = gio::Menu::new();
    let mut section: Option<gio::Menu> = None;
    for child in root.children.iter().filter(|child| child.visible) {
        if child.separator {
            if let Some(previous) = section.replace(gio::Menu::new()) {
                menu.append_section(None, &previous);
            }
            continue;
        }
        let item = gio::MenuItem::new(Some(&child.label), None);
        item.set_action_and_target_value(
            Some("sni.item-clicked"),
            Some(&(key, child.id).to_variant()),
        );
        if !child.children.is_empty() {
            item.set_submenu(Some(&build_menu(child, key)));
        }
        section.as_ref().unwrap_or(&menu).append_item(&item);
    }
    if let Some(section) = section {
        menu.append_section(None, &section);
    }
    menu
}
impl Metadata {
    fn new(item: &TrayItem) -> Self {
        Self {
            strings: [
                &item.category,
                &item.id,
                &item.title,
                &item.status,
                &item.icon_theme_path,
                &item.icon.name,
                &item.overlay.name,
                &item.attention.name,
                &item.attention_movie_name,
            ]
            .map(|value| string(value)),
            menu: string(item.menu_path.as_deref().unwrap_or("")),
            images: [&item.icon, &item.overlay, &item.attention]
                .map(|icon| icon.pixmap.as_ref().and_then(pixbuf)),
            theme: None,
        }
    }
}

/// The ABI record is the first field, allowing item-only C methods to recover
/// their private owner. Every non-null pointer must be a live borrowed record.
#[repr(C)]
struct Record {
    value: UnsafeCell<ItemRecord>,
    key: ItemKey,
    canonical: CString,
    owner: CString,
    path: CString,
    item: RefCell<TrayItem>,
    metadata: RefCell<Metadata>,
    adapter: glib::WeakRef<TrayAdapter>,
    menu: RefCell<Option<OwnedMenu>>,
    menu_revision: Cell<u64>,
    theme_revision: Cell<u64>,
    theme_task: RefCell<Option<glib::JoinHandle<()>>>,
}
impl Record {
    fn new(item: TrayItem, adapter: &TrayAdapter) -> Rc<Self> {
        let record = Rc::new(Self {
            value: UnsafeCell::new(ItemRecord::default()),
            key: item.key.clone(),
            canonical: string(&item.key.registration()),
            owner: string(&item.key.owner),
            path: string(&item.key.path),
            metadata: RefCell::new(Metadata::new(&item)),
            item: RefCell::new(item),
            adapter: adapter.downgrade(),
            menu: RefCell::new(None),
            menu_revision: Cell::new(0),
            theme_revision: Cell::new(0),
            theme_task: RefCell::new(None),
        });
        record.write_fields();
        record
    }
    fn pointer(&self) -> *mut ItemRecord {
        self.value.get()
    }
    fn write_fields(&self) {
        let metadata = self.metadata.borrow();
        let [
            category,
            id,
            title,
            status,
            icon_theme_path,
            icon_name,
            overlay_icon_name,
            attention_icon_name,
            attention_movie_name,
        ] = metadata.strings.each_ref().map(|value| value.as_ptr());
        let [icon_pixmap, overlay_icon_pixmap, attention_icon_pixmap] =
            metadata.images.each_ref().map(pixbuf_pointer);
        let item = self.item.borrow();
        let menu = self.menu.borrow();
        unsafe {
            *self.value.get() = ItemRecord {
                proxy: std::ptr::null_mut(),
                menu_proxy: std::ptr::null_mut(),
                action_group: menu
                    .as_ref()
                    .map_or(std::ptr::null_mut(), |menu| menu.actions.as_ptr().cast()),
                menu_model: menu
                    .as_ref()
                    .and_then(|menu| menu.model.as_ref())
                    .map_or(std::ptr::null_mut(), |model| model.as_ptr().cast()),
                bus_name: self.owner.as_ptr(),
                obj_name: self.path.as_ptr(),
                register_service_name: self.canonical.as_ptr(),
                category,
                id,
                title,
                status,
                window_id: item.window_id,
                icon_pixmap_from_theme: pixbuf_pointer(&metadata.theme),
                icon_theme_path,
                icon_name,
                icon_pixmap,
                overlay_icon_name,
                overlay_icon_pixmap,
                attention_icon_name,
                attention_icon_pixmap,
                attention_movie_name,
            };
        }
    }
    fn update(&self, item: TrayItem) -> bool {
        let old = self.item.borrow().clone();
        let theme_changed =
            old.icon_theme_path != item.icon_theme_path || old.icon.name != item.icon.name;
        if old.menu_path != item.menu_path {
            self.stop_menu();
        }
        let mut metadata = Metadata::new(&item);
        if !theme_changed {
            metadata.theme = self.metadata.borrow().theme.clone();
        }
        self.item.replace(item);
        self.metadata.replace(metadata);
        self.write_fields();
        theme_changed
    }
    fn start_menu(self: &Rc<Self>) {
        if self.menu.borrow().is_some() {
            return;
        }
        let Some(adapter) = self.adapter.upgrade() else {
            return;
        };
        let Some(service) = adapter.imp().service.borrow().clone() else {
            return;
        };
        let menu = match service.menu(&self.key) {
            Ok(Some(menu)) => menu,
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
        let weak_record = Rc::downgrade(self);
        let weak_adapter = adapter.downgrade();
        let handler = menu.connect_local("snapshot", false, move |values| {
            if let Some(record) = weak_record.upgrade()
                && let Some(adapter) = weak_adapter.upgrade()
            {
                adapter.push(Event::Menu {
                    record,
                    revision,
                    state: values[1].get::<MenuState>().expect("typed menu snapshot"),
                });
            }
            None
        });
        let actions = menu_actions(self, &menu, revision);
        self.menu.replace(Some(OwnedMenu {
            service: menu.clone(),
            handler: Some(handler),
            actions,
            model: None,
        }));
        self.write_fields();
        let state = menu.state();
        if state != MenuState::default() {
            adapter.push(Event::Menu {
                record: self.clone(),
                revision,
                state,
            });
        }
    }
    fn stop_menu(&self) {
        self.menu_revision
            .set(self.menu_revision.get().wrapping_add(1));
        let menu = self.menu.borrow_mut().take();
        drop(menu);
        self.write_fields();
    }
    fn cancel_theme(&self) {
        self.theme_revision
            .set(self.theme_revision.get().wrapping_add(1));
        if let Some(task) = self.theme_task.borrow_mut().take() {
            task.abort();
        }
    }
    fn load_theme(self: &Rc<Self>) {
        self.cancel_theme();
        let item = self.item.borrow().clone();
        if item.icon_theme_path.is_empty() || item.icon.name.is_empty() {
            return;
        }
        let revision = self.theme_revision.get();
        let record = Rc::downgrade(self);
        let adapter = self.adapter.clone();
        let task = glib::MainContext::ref_thread_default().spawn_local(async move {
            // GioFuture cancels its native request when timeout/removal drops it.
            let result = glib::future_with_timeout(
                Duration::from_secs(2),
                load_theme_icon(&item.icon_theme_path, &item.icon.name),
            )
            .await;
            let image = match result {
                Ok(Ok(image)) => Some(image),
                Ok(Err(error)) => {
                    glib::g_message!(
                        "way-shell",
                        "Tray icon {} could not be loaded: {error}",
                        item.icon.name
                    );
                    None
                }
                Err(_) => {
                    glib::g_message!(
                        "way-shell",
                        "Tray icon {} loading timed out",
                        item.icon.name
                    );
                    None
                }
            };
            if let Some(adapter) = adapter.upgrade() {
                adapter.push(Event::Theme {
                    record,
                    revision,
                    image,
                });
            }
        });
        self.theme_task.replace(Some(task));
    }
}
impl Drop for Record {
    fn drop(&mut self) {
        self.cancel_theme();
        self.stop_menu();
    }
}

async fn load_theme_icon(directory: &str, name: &str) -> Result<Pixbuf, glib::Error> {
    let directory = gio::File::for_path(directory);
    let file = if name.contains('.') {
        directory.child(name)
    } else {
        let entries = directory
            .enumerate_children_future(
                "standard::name",
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await?;
        let result = async {
            loop {
                let batch = entries
                    .next_files_future(64, glib::Priority::DEFAULT)
                    .await?;
                if batch.is_empty() {
                    break;
                }
                for entry in batch {
                    let path = entry.name();
                    let text = path.to_string_lossy();
                    if text.starts_with(name)
                        && [".png", ".svg", ".xpm", ".ico"]
                            .iter()
                            .any(|suffix| text.ends_with(suffix))
                    {
                        return Ok(directory.child(path));
                    }
                }
            }
            Err(glib::Error::new(
                gio::IOErrorEnum::NotFound,
                "icon was absent from its theme directory",
            ))
        }
        .await;
        let _ = entries.close_future(glib::Priority::DEFAULT).await;
        result?
    };
    let info = file
        .query_info_future(
            "standard::type",
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await?;
    if info.file_type() != gio::FileType::Regular {
        return Err(glib::Error::new(
            gio::IOErrorEnum::InvalidArgument,
            "tray icon is not a regular file",
        ));
    }
    let stream = file.read_future(glib::Priority::DEFAULT).await?;
    let result = Pixbuf::from_stream_at_scale_future(&stream, 256, 256, true).await;
    let _ = stream.close_future(glib::Priority::DEFAULT).await;
    result
}

struct ItemTable(*mut ffi::GHashTable);
impl Default for ItemTable {
    fn default() -> Self {
        Self(unsafe { ffi::g_hash_table_new(Some(ffi::g_str_hash), Some(ffi::g_str_equal)) })
    }
}
impl Drop for ItemTable {
    fn drop(&mut self) {
        unsafe { ffi::g_hash_table_unref(self.0) };
    }
}
fn table_type() -> glib::Type {
    unsafe { from_glib(ffi::g_hash_table_get_type()) }
}
impl ItemTable {
    fn insert(&self, record: &Record) {
        unsafe {
            ffi::g_hash_table_insert(
                self.0,
                record.canonical.as_ptr().cast_mut().cast(),
                record.pointer().cast(),
            )
        };
    }
    fn remove(&self, record: &Record) {
        unsafe { ffi::g_hash_table_remove(self.0, record.canonical.as_ptr().cast()) };
    }
    fn value(&self) -> glib::Value {
        let mut value = glib::Value::from_type(table_type());
        unsafe { glib::gobject_ffi::g_value_set_boxed(value.to_glib_none_mut().0, self.0.cast()) };
        value
    }
}
enum Event {
    Service(Box<TrayEvent>),
    Menu {
        record: Rc<Record>,
        revision: u64,
        state: MenuState,
    },
    Theme {
        record: Weak<Record>,
        revision: u64,
        image: Option<Pixbuf>,
    },
}
mod imp {
    use super::*;
    #[derive(Default)]
    pub struct TrayAdapter {
        pub service: RefCell<Option<TrayService>>,
        pub handler: RefCell<Option<glib::SignalHandlerId>>,
        pub(super) records: RefCell<BTreeMap<String, Rc<Record>>>,
        pub(super) table: ItemTable,
        pub(super) events: RefCell<VecDeque<Event>>,
        pub publishing: Cell<bool>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for TrayAdapter {
        const NAME: &'static str = "WayShellTrayAdapter";
        type Type = super::TrayAdapter;
    }
    impl ObjectImpl for TrayAdapter {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                let mut signals: Vec<_> =
                    ["status-notifier-item-added", "status-notifier-item-removed"]
                        .into_iter()
                        .map(|name| {
                            glib::subclass::Signal::builder(name)
                                .param_types([table_type(), glib::Type::POINTER])
                                .build()
                        })
                        .collect();
                signals.push(
                    glib::subclass::Signal::builder("status-notifier-item-changed")
                        .param_types([table_type()])
                        .build(),
                );
                signals.extend(
                    [
                        "status-notifier-item-properties-changed",
                        "status-notifier-item-menu-will-update",
                        "status-notifier-item-menu-updated",
                    ]
                    .into_iter()
                    .map(|name| {
                        glib::subclass::Signal::builder(name)
                            .param_types([glib::Type::POINTER])
                            .build()
                    }),
                );
                signals
            })
        }
        fn dispose(&self) {
            let service = self.service.borrow_mut().take();
            if let Some(service) = service {
                if let Some(handler) = self.handler.borrow_mut().take() {
                    service.disconnect(handler);
                }
                service.stop();
            }
            self.events.borrow_mut().clear();
            let records = std::mem::take(&mut *self.records.borrow_mut());
            unsafe { ffi::g_hash_table_remove_all(self.table.0) };
            for record in records.into_values() {
                record.cancel_theme();
                record.stop_menu();
            }
        }
    }
}
glib::wrapper! { pub struct TrayAdapter(ObjectSubclass<imp::TrayAdapter>); }
impl TrayAdapter {
    fn new(service: TrayService) -> Self {
        let adapter: Self = glib::Object::new();
        let weak = adapter.downgrade();
        let handler = service.connect_local("event", false, move |values| {
            if let Some(adapter) = weak.upgrade() {
                adapter.push(Event::Service(Box::new(
                    values[1].get::<TrayEvent>().expect("typed tray event"),
                )));
            }
            None
        });
        let items = service.state().items;
        adapter.imp().service.replace(Some(service));
        adapter.imp().handler.replace(Some(handler));
        for item in items {
            adapter.push(Event::Service(Box::new(TrayEvent::Added(item))));
        }
        adapter
    }
    fn push(&self, event: Event) {
        self.imp().events.borrow_mut().push_back(event);
        if self.imp().publishing.replace(true) {
            return;
        }
        loop {
            let event = self.imp().events.borrow_mut().pop_front();
            let Some(event) = event else {
                break;
            };
            self.apply(event);
        }
        self.imp().publishing.set(false);
    }
    fn emit_item(&self, signal: &str, record: &Record, table: bool) {
        let pointer = record.pointer().cast::<c_void>().to_value();
        if table {
            self.emit_by_name_with_values(signal, &[self.imp().table.value(), pointer]);
        } else {
            self.emit_by_name_with_values(signal, &[pointer]);
        }
    }
    fn contains(&self, record: &Rc<Record>) -> bool {
        self.imp()
            .records
            .borrow()
            .get(record.canonical.to_str().unwrap())
            .is_some_and(|value| Rc::ptr_eq(value, record))
    }
    fn apply_service(&self, event: TrayEvent) {
        match event {
            TrayEvent::Added(item) | TrayEvent::Changed(item) => {
                let key = item.key.registration();
                let current = self.imp().records.borrow().get(&key).cloned();
                let menu_changed = current
                    .as_ref()
                    .is_some_and(|record| record.item.borrow().menu_path != item.menu_path);
                if menu_changed && let Some(record) = &current {
                    self.emit_item("status-notifier-item-menu-will-update", record, false);
                }
                let (record, added, theme_changed) = if let Some(record) = current {
                    let changed = record.update(item);
                    (record, false, changed)
                } else {
                    let record = Record::new(item, self);
                    self.imp().table.insert(&record);
                    self.imp().records.borrow_mut().insert(key, record.clone());
                    (record, true, true)
                };
                self.emit_item(
                    if added {
                        "status-notifier-item-added"
                    } else {
                        "status-notifier-item-properties-changed"
                    },
                    &record,
                    added,
                );
                record.start_menu();
                if menu_changed {
                    self.emit_item("status-notifier-item-menu-updated", &record, false);
                }
                if theme_changed {
                    record.load_theme();
                }
            }
            TrayEvent::Removed(item) => {
                let key = item.key.registration();
                let record = self.imp().records.borrow().get(&key).cloned();
                if let Some(record) = record {
                    // Existing C observers see the old item and its table entry
                    // until every removal handler has finished borrowing them.
                    self.emit_item("status-notifier-item-removed", &record, true);
                    record.cancel_theme();
                    record.stop_menu();
                    self.imp().table.remove(&record);
                    self.imp().records.borrow_mut().remove(&key);
                }
            }
        }
    }
    fn apply(&self, event: Event) {
        match event {
            Event::Service(event) => self.apply_service(*event),
            Event::Menu {
                record,
                revision,
                state,
            } => {
                if !self.contains(&record)
                    || record.menu_revision.get() != revision
                    || record.menu.borrow().is_none()
                {
                    return;
                }
                let model = state
                    .root
                    .as_ref()
                    .map(|root| build_menu(root, &record.key.registration()));
                self.emit_item("status-notifier-item-menu-will-update", &record, false);
                // Explicit GObject disposal can release the endpoint directly,
                // unlike ordinary service removals, which remain queued.
                if !self.contains(&record) || record.menu_revision.get() != revision {
                    return;
                }
                let actions = {
                    let mut menu = record.menu.borrow_mut();
                    let Some(menu) = menu.as_mut() else {
                        return;
                    };
                    menu.model = model;
                    menu.actions.clone()
                };
                record.write_fields();
                for name in ["item-clicked", "about-to-show"] {
                    actions
                        .lookup_action(name)
                        .unwrap()
                        .downcast::<gio::SimpleAction>()
                        .unwrap()
                        .set_enabled(state.available);
                }
                self.emit_item("status-notifier-item-menu-updated", &record, false);
            }
            Event::Theme {
                record,
                revision,
                image,
            } => {
                let Some(record) = record.upgrade().filter(|record| {
                    self.contains(record) && record.theme_revision.get() == revision
                }) else {
                    return;
                };
                record.theme_task.borrow_mut().take();
                record.metadata.borrow_mut().theme = image;
                record.write_fields();
                self.emit_item("status-notifier-item-properties-changed", &record, false);
            }
        }
    }
    fn command(&self, key: &ItemKey, action: ItemCommand) {
        let service = self.imp().service.borrow().clone();
        if let Some(service) = service {
            service.command(key, action, |result| {
                if let Err(error) = result {
                    glib::g_message!("way-shell", "Tray item operation failed: {error}");
                }
            });
        }
    }
}
thread_local! { static GLOBAL: RefCell<Option<TrayAdapter>> = const { RefCell::new(None) }; }
pub fn shutdown() {
    let adapter = GLOBAL.with(|global| global.borrow().clone());
    if let Some(adapter) = adapter {
        let service = adapter.imp().service.borrow().clone();
        if let Some(service) = service {
            service.stop();
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn status_notifier_service_get_type() -> ffi::GType {
    catch_unwind(|| TrayAdapter::static_type().into_glib()).unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "C" fn status_notifier_service_global_init() -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if GLOBAL.with(|global| global.borrow().is_some()) {
            return 0;
        }
        let settings = gio::Settings::new("org.ldelossa.way-shell.panel");
        if !settings.boolean("enable-tray-icons") {
            return 0;
        }
        let adapter = TrayAdapter::new(TrayService::new());
        GLOBAL.with(|global| global.replace(Some(adapter)));
        0
    }))
    .unwrap_or(-1)
}
#[unsafe(no_mangle)]
pub extern "C" fn status_notifier_service_get_global() -> *mut glib::gobject_ffi::GObject {
    catch_unwind(AssertUnwindSafe(|| {
        GLOBAL.with(|global| {
            global
                .borrow()
                .as_ref()
                .map_or(std::ptr::null_mut(), |adapter| adapter.as_ptr().cast())
        })
    }))
    .unwrap_or(std::ptr::null_mut())
}
/// # Safety
/// The service is null or a live borrowed adapter. The returned hash and records
/// are borrowed; they must not be mutated or released by the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn status_notifier_service_get_items(
    service: *mut glib::gobject_ffi::GObject,
) -> *mut ffi::GHashTable {
    catch_unwind(AssertUnwindSafe(|| {
        if service.is_null() {
            return std::ptr::null_mut();
        }
        let object: Borrowed<glib::Object> = unsafe { from_glib_borrow(service) };
        object
            .downcast_ref::<TrayAdapter>()
            .map_or(std::ptr::null_mut(), |adapter| adapter.imp().table.0)
    }))
    .unwrap_or(std::ptr::null_mut())
}
unsafe fn record<'a>(item: *mut ItemRecord) -> Option<&'a Record> {
    if item.is_null() {
        None
    } else {
        Some(unsafe { &*item.cast::<Record>() })
    }
}
macro_rules! getter {
    ($name:ident, $ty:ty, $fallback:expr, $get:expr) => {
        /// # Safety
        /// The item is null or a live record borrowed on the application thread.
        /// Pointer results remain borrowed until the next update/removal signal.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(item: *mut ItemRecord) -> $ty {
            catch_unwind(AssertUnwindSafe(|| {
                unsafe { record(item) }.map_or($fallback, $get)
            }))
            .unwrap_or($fallback)
        }
    };
}
macro_rules! field_getter {
    ($name:ident, $field:ident, $ty:ty, $fallback:expr) => {
        getter!($name, $ty, $fallback, |record: &Record| unsafe {
            (*record.value.get()).$field
        });
    };
}
field_getter!(
    status_notifier_item_get_category,
    category,
    *const c_char,
    std::ptr::null()
);
field_getter!(
    status_notifier_item_get_id,
    id,
    *const c_char,
    std::ptr::null()
);
field_getter!(
    status_notifier_item_get_title,
    title,
    *const c_char,
    std::ptr::null()
);
field_getter!(
    status_notifier_item_get_status,
    status,
    *const c_char,
    std::ptr::null()
);
getter!(
    status_notifier_item_get_window_id,
    i32,
    0,
    |record: &Record| unsafe { (*record.value.get()).window_id as i32 }
);
field_getter!(
    status_notifier_item_get_icon_name,
    icon_name,
    *const c_char,
    std::ptr::null()
);
field_getter!(
    status_notifier_item_get_icon_pixmap,
    icon_pixmap,
    *mut gtk::gdk_pixbuf::ffi::GdkPixbuf,
    std::ptr::null_mut()
);
field_getter!(
    status_notifier_item_get_overlay_icon_name,
    overlay_icon_name,
    *const c_char,
    std::ptr::null()
);
field_getter!(
    status_notifier_item_get_overlay_icon_pixmap,
    overlay_icon_pixmap,
    *mut gtk::gdk_pixbuf::ffi::GdkPixbuf,
    std::ptr::null_mut()
);
field_getter!(
    status_notifier_item_get_attention_icon_name,
    attention_icon_name,
    *const c_char,
    std::ptr::null()
);
field_getter!(
    status_notifier_item_get_attention_icon_pixmap,
    attention_icon_pixmap,
    *mut gtk::gdk_pixbuf::ffi::GdkPixbuf,
    std::ptr::null_mut()
);
field_getter!(
    status_notifier_item_get_attention_movie_name,
    attention_movie_name,
    *const c_char,
    std::ptr::null()
);
getter!(
    status_notifier_item_get_key,
    *const c_char,
    std::ptr::null(),
    |record: &Record| record.canonical.as_ptr()
);
getter!(
    status_notifier_item_get_menu,
    *const c_char,
    std::ptr::null(),
    |record: &Record| record.metadata.borrow().menu.as_ptr()
);
getter!(
    status_notifier_item_get_item_is_menu,
    i32,
    0,
    |record: &Record| i32::from(record.item.borrow().item_is_menu)
);

fn item_command(item: *mut ItemRecord, command: ItemCommand) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(record) = (unsafe { record(item) }) else {
            return;
        };
        if let Some(adapter) = record.adapter.upgrade() {
            adapter.command(&record.key, command);
        }
    }));
}
macro_rules! coordinate_command {
    ($name:ident, $variant:ident) => {
        /// # Safety
        /// The item is null or a live borrowed record on its GLib thread.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(item: *mut ItemRecord, x: i32, y: i32) {
            item_command(item, ItemCommand::$variant { x, y });
        }
    };
}
coordinate_command!(status_notifier_item_activate, Activate);
coordinate_command!(status_notifier_item_secondary_activate, SecondaryActivate);
coordinate_command!(status_notifier_item_context_menu, ContextMenu);
/// # Safety
/// The item is null or a live borrowed record. Nonzero horizontal selects that
/// axis; zero selects vertical, matching the service's typed orientation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn status_notifier_item_scroll(
    item: *mut ItemRecord,
    delta: i32,
    horizontal: i32,
) {
    item_command(
        item,
        ItemCommand::Scroll {
            delta,
            orientation: if horizontal != 0 {
                Orientation::Horizontal
            } else {
                Orientation::Vertical
            },
        },
    );
}
/// # Safety
/// The item is null or a live borrowed record on its owning GLib main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn status_notifier_item_about_to_show(item: *mut ItemRecord, id: i32) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let service = unsafe { record(item) }.and_then(|record| {
            record
                .menu
                .borrow()
                .as_ref()
                .map(|menu| menu.service.clone())
        });
        if let Some(service) = service {
            service.about_to_show(id, |result| {
                if let Err(error) = result {
                    glib::g_message!("way-shell", "Tray menu AboutToShow failed: {error}");
                }
            });
        }
    }));
}

#[cfg(test)]
#[path = "../../shell/tests/common/tray.rs"]
mod fixture;

#[cfg(test)]
#[path = "../../shell/tests/common/tray_menu.rs"]
mod menu_fixture;

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        ffi::CStr,
        mem::{offset_of, size_of},
        sync::atomic::{AtomicU64, Ordering},
    };
    use way_shell::services::tray::{Icon, Tooltip};

    fn item(path: &str) -> TrayItem {
        TrayItem {
            key: ItemKey {
                owner: ":1.42".into(),
                path: path.into(),
            },
            category: "ApplicationStatus".into(),
            id: "fixture".into(),
            title: "First".into(),
            status: "Active".into(),
            window_id: u32::MAX,
            icon_theme_path: String::new(),
            icon: Icon {
                name: String::new(),
                pixmap: Some(RgbaImage {
                    width: 1,
                    height: 1,
                    pixels: vec![10, 20, 30, 128],
                }),
            },
            overlay: Icon {
                name: "overlay".into(),
                pixmap: Some(RgbaImage {
                    width: 1,
                    height: 1,
                    pixels: vec![90, 80, 70, 255],
                }),
            },
            attention: Icon::default(),
            attention_movie_name: String::new(),
            tooltip: Tooltip::default(),
            item_is_menu: false,
            menu_path: None,
            label: String::new(),
        }
    }
    fn lookup(adapter: &TrayAdapter, key: &str) -> *mut ItemRecord {
        let key = string(key);
        unsafe { ffi::g_hash_table_lookup(adapter.imp().table.0, key.as_ptr().cast()).cast() }
    }
    fn text(pointer: *const c_char) -> String {
        assert!(!pointer.is_null());
        unsafe { CStr::from_ptr(pointer).to_str().unwrap().to_owned() }
    }
    fn push(adapter: &TrayAdapter, event: TrayEvent) {
        adapter.push(Event::Service(Box::new(event)));
    }
    fn count(adapter: &TrayAdapter) -> u32 {
        unsafe { ffi::g_hash_table_size(adapter.imp().table.0) }
    }

    fn node(id: i32, label: &str, children: Vec<MenuNode>) -> MenuNode {
        MenuNode {
            id,
            label: label.into(),
            visible: true,
            separator: false,
            children,
        }
    }
    fn assert_action(
        model: &impl IsA<gio::MenuModel>,
        index: i32,
        label: &str,
        key: &str,
        id: i32,
    ) {
        assert_eq!(
            model
                .item_attribute_value(index, "label", None)
                .unwrap()
                .get::<String>()
                .unwrap(),
            label
        );
        assert_eq!(
            model
                .item_attribute_value(index, "action", None)
                .unwrap()
                .get::<String>()
                .unwrap(),
            "sni.item-clicked"
        );
        assert_eq!(
            model
                .item_attribute_value(index, "target", None)
                .unwrap()
                .get::<(String, i32)>()
                .unwrap(),
            (key.into(), id)
        );
    }
    #[test]
    fn menu_model_preserves_labels_sections_submenus_and_canonical_targets() {
        let mut hidden = node(2, "Hidden", vec![]);
        hidden.visible = false;
        let mut separator = node(3, "", vec![]);
        separator.separator = true;
        let root = node(
            0,
            "",
            vec![
                node(1, "_Open", vec![]),
                hidden.clone(),
                separator.clone(),
                node(4, "Submenu", vec![node(5, "Child", vec![])]),
                MenuNode {
                    id: 6,
                    ..separator.clone()
                },
                node(7, "Quit", vec![]),
            ],
        );
        let key = ":1.42/First";
        let menu = build_menu(&root, key);
        assert_eq!(menu.n_items(), 3);
        assert_action(&menu, 0, "_Open", key, 1);
        let section = menu.item_link(1, "section").unwrap();
        assert_eq!(section.n_items(), 1);
        assert_action(&section, 0, "Submenu", key, 4);
        let submenu = section.item_link(0, "submenu").unwrap();
        assert_eq!(submenu.n_items(), 1);
        assert_action(&submenu, 0, "Child", key, 5);
        let section = menu.item_link(2, "section").unwrap();
        assert_action(&section, 0, "Quit", key, 7);

        let root = node(
            0,
            "",
            vec![
                separator.clone(),
                separator,
                node(8, "Hidden children", vec![hidden]),
            ],
        );
        let menu = build_menu(&root, key);
        assert_eq!(
            menu.n_items(),
            2,
            "consecutive separators retain empty sections"
        );
        assert_eq!(menu.item_link(0, "section").unwrap().n_items(), 0);
        let section = menu.item_link(1, "section").unwrap();
        let submenu = section.item_link(0, "submenu").unwrap();
        assert_eq!(
            submenu.n_items(),
            0,
            "invisible-only children still create an empty submenu"
        );
    }

    #[test]
    fn menu_snapshot_publication_retains_records_and_orders_nested_removal() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let adapter: TrayAdapter = glib::Object::new();
                let initial = item("/First");
                push(&adapter, TrayEvent::Added(initial.clone()));
                let record = adapter
                    .imp()
                    .records
                    .borrow()
                    .values()
                    .next()
                    .unwrap()
                    .clone();
                let service: MenuService = glib::Object::new();
                let revision = record.menu_revision.get();
                record.menu.replace(Some(OwnedMenu {
                    actions: menu_actions(&record, &service, revision),
                    service,
                    handler: None,
                    model: None,
                }));
                record.write_fields();
                let log = Rc::new(RefCell::new(Vec::new()));
                let weak_adapter = adapter.downgrade();
                let removed = initial.clone();
                adapter.connect_local("status-notifier-item-menu-will-update", false, move |_| {
                    push(
                        &weak_adapter.upgrade().unwrap(),
                        TrayEvent::Removed(removed.clone()),
                    );
                    None
                });
                for (signal, index) in [
                    ("status-notifier-item-menu-will-update", 1),
                    ("status-notifier-item-menu-updated", 1),
                    ("status-notifier-item-removed", 2),
                ] {
                    let log = log.clone();
                    let weak = adapter.downgrade();
                    adapter.connect_local(signal, false, move |values| {
                        let pointer = values[index]
                            .get::<*mut c_void>()
                            .unwrap()
                            .cast::<ItemRecord>();
                        let adapter = weak.upgrade().unwrap();
                        let current = lookup(&adapter, ":1.42/First") == pointer;
                        let title = unsafe { text(status_notifier_item_get_title(pointer)) };
                        let model_exists = unsafe { !(*pointer).menu_model.is_null() };
                        log.borrow_mut()
                            .push((signal, current, title, model_exists));
                        if index == 2 {
                            let record = adapter
                                .imp()
                                .records
                                .borrow()
                                .values()
                                .next()
                                .unwrap()
                                .clone();
                            adapter.push(Event::Menu {
                                revision: record.menu_revision.get(),
                                record,
                                state: MenuState::default(),
                            });
                        }
                        None
                    });
                }
                let weak = Rc::downgrade(&record);
                adapter.push(Event::Menu {
                    record,
                    revision,
                    state: MenuState {
                        available: true,
                        revision: 1,
                        root: Some(node(0, "", vec![node(1, "Open", vec![])])),
                    },
                });
                assert_eq!(
                    &*log.borrow(),
                    &[
                        (
                            "status-notifier-item-menu-will-update",
                            true,
                            "First".into(),
                            false
                        ),
                        (
                            "status-notifier-item-menu-updated",
                            true,
                            "First".into(),
                            true
                        ),
                        ("status-notifier-item-removed", true, "First".into(), true),
                    ]
                );
                assert_eq!(count(&adapter), 0);
                assert!(
                    weak.upgrade().is_none(),
                    "queued obsolete menu event releases its record"
                );
            })
            .unwrap();
    }

    #[test]
    fn explicit_disposal_during_menu_publication_keeps_borrows_and_skips_swap() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let adapter: TrayAdapter = glib::Object::new();
                push(&adapter, TrayEvent::Added(item("/First")));
                let record = adapter
                    .imp()
                    .records
                    .borrow()
                    .values()
                    .next()
                    .unwrap()
                    .clone();
                let service: MenuService = glib::Object::new();
                let revision = record.menu_revision.get();
                record.menu.replace(Some(OwnedMenu {
                    actions: menu_actions(&record, &service, revision),
                    service,
                    handler: None,
                    model: None,
                }));
                record.write_fields();
                let weak = adapter.downgrade();
                let seen = Rc::new(Cell::new(false));
                let observed = seen.clone();
                adapter.connect_local(
                    "status-notifier-item-menu-will-update",
                    false,
                    move |values| {
                        let pointer = values[1].get::<*mut c_void>().unwrap().cast::<ItemRecord>();
                        unsafe {
                            weak.upgrade().unwrap().run_dispose();
                        }
                        // GObject disposal disconnects subsequent observers.
                        // The observer already running retains its item borrow.
                        observed.set(unsafe {
                            text(status_notifier_item_get_id(pointer)) == "fixture"
                        });
                        None
                    },
                );
                let updated = Rc::new(Cell::new(false));
                let changed = updated.clone();
                adapter.connect_local("status-notifier-item-menu-updated", false, move |_| {
                    changed.set(true);
                    None
                });
                let weak = Rc::downgrade(&record);
                adapter.push(Event::Menu {
                    record,
                    revision,
                    state: MenuState {
                        available: true,
                        revision: 1,
                        root: Some(node(0, "", vec![node(1, "Open", vec![])])),
                    },
                });
                assert!(seen.get());
                assert!(!updated.get());
                assert_eq!(count(&adapter), 0);
                assert!(weak.upgrade().is_none());
            })
            .unwrap();
    }

    fn set_menu(item: &fixture::Item, path: &str) {
        item.values.borrow_mut().insert(
            "Menu".into(),
            glib::variant::ObjectPath::try_from(path)
                .unwrap()
                .to_variant(),
        );
        item.properties_changed();
    }
    fn model(pointer: *mut ItemRecord) -> Option<gio::Menu> {
        unsafe { from_glib_none((*pointer).menu_model.cast::<gio::ffi::GMenu>()) }
    }
    fn actions(pointer: *mut ItemRecord) -> gio::SimpleActionGroup {
        unsafe {
            from_glib_none(
                (*pointer)
                    .action_group
                    .cast::<gio::ffi::GSimpleActionGroup>(),
            )
        }
    }
    #[test]
    fn private_bus_menus_route_actions_refresh_and_cancel_replaced_endpoints() {
        let (_bus, address) = menu_fixture::Bus::start();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let client = menu_fixture::connect(&address);
                let service = TrayService::on_connection(&menu_fixture::connect(&address));
                let adapter = TrayAdapter::new(service.clone());
                menu_fixture::wait(&context, || service.state().available);
                let first = fixture::Item::new(&client, "/First", "first");
                let second = fixture::Item::new(&client, "/Second", "second");
                let first_menu = menu_fixture::Menu::new(&client, "/FirstMenu");
                let second_menu = menu_fixture::Menu::new(&client, "/SecondMenu");
                set_menu(&first, "/FirstMenu");
                set_menu(&second, "/SecondMenu");
                fixture::register(&context, &client, "/First").unwrap();
                fixture::register(&context, &client, "/Second").unwrap();
                menu_fixture::wait(&context, || count(&adapter) == 2);
                let owner = client.unique_name().unwrap();
                let first_key = format!("{owner}/First");
                let second_key = format!("{owner}/Second");
                let first_pointer = lookup(&adapter, &first_key);
                let second_pointer = lookup(&adapter, &second_key);
                menu_fixture::wait(&context, || {
                    model(first_pointer).is_some() && model(second_pointer).is_some()
                });
                let old_actions = actions(first_pointer);
                let old_model = model(first_pointer).unwrap();
                let old_service = adapter
                    .imp()
                    .records
                    .borrow()
                    .get(&first_key)
                    .unwrap()
                    .menu
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .service
                    .downgrade();
                assert_action(&old_model, 0, "_Open", &first_key, 1);
                let before = (glib::real_time() / 1_000_000) as u32;
                old_actions.activate_action(
                    "item-clicked",
                    Some(&(first_key.as_str(), 1_i32).to_variant()),
                );
                old_actions.activate_action(
                    "item-clicked",
                    Some(&(second_key.as_str(), 1_i32).to_variant()),
                );
                actions(second_pointer).activate_action(
                    "item-clicked",
                    Some(&(second_key.as_str(), 7_i32).to_variant()),
                );
                menu_fixture::wait(&context, || {
                    first_menu
                        .requests
                        .borrow()
                        .iter()
                        .any(|(method, _)| method == "Event")
                        && second_menu
                            .requests
                            .borrow()
                            .iter()
                            .any(|(method, _)| method == "Event")
                });
                let events: Vec<_> = first_menu
                    .requests
                    .borrow()
                    .iter()
                    .filter(|(method, _)| method == "Event")
                    .cloned()
                    .collect();
                assert_eq!(
                    events.len(),
                    1,
                    "a retained group cannot target a different item"
                );
                assert_eq!(events[0].1.type_().as_str(), "(isvu)");
                let (id, event, data, timestamp) = events[0]
                    .1
                    .get::<(i32, String, glib::Variant, u32)>()
                    .unwrap();
                assert_eq!(
                    (id, event.as_str(), data.get::<i32>()),
                    (1, "clicked", Some(0))
                );
                assert!(timestamp >= before && timestamp <= (glib::real_time() / 1_000_000) as u32);

                first_menu.layout.replace(menu_fixture::layout("Updated"));
                first_menu.needs_update.set(true);
                unsafe { status_notifier_item_about_to_show(first_pointer, 0) };
                menu_fixture::wait(&context, || {
                    model(first_pointer)
                        .unwrap()
                        .item_attribute_value(0, "label", None)
                        .unwrap()
                        .get::<String>()
                        .as_deref()
                        == Some("Updated")
                });
                let about = first_menu
                    .requests
                    .borrow()
                    .iter()
                    .find(|(method, _)| method == "AboutToShow")
                    .unwrap()
                    .1
                    .clone();
                assert_eq!(about.get::<(i32,)>(), Some((0,)));

                set_menu(&first, "/SecondMenu");
                menu_fixture::wait(&context, || {
                    lookup(&adapter, &first_key) == first_pointer
                        && model(first_pointer).is_some_and(|menu| {
                            menu.item_attribute_value(0, "label", None)
                                .unwrap()
                                .get::<String>()
                                .as_deref()
                                == Some("_Open")
                        })
                });
                assert!(old_service.upgrade().is_none());
                assert!(!old_actions.is_action_enabled("item-clicked"));
                let old_events = first_menu.requests.borrow().len();
                old_actions.activate_action(
                    "item-clicked",
                    Some(&(first_key.as_str(), 1_i32).to_variant()),
                );
                context.block_on(glib::timeout_future(Duration::from_millis(20)));
                assert_eq!(first_menu.requests.borrow().len(), old_events);
                assert_action(&old_model, 0, "_Open", &first_key, 1);
                set_menu(&first, "/");
                menu_fixture::wait(&context, || model(first_pointer).is_none());
                unsafe {
                    assert!((*first_pointer).action_group.is_null());
                }
                service.stop();
                assert_eq!(count(&adapter), 0);
            })
            .unwrap();
    }

    #[test]
    fn stopping_live_service_in_first_menu_observer_keeps_later_borrows_valid() {
        let (_bus, address) = fixture::Bus::start();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let client = fixture::connect(&address);
                let service = TrayService::on_connection(&fixture::connect(&address));
                let adapter = TrayAdapter::new(service.clone());
                fixture::wait(&context, || service.state().available);
                let item = fixture::Item::new(&client, "/Item", "menu-owner");
                let _menu = menu_fixture::Menu::new(&client, "/Menu");
                set_menu(&item, "/Menu");
                let stop = service.downgrade();
                adapter.connect_local("status-notifier-item-menu-will-update", false, move |_| {
                    stop.upgrade().unwrap().stop();
                    None
                });
                let seen = Rc::new(RefCell::new(Vec::new()));
                for (signal, index) in [
                    ("status-notifier-item-menu-will-update", 1),
                    ("status-notifier-item-menu-updated", 1),
                    ("status-notifier-item-removed", 2),
                ] {
                    let seen = seen.clone();
                    let weak = adapter.downgrade();
                    adapter.connect_local(signal, false, move |values| {
                        let pointer = values[index]
                            .get::<*mut c_void>()
                            .unwrap()
                            .cast::<ItemRecord>();
                        let key = unsafe { text(status_notifier_item_get_key(pointer)) };
                        seen.borrow_mut().push((
                            signal,
                            lookup(&weak.upgrade().unwrap(), &key) == pointer,
                            unsafe { text(status_notifier_item_get_id(pointer)) },
                        ));
                        None
                    });
                }
                fixture::register(&context, &client, "/Item").unwrap();
                fixture::wait(&context, || seen.borrow().len() == 3);
                assert_eq!(
                    &*seen.borrow(),
                    &[
                        (
                            "status-notifier-item-menu-will-update",
                            true,
                            "menu-owner".into()
                        ),
                        (
                            "status-notifier-item-menu-updated",
                            true,
                            "menu-owner".into()
                        ),
                        ("status-notifier-item-removed", true, "menu-owner".into()),
                    ]
                );
                assert_eq!(count(&adapter), 0);
            })
            .unwrap();
    }

    #[test]
    fn c_layout_retains_the_original_header_offsets() {
        assert_eq!(size_of::<ItemRecord>(), 168);
        assert_eq!(offset_of!(ItemRecord, proxy), 0);
        assert_eq!(offset_of!(ItemRecord, menu_proxy), 8);
        assert_eq!(offset_of!(ItemRecord, action_group), 16);
        assert_eq!(offset_of!(ItemRecord, menu_model), 24);
        assert_eq!(offset_of!(ItemRecord, bus_name), 32);
        assert_eq!(offset_of!(ItemRecord, obj_name), 40);
        assert_eq!(offset_of!(ItemRecord, register_service_name), 48);
        assert_eq!(offset_of!(ItemRecord, category), 56);
        assert_eq!(offset_of!(ItemRecord, title), 72);
        assert_eq!(offset_of!(ItemRecord, window_id), 88);
        assert_eq!(offset_of!(ItemRecord, icon_pixmap_from_theme), 96);
        assert_eq!(offset_of!(ItemRecord, icon_pixmap), 120);
        assert_eq!(offset_of!(ItemRecord, overlay_icon_pixmap), 136);
        assert_eq!(offset_of!(ItemRecord, attention_movie_name), 160);
        assert_eq!(offset_of!(Record, value), 0);
    }

    #[test]
    fn stable_records_distinct_paths_and_owned_icon_bytes() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let adapter: TrayAdapter = glib::Object::new();
                let mut first = item("/First");
                let second = item("/Second");
                push(&adapter, TrayEvent::Added(first.clone()));
                push(&adapter, TrayEvent::Added(second.clone()));
                assert_eq!(count(&adapter), 2);
                let pointer = lookup(&adapter, &first.key.registration());
                assert_ne!(pointer, lookup(&adapter, &second.key.registration()));
                let icon: Pixbuf =
                    unsafe { from_glib_none(status_notifier_item_get_icon_pixmap(pointer)) };
                let overlay: Pixbuf = unsafe {
                    from_glib_none(status_notifier_item_get_overlay_icon_pixmap(pointer))
                };
                assert_eq!(icon.read_pixel_bytes().as_ref(), [10, 20, 30, 128]);
                assert_eq!(overlay.read_pixel_bytes().as_ref(), [90, 80, 70, 255]);
                unsafe {
                    assert!((*pointer).proxy.is_null());
                    assert_eq!(text((*pointer).bus_name), ":1.42");
                    assert_eq!(text((*pointer).obj_name), "/First");
                    assert_eq!(text(status_notifier_item_get_key(pointer)), ":1.42/First");
                    assert_eq!(status_notifier_item_get_window_id(pointer) as u32, u32::MAX);
                }
                first.title = "Updated".into();
                first.icon.pixmap = None;
                first.overlay.pixmap = None;
                push(&adapter, TrayEvent::Changed(first.clone()));
                assert_eq!(lookup(&adapter, &first.key.registration()), pointer);
                unsafe {
                    assert_eq!(text(status_notifier_item_get_title(pointer)), "Updated");
                    assert!(status_notifier_item_get_icon_pixmap(pointer).is_null());
                }
                push(&adapter, TrayEvent::Removed(first));
                push(&adapter, TrayEvent::Removed(second));
                assert_eq!(count(&adapter), 0);
                drop(adapter);
                assert_eq!(icon.read_pixel_bytes().as_ref(), [10, 20, 30, 128]);
                assert_eq!(overlay.read_pixel_bytes().as_ref(), [90, 80, 70, 255]);
                for invalid in [
                    RgbaImage {
                        width: 0,
                        height: 1,
                        pixels: vec![],
                    },
                    RgbaImage {
                        width: u32::MAX,
                        height: u32::MAX,
                        pixels: vec![0; 4],
                    },
                    RgbaImage {
                        width: 2,
                        height: 2,
                        pixels: vec![0; 15],
                    },
                ] {
                    assert!(pixbuf(&invalid).is_none());
                }
            })
            .unwrap();
    }

    #[test]
    fn nested_updates_keep_records_and_hash_entries_valid_for_all_observers() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let adapter: TrayAdapter = glib::Object::new();
                let initial = item("/First");
                push(&adapter, TrayEvent::Added(initial.clone()));
                let old = adapter
                    .imp()
                    .records
                    .borrow()
                    .values()
                    .next()
                    .unwrap()
                    .clone();
                let log = Rc::new(RefCell::new(Vec::new()));
                let weak = adapter.downgrade();
                let mut next = initial.clone();
                next.title = "Replacement".into();
                let removed = initial.clone();
                let once = Cell::new(false);
                adapter.connect_local(
                    "status-notifier-item-properties-changed",
                    false,
                    move |_| {
                        if !once.replace(true) {
                            let adapter = weak.upgrade().unwrap();
                            push(&adapter, TrayEvent::Removed(removed.clone()));
                            push(&adapter, TrayEvent::Added(next.clone()));
                        }
                        None
                    },
                );
                for (signal, pointer_index) in [
                    ("status-notifier-item-properties-changed", 1),
                    ("status-notifier-item-removed", 2),
                    ("status-notifier-item-added", 2),
                ] {
                    let seen = log.clone();
                    let weak = adapter.downgrade();
                    adapter.connect_local(signal, false, move |values| {
                        let pointer = values[pointer_index]
                            .get::<*mut c_void>()
                            .unwrap()
                            .cast::<ItemRecord>();
                        let adapter = weak.upgrade().unwrap();
                        unsafe {
                            let key = text(status_notifier_item_get_key(pointer));
                            assert_eq!(
                                lookup(&adapter, &key),
                                pointer,
                                "record remains in the public hash throughout callbacks"
                            );
                            if pointer_index == 2 {
                                let table = glib::gobject_ffi::g_value_get_boxed(
                                    values[1].to_glib_none().0,
                                );
                                assert_eq!(table, adapter.imp().table.0.cast());
                            }
                            seen.borrow_mut()
                                .push((signal, text(status_notifier_item_get_title(pointer))));
                        }
                        None
                    });
                }
                let mut changed = initial;
                changed.title = "Changed".into();
                push(&adapter, TrayEvent::Changed(changed));
                assert_eq!(
                    &*log.borrow(),
                    &[
                        ("status-notifier-item-properties-changed", "Changed".into()),
                        ("status-notifier-item-removed", "Changed".into()),
                        ("status-notifier-item-added", "Replacement".into()),
                    ]
                );
                assert_ne!(lookup(&adapter, ":1.42/First"), old.pointer());
            })
            .unwrap();
    }

    #[test]
    fn weak_subscriptions_cleanup_and_stopped_global_lifetime() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                for _ in 0..10 {
                    let service: TrayService = glib::Object::new();
                    let weak_service = service.downgrade();
                    let adapter = TrayAdapter::new(service);
                    let weak = adapter.downgrade();
                    drop(adapter);
                    assert!(weak.upgrade().is_none());
                    assert!(weak_service.upgrade().is_none());
                }
                let service: TrayService = glib::Object::new();
                let adapter = TrayAdapter::new(service);
                let pointer = adapter.as_ptr().cast();
                GLOBAL.with(|global| global.replace(Some(adapter)));
                shutdown();
                shutdown();
                assert_eq!(status_notifier_service_get_global(), pointer);
                unsafe {
                    assert_eq!(
                        ffi::g_hash_table_size(status_notifier_service_get_items(pointer)),
                        0
                    )
                };
                GLOBAL.with(|global| global.borrow_mut().take());
                assert!(status_notifier_service_get_global().is_null());
            })
            .unwrap();
    }

    #[test]
    fn private_bus_items_c_commands_and_owner_loss() {
        let (_bus, address) = fixture::Bus::start();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let client = fixture::connect(&address);
                let service = TrayService::on_connection(&fixture::connect(&address));
                let adapter = TrayAdapter::new(service.clone());
                fixture::wait(&context, || service.state().available);
                let first = fixture::Item::new(&client, "/First", "first");
                let second = fixture::Item::new(&client, "/Second", "second");
                first.values.borrow_mut().insert(
                    "IconPixmap".into(),
                    vec![(1_i32, 1_i32, vec![128_u8, 1, 2, 3])].to_variant(),
                );
                fixture::register(&context, &client, "/First").unwrap();
                fixture::register(&context, &client, "/Second").unwrap();
                fixture::wait(&context, || count(&adapter) == 2);
                let owner = client.unique_name().unwrap();
                let first_pointer = lookup(&adapter, &format!("{owner}/First"));
                let second_pointer = lookup(&adapter, &format!("{owner}/Second"));
                assert!(!first_pointer.is_null() && !second_pointer.is_null());
                unsafe {
                    status_notifier_item_activate(first_pointer, 12, 13);
                    status_notifier_item_secondary_activate(first_pointer, 14, 15);
                    status_notifier_item_context_menu(second_pointer, 16, 17);
                    status_notifier_item_scroll(second_pointer, -120, 0);
                }
                fixture::wait(&context, || {
                    first.actions.borrow().len() == 2 && second.actions.borrow().len() == 2
                });
                assert_eq!(first.actions.borrow()[0].0, "Activate");
                assert_eq!(
                    first.actions.borrow()[0].1.get::<(i32, i32)>(),
                    Some((12, 13))
                );
                assert_eq!(first.actions.borrow()[1].0, "SecondaryActivate");
                assert_eq!(second.actions.borrow()[0].0, "ContextMenu");
                assert_eq!(
                    second.actions.borrow()[1].1.get::<(i32, String)>(),
                    Some((-120, "vertical".into()))
                );
                first
                    .values
                    .borrow_mut()
                    .insert("Title".into(), "Updated over D-Bus".to_variant());
                first.signal("NewTitle", None);
                fixture::wait(&context, || unsafe {
                    text(status_notifier_item_get_title(first_pointer)) == "Updated over D-Bus"
                });
                assert_eq!(lookup(&adapter, &format!("{owner}/First")), first_pointer);
                drop(first);
                drop(second);
                client.close_sync(gio::Cancellable::NONE).unwrap();
                fixture::wait(&context, || count(&adapter) == 0);
                drop(service);
                let weak = adapter.downgrade();
                drop(adapter);
                assert!(weak.upgrade().is_none());
            })
            .unwrap();
    }

    #[test]
    fn asynchronous_theme_updates_ignore_stale_generations_and_release_owners() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "way-shell-tray-icons-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let image = pixbuf(&RgbaImage {
            width: 1,
            height: 1,
            pixels: vec![5, 6, 7, 255],
        })
        .unwrap();
        std::fs::write(
            directory.join("fixture.png"),
            image.save_to_bufferv("png", &[]).unwrap(),
        )
        .unwrap();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let adapter: TrayAdapter = glib::Object::new();
                let mut item = item("/First");
                item.icon.name = "fixture".into();
                item.icon_theme_path = directory.to_str().unwrap().into();
                push(&adapter, TrayEvent::Added(item.clone()));
                let record = adapter
                    .imp()
                    .records
                    .borrow()
                    .values()
                    .next()
                    .unwrap()
                    .clone();
                fixture::wait(&context, || unsafe {
                    !(*record.pointer()).icon_pixmap_from_theme.is_null()
                });
                let revision = record.theme_revision.get();
                item.icon_theme_path.clear();
                push(&adapter, TrayEvent::Changed(item.clone()));
                adapter.push(Event::Theme {
                    record: Rc::downgrade(&record),
                    revision,
                    image: Some(image.clone()),
                });
                unsafe {
                    assert!((*record.pointer()).icon_pixmap_from_theme.is_null());
                }
                item.icon_theme_path = directory.to_str().unwrap().into();
                item.icon.name = "fixture.png".into();
                push(&adapter, TrayEvent::Changed(item.clone()));
                fixture::wait(&context, || unsafe {
                    !(*record.pointer()).icon_pixmap_from_theme.is_null()
                });
                push(&adapter, TrayEvent::Removed(item.clone()));
                push(&adapter, TrayEvent::Added(item));
                adapter.push(Event::Theme {
                    record: Rc::downgrade(&record),
                    revision: record.theme_revision.get(),
                    image: Some(image),
                });
                let new_record = adapter
                    .imp()
                    .records
                    .borrow()
                    .values()
                    .next()
                    .unwrap()
                    .clone();
                assert!(!Rc::ptr_eq(&record, &new_record));
                let weak = adapter.downgrade();
                drop(adapter);
                assert!(weak.upgrade().is_none());
                context.block_on(glib::timeout_future(Duration::from_millis(30)));
                assert!(new_record.theme_task.borrow().is_none());
            })
            .unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn global_initialization_respects_disabled_setting() {
        const CHILD: &str = "WAY_SHELL_TRAY_GLOBAL_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let context = glib::MainContext::new();
            context
                .with_thread_default(|| {
                    let settings = gio::Settings::new("org.ldelossa.way-shell.panel");
                    settings.set_boolean("enable-tray-icons", false).unwrap();
                    assert_eq!(status_notifier_service_global_init(), 0);
                    assert!(status_notifier_service_get_global().is_null());
                    settings.set_boolean("enable-tray-icons", true).unwrap();
                    assert_eq!(status_notifier_service_global_init(), 0);
                    let adapter = GLOBAL.with(|global| global.borrow().clone()).unwrap();
                    let service = adapter.imp().service.borrow().clone().unwrap();
                    fixture::wait(&context, || service.state().available);
                    assert_eq!(status_notifier_service_global_init(), 0);
                    assert_eq!(
                        status_notifier_service_get_global(),
                        adapter.as_ptr().cast()
                    );
                    shutdown();
                    assert!(!service.state().available);
                    assert_eq!(count(&adapter), 0);
                    GLOBAL.with(|global| global.borrow_mut().take());
                })
                .unwrap();
            return;
        }
        let (_bus, address) = fixture::Bus::start();
        let directory =
            std::env::temp_dir().join(format!("way-shell-tray-schema-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("org.ldelossa.way-shell.gschema.xml"),
            include_bytes!("../../../data/org.ldelossa.way-shell.gschema.xml"),
        )
        .unwrap();
        assert!(
            std::process::Command::new("glib-compile-schemas")
                .arg("--strict")
                .arg(&directory)
                .status()
                .unwrap()
                .success()
        );
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tray::tests::global_initialization_respects_disabled_setting",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("GSETTINGS_BACKEND", "memory")
            .env("GSETTINGS_SCHEMA_DIR", &directory)
            .env("DBUS_SESSION_BUS_ADDRESS", address)
            .output()
            .unwrap();
        std::fs::remove_dir_all(directory).unwrap();
        assert!(
            output.status.success(),
            "tray global fixture failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
