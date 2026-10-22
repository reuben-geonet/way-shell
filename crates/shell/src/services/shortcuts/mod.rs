//! Application-owned, lazy saved-configuration inventory. GTK never parses files.
pub mod discovery;
mod rows;
use super::wm::{Backend, WindowManager};
use gio::prelude::*;
pub use rows::{Row, rows};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};
use way_shell_shortcuts::{Compositor, Context, Session, Snapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Loading,
    Current,
    Incomplete,
    Stale,
    Unavailable,
}
#[derive(Clone, Debug)]
pub struct Data {
    pub status: Status,
    pub source: Option<PathBuf>,
    pub snapshot: Snapshot,
    pub rows: Vec<Row>,
    pub messages: Vec<String>,
    pub reload_failed: bool,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            status: Status::Loading,
            source: None,
            snapshot: Snapshot::default(),
            rows: Vec::new(),
            messages: Vec::new(),
            reload_failed: false,
        }
    }
}
type Changed = Rc<dyn Fn()>;
pub struct ShortcutsService {
    manager: WindowManager,
    settings: gio::Settings,
    context: Context,
    data: RefCell<Data>,
    complete: RefCell<Option<(PathBuf, Snapshot)>>,
    handlers: RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>,
    watches: RefCell<Vec<gio::FileMonitor>>,
    debounce: RefCell<Option<glib::SourceId>>,
    task: RefCell<Option<glib::JoinHandle<()>>>,
    callbacks: RefCell<BTreeMap<u64, Changed>>,
    next_callback: Cell<u64>,
    generation: Cell<u64>,
    dirty: Cell<bool>,
    busy: Cell<bool>,
    visible: Cell<bool>,
    stopped: Cell<bool>,
}
impl ShortcutsService {
    pub fn new(manager: WindowManager, settings: gio::Settings) -> Rc<Self> {
        Self::with_context(
            manager,
            settings,
            Context {
                home: Some(glib::home_dir()),
                environment: std::env::vars_os()
                    .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
                    .collect(),
                session: Session::Unknown,
            },
        )
    }
    pub fn with_context(
        manager: WindowManager,
        settings: gio::Settings,
        context: Context,
    ) -> Rc<Self> {
        let this = Rc::new(Self {
            manager,
            settings,
            context,
            data: RefCell::new(Data::default()),
            complete: RefCell::new(None),
            handlers: RefCell::new(Vec::new()),
            watches: RefCell::new(Vec::new()),
            debounce: RefCell::new(None),
            task: RefCell::new(None),
            callbacks: RefCell::new(BTreeMap::new()),
            next_callback: Cell::new(0),
            generation: Cell::new(0),
            dirty: Cell::new(true),
            busy: Cell::new(false),
            visible: Cell::new(false),
            stopped: Cell::new(false),
        });
        for signal in ["config-changed", "connection-changed"] {
            let weak = Rc::downgrade(&this);
            let handler = this.manager.connect_local(signal, false, move |_| {
                if let Some(this) = weak.upgrade() {
                    this.invalidate(false);
                }
                None
            });
            this.handlers
                .borrow_mut()
                .push((this.manager.clone().upcast(), handler));
        }
        for key in ["sway-config-path", "niri-config-path"] {
            let weak = Rc::downgrade(&this);
            let handler = this.settings.connect_changed(Some(key), move |_, _| {
                if let Some(this) = weak.upgrade() {
                    this.complete.borrow_mut().take();
                    this.data.replace(Data::default());
                    this.invalidate(false);
                    this.publish();
                }
            });
            this.handlers
                .borrow_mut()
                .push((this.settings.clone().upcast(), handler));
        }
        this
    }
    pub fn data(&self) -> Data {
        self.data.borrow().clone()
    }
    pub fn on_changed(&self, callback: impl Fn() + 'static) -> u64 {
        let id = self.next_callback.get();
        self.next_callback.set(id + 1);
        self.callbacks.borrow_mut().insert(id, Rc::new(callback));
        id
    }
    pub fn disconnect(&self, id: u64) {
        self.callbacks.borrow_mut().remove(&id);
    }
    pub fn set_visible(self: &Rc<Self>, visible: bool) {
        if self.stopped.get() {
            return;
        }
        self.visible.set(visible);
        if visible && self.dirty.get() {
            self.refresh();
        }
    }
    pub fn retry(self: &Rc<Self>) {
        self.invalidate(false);
    }
    fn publish(&self) {
        let callbacks: Vec<_> = self.callbacks.borrow().values().cloned().collect();
        for callback in callbacks {
            if self.stopped.get() {
                break;
            }
            callback();
        }
    }
    fn invalidate(self: &Rc<Self>, debounce: bool) {
        if self.stopped.get() {
            return;
        }
        self.generation.set(self.generation.get().wrapping_add(1));
        self.dirty.set(true);
        if let Some(source) = self.debounce.borrow_mut().take() {
            source.remove();
        }
        if !self.visible.get() {
            return;
        }
        if debounce {
            let weak = Rc::downgrade(self);
            self.debounce.replace(Some(glib::timeout_add_local_once(
                Duration::from_millis(150),
                move || {
                    if let Some(this) = weak.upgrade() {
                        this.debounce.borrow_mut().take();
                        this.refresh();
                    }
                },
            )));
        } else {
            self.refresh();
        }
    }
    fn refresh(self: &Rc<Self>) {
        if self.stopped.get() || self.busy.get() || !self.visible.get() {
            return;
        }
        self.busy.set(true);
        self.dirty.set(false);
        let generation = self.generation.get();
        let backend = self.manager.backend();
        let context = self.context.clone();
        let override_path = self
            .settings
            .string(if backend == Backend::Sway {
                "sway-config-path"
            } else {
                "niri-config-path"
            })
            .to_string();
        let sway_path = self.manager.loaded_config_path();
        let socket = self.manager.socket_path();
        let weak = Rc::downgrade(self);
        let task = glib::MainContext::ref_thread_default().spawn_local(async move {
            let result = gio::spawn_blocking(move || {
                let discovery = if backend == Backend::Niri {
                    let process = discovery::process_context(&socket);
                    discovery::niri_path(&override_path, process.as_ref(), &context)
                } else {
                    let mut diagnostics = Vec::new();
                    let path = if override_path.is_empty() { sway_path } else { discovery::explicit(&override_path, &context, &mut diagnostics) };
                    if path.is_none() && diagnostics.is_empty() { diagnostics.push("Sway has not reported its configuration path. Reconnect or set sway-config-path.".into()); }
                    discovery::Discovery { path, context, diagnostics, dependencies: Vec::new() }
                };
                let mut snapshot = discovery.path.as_ref().map(|path| way_shell_shortcuts::load(if backend == Backend::Sway { Compositor::Sway } else { Compositor::Niri }, path, &discovery.context)).unwrap_or_default();
                for path in discovery.dependencies { snapshot.files.insert(path.clone()); if let Some(parent) = path.parent() { snapshot.directories.insert(parent.into()); } }
                { let watches = watch_directories(&snapshot); (discovery.path, snapshot, discovery.diagnostics, watches) }
            }).await;
            let Some(this) = weak.upgrade().filter(|this| !this.stopped.get()) else { return; };
            this.task.borrow_mut().take(); this.busy.set(false);
            if this.generation.get() == generation {
                match result {
                    Ok((path, snapshot, messages, watches)) => {
                        this.watch(watches);
                        let data = reconcile(path, snapshot, messages, &mut this.complete.borrow_mut(), this.manager.config_reload_failed());
                        this.data.replace(data);
                    }
                    Err(_) => { this.data.borrow_mut().status = Status::Unavailable; this.data.borrow_mut().messages = vec!["Shortcut loading worker stopped. Retry to load again.".into()]; }
                }
                this.publish();
            }
            if this.dirty.get() && this.debounce.borrow().is_none() { this.refresh(); }
        });
        self.task.replace(Some(task));
    }
    fn watch(self: &Rc<Self>, directories: Vec<PathBuf>) {
        let previous = self.watches.take();
        for path in directories {
            if let Ok(watch) = gio::File::for_path(path)
                .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
            {
                let weak = Rc::downgrade(self);
                watch.connect_changed(move |_, _, _, _| {
                    if let Some(this) = weak.upgrade() {
                        this.invalidate(true);
                    }
                });
                self.watches.borrow_mut().push(watch);
            }
        }
        for watch in previous {
            watch.cancel();
        }
    }
    pub fn stop(&self) {
        if self.stopped.replace(true) {
            return;
        }
        self.generation.set(self.generation.get().wrapping_add(1));
        if let Some(task) = self.task.borrow_mut().take() {
            task.abort();
        }
        if let Some(source) = self.debounce.borrow_mut().take() {
            source.remove();
        }
        for watch in self.watches.take() {
            watch.cancel();
        }
        for (object, handler) in self.handlers.take() {
            object.disconnect(handler);
        }
        self.callbacks.borrow_mut().clear();
    }
}
impl Drop for ShortcutsService {
    fn drop(&mut self) {
        self.stop();
    }
}

fn reconcile(
    path: Option<PathBuf>,
    snapshot: Snapshot,
    messages: Vec<String>,
    complete: &mut Option<(PathBuf, Snapshot)>,
    reload_failed: bool,
) -> Data {
    if complete
        .as_ref()
        .is_some_and(|(old, _)| Some(old) != path.as_ref())
    {
        *complete = None;
    }
    let mut status = if !snapshot.root_readable {
        Status::Unavailable
    } else if snapshot.complete() {
        Status::Current
    } else {
        Status::Incomplete
    };
    let mut displayed = snapshot.clone();
    if status == Status::Current {
        if let Some(path) = &path {
            *complete = Some((path.clone(), snapshot));
        }
    } else if let Some((_, previous)) = complete {
        status = Status::Stale;
        displayed.bindings = previous.bindings.clone();
        displayed.mod_legend = previous.mod_legend;
    }
    let rows = rows(&displayed);
    Data {
        status,
        source: path,
        snapshot: displayed,
        rows,
        messages,
        reload_failed,
    }
}

// Called on the worker: even ancestor probing can block on a network filesystem.
fn watch_directories(snapshot: &Snapshot) -> Vec<PathBuf> {
    let mut directories = snapshot.directories.clone();
    for path in &snapshot.files {
        if let Some(parent) = path.parent() {
            directories.insert(parent.into());
        }
    }
    directories
        .into_iter()
        .filter_map(|path| path.ancestors().find(|p| p.is_dir()).map(Path::to_path_buf))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires an isolated Wayland compositor"]
    fn shortcut_loading_navigation_minimum_errors_and_reopening() {
        use crate::ui::shortcuts::ShortcutsSheet;
        use gtk::prelude::*;
        crate::ui::init_test_display();
        let context = glib::MainContext::default();
        let pause = |ms| context.block_on(glib::timeout_future(Duration::from_millis(ms)));
        let manager: WindowManager = glib::Object::new();
        let service = ShortcutsService::with_context(
            manager.clone(),
            gio::Settings::new("org.ldelossa.way-shell.window-manager"),
            Context::default(),
        );
        // Drive worker publications explicitly so timings do not depend on disk speed.
        service.dirty.set(false);
        let sheet = ShortcutsSheet::new(service.clone(), manager).unwrap();
        let display = gtk::gdk::Display::default().unwrap();
        let monitor = display
            .monitors()
            .item(0)
            .unwrap()
            .downcast::<gtk::gdk::Monitor>()
            .unwrap();
        fn labels(widget: &gtk::Widget) -> Vec<String> {
            let mut texts = Vec::new();
            // GTK only parents an expander's content while it is expanded.
            if let Some(expander) = widget.downcast_ref::<gtk::Expander>()
                && let Some(child) = expander.child()
            {
                texts.extend(labels(&child));
            }
            if let Some(label) = widget.downcast_ref::<gtk::Label>()
                && label.opacity() > 0.0
            {
                texts.push(label.text().to_string());
            }
            for child in std::iter::successors(widget.first_child(), |w| w.next_sibling()) {
                texts.extend(labels(&child));
            }
            texts
        }
        let texts = || labels(sheet.popup().window().upcast_ref());
        let loading = || texts().iter().any(|s| s.contains("Loading configured"));
        let navigation = || texts().iter().any(|s| s == "Inside Way-Shell");
        let snapshot = way_shell_shortcuts::parse_text(
            Compositor::Sway,
            Path::new("/config"),
            "bindsym Mod4+A focus left",
            &Context::default(),
        );
        let good = reconcile(Some("/config".into()), snapshot, vec![], &mut None, false);
        let publish = |data: Data| {
            service.data.replace(data);
            service.publish();
        };
        sheet.popup().show(&monitor).unwrap();
        assert!(navigation());
        assert!(!loading());
        pause(40);
        publish(good.clone());
        pause(320);
        assert!(!loading());
        assert!(
            texts()
                .iter()
                .any(|s| s == "A" || s.contains("focus left") || s.contains("Focus left"))
        );
        publish(Data::default());
        pause(320);
        assert!(loading());
        assert!(navigation());
        publish(good.clone());
        pause(100);
        assert!(loading());
        pause(420);
        assert!(!loading());
        publish(Data::default());
        pause(320);
        publish(Data {
            status: Status::Unavailable,
            messages: vec!["Immediate fixture failure".into()],
            ..Data::default()
        });
        assert!(!loading());
        assert!(texts().iter().any(|s| s == "Retry"));
        assert!(
            texts()
                .iter()
                .any(|s| s.contains("Immediate fixture failure"))
        );
        publish(Data::default());
        pause(320);
        sheet.hide();
        publish(good);
        pause(520);
        assert!(!sheet.popup().is_visible());
        sheet.popup().show(&monitor).unwrap();
        assert!(!loading());
        assert!(navigation());
        sheet.close();
        service.stop();
    }
    #[test]
    fn stale_only_for_the_same_source_and_recovers() {
        let path = PathBuf::from("/config");
        let good = way_shell_shortcuts::parse_text(
            Compositor::Sway,
            &path,
            "bindsym Mod4+A focus left",
            &Context::default(),
        );
        let bad = way_shell_shortcuts::parse_text(
            Compositor::Sway,
            &path,
            "mode resize {",
            &Context::default(),
        );
        let mut cache = None;
        assert_eq!(
            reconcile(Some(path.clone()), good.clone(), vec![], &mut cache, false).status,
            Status::Current
        );
        let stale = reconcile(Some(path.clone()), bad.clone(), vec![], &mut cache, true);
        assert_eq!(stale.status, Status::Stale);
        assert_eq!(stale.rows.len(), 1);
        assert!(stale.reload_failed);
        assert_eq!(
            reconcile(Some(path), good, vec![], &mut cache, false).status,
            Status::Current
        );
        let changed = reconcile(Some("/other".into()), bad, vec![], &mut cache, false);
        assert_eq!(changed.status, Status::Incomplete);
        assert!(changed.rows.is_empty());
        assert!(cache.is_none());
    }
}
