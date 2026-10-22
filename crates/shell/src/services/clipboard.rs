//! One history worker owns SQLite. Clipboard transport stays on the Wayland loop.
pub mod preview;
mod sound;
use super::{
    settings,
    wayland::{ClipboardOffer, Toplevel, WaylandService},
};
use gio::prelude::*;
pub use preview::Preview;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use way_shell_clipboard::{
    Content, Entry, HistoryPolicy, Kind, Retention, Store, actions::ActionConfig,
};

type Reply = Result<Vec<Entry>, String>;
type Operation = Box<dyn FnOnce(&mut Store) -> Reply + Send>;
struct Work {
    operation: Operation,
    reply: futures_channel::oneshot::Sender<Reply>,
}
struct Worker {
    sender: Option<mpsc::Sender<Work>>,
    thread: Option<std::thread::JoinHandle<()>>,
    policy: Arc<AtomicU8>,
}
impl Worker {
    fn new(
        path: Option<PathBuf>,
        policy: HistoryPolicy,
        previous: Option<Self>,
        preserve: bool,
    ) -> Self {
        let (sender, receiver) = mpsc::channel::<Work>();
        let policy = Arc::new(AtomicU8::new(policy as u8));
        let shutdown_policy = policy.clone();
        let thread = std::thread::spawn(move || {
            // Finish the old owner's shutdown transaction before reopening its database.
            if let Some(previous) = previous {
                if preserve {
                    previous
                        .policy
                        .store(HistoryPolicy::KeepAll as u8, Ordering::Release);
                }
                previous.finish();
            }
            let mut store = (|| {
                if let Some(path) = &path {
                    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
                    if let Some(parent) = path.parent() {
                        std::fs::DirBuilder::new()
                            .recursive(true)
                            .mode(0o700)
                            .create(parent)
                            .map_err(|e| e.to_string())?;
                    }
                    std::fs::OpenOptions::new()
                        .create(true)
                        .truncate(false)
                        .write(true)
                        .mode(0o600)
                        .open(path)
                        .map_err(|e| e.to_string())?;
                }
                let mut store = Store::open(path.as_deref())?;
                if !preserve {
                    store.clear(read_policy(&shutdown_policy))?;
                }
                Ok::<_, String>(store)
            })();
            while let Ok(work) = receiver.recv() {
                let result = match &mut store {
                    Ok(store) => (work.operation)(store),
                    Err(error) => Err(error.clone()),
                };
                let _ = work.reply.send(result);
            }
            if let Ok(store) = &mut store
                && let Err(error) = store.clear(read_policy(&shutdown_policy))
            {
                glib::g_warning!(
                    "way-shell",
                    "Could not apply clipboard shutdown policy: {error}"
                );
            }
        });
        Self {
            sender: Some(sender),
            thread: Some(thread),
            policy,
        }
    }
    fn finish(mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
    fn request(
        &self,
        operation: impl FnOnce(&mut Store) -> Reply + Send + 'static,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Reply>>> {
        let (reply, receiver) = futures_channel::oneshot::channel();
        let sent = self
            .sender
            .as_ref()
            .unwrap()
            .send(Work {
                operation: Box::new(operation),
                reply,
            })
            .is_ok();
        Box::pin(async move {
            if !sent {
                return Err("Clipboard worker has stopped".into());
            }
            receiver
                .await
                .map_err(|_| "Clipboard worker stopped unexpectedly".to_owned())?
        })
    }
}
fn read_policy(policy: &AtomicU8) -> HistoryPolicy {
    match policy.load(Ordering::Acquire) {
        0 => HistoryPolicy::Clear,
        2 => HistoryPolicy::KeepAll,
        _ => HistoryPolicy::KeepPinnedAndTagged,
    }
}
type Changed = Rc<dyn Fn()>;
pub struct ClipboardService {
    pub settings: gio::Settings,
    pub wayland: WaylandService,
    worker: RefCell<Option<Worker>>,
    entries: RefCell<Vec<Entry>>,
    error: RefCell<Option<String>>,
    ready: Cell<bool>,
    stopped: Cell<bool>,
    generation: Arc<AtomicU64>,
    backend_generation: Cell<u64>,
    transfers: RefCell<BTreeMap<u64, glib::JoinHandle<()>>>,
    handlers: RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>,
    callbacks: RefCell<BTreeMap<u64, Changed>>,
    next: Cell<u64>,
    revision: Cell<u64>,
    copied: Cell<u64>,
    copied_text: RefCell<Option<String>>,
    notifications: Option<super::notifications::NotificationsService>,
    sound: RefCell<Option<sound::Sound>>,
    retention_timer: RefCell<Option<glib::SourceId>>,
}
impl ClipboardService {
    pub fn new(
        wayland: WaylandService,
        notifications: super::notifications::NotificationsService,
    ) -> Result<Rc<Self>, String> {
        let settings =
            settings::open("org.ldelossa.way-shell.clipboard").map_err(|e| e.to_string())?;
        Ok(Self::with_settings(wayland, settings, Some(notifications)))
    }
    pub(crate) fn with_settings(
        wayland: WaylandService,
        settings: gio::Settings,
        notifications: Option<super::notifications::NotificationsService>,
    ) -> Rc<Self> {
        let this = Rc::new(Self {
            settings,
            wayland,
            worker: RefCell::new(None),
            entries: RefCell::new(Vec::new()),
            error: RefCell::new(None),
            ready: Cell::new(false),
            stopped: Cell::new(false),
            generation: Arc::new(AtomicU64::new(0)),
            backend_generation: Cell::new(0),
            transfers: RefCell::new(BTreeMap::new()),
            handlers: RefCell::new(Vec::new()),
            callbacks: RefCell::new(BTreeMap::new()),
            next: Cell::new(0),
            revision: Cell::new(0),
            copied: Cell::new(0),
            copied_text: RefCell::new(None),
            notifications,
            sound: RefCell::new(None),
            retention_timer: RefCell::new(None),
        });
        this.configure(false);
        let weak = Rc::downgrade(&this);
        let handler = this
            .wayland
            .connect_local("clipboard-offer", false, move |values| {
                if let Some(this) = weak.upgrade() {
                    this.capture(values[1].get::<ClipboardOffer>().unwrap());
                }
                None
            });
        this.handlers
            .borrow_mut()
            .push((this.wayland.clone().upcast(), handler));
        let weak = Rc::downgrade(&this);
        let handler = this.settings.connect_changed(None, move |_, key| {
            if let Some(this) = weak.upgrade() {
                if matches!(key, "incognito" | "wmclass-exclusions") {
                    this.cancel_transfers();
                }
                if matches!(key, "database-backend" | "database-location") {
                    this.configure(false);
                } else if key == "clipboard-history" {
                    if let Some(worker) = this.worker.borrow().as_ref() {
                        worker.policy.store(this.policy() as u8, Ordering::Release);
                    }
                } else if matches!(key, "history-length" | "history-time") {
                    this.watch_retention();
                    let retention = this.retention();
                    this.change(move |store| {
                        store.prune(retention, now())?;
                        store.entries()
                    });
                }
                this.revision.set(this.revision.get().wrapping_add(1));
                this.changed();
            }
        });
        this.handlers
            .borrow_mut()
            .push((this.settings.clone().upcast(), handler));
        for name in [
            "theme",
            "text-item",
            "code-item",
            "image-item",
            "file-item",
            "link-item",
            "character-item",
        ] {
            let child = this.settings.child(name);
            let weak = Rc::downgrade(&this);
            let handler = child.connect_changed(None, move |_, _| {
                if let Some(this) = weak.upgrade() {
                    this.revision.set(this.revision.get().wrapping_add(1));
                    this.changed();
                }
            });
            this.handlers.borrow_mut().push((child.upcast(), handler));
        }
        for offer in this.wayland.clipboard_offers() {
            this.capture(offer);
        }
        this.watch_retention();
        this
    }
    fn configure(self: &Rc<Self>, preserve: bool) {
        self.cancel_transfers();
        self.backend_generation
            .set(self.backend_generation.get().wrapping_add(1));
        self.ready.set(false);
        self.error.borrow_mut().take();
        if !preserve {
            self.entries.borrow_mut().clear();
        }
        let memory = self.settings.string("database-backend") == "memory";
        let location = self.settings.string("database-location");
        let path = (!memory).then(|| {
            if location.is_empty() {
                glib::user_data_dir().join("way-shell/clipboard/history.db")
            } else {
                PathBuf::from(location.as_str())
            }
        });
        let previous = self.worker.borrow_mut().take();
        self.worker
            .replace(Some(Worker::new(path, self.policy(), previous, preserve)));
        self.changed();
        self.change(|store| store.entries());
    }
    pub fn retry(self: &Rc<Self>) {
        self.configure(true);
    }
    pub fn prune(self: &Rc<Self>) {
        let retention = self.retention();
        self.change(move |store| {
            store.prune(retention, now())?;
            store.entries()
        });
    }
    fn watch_retention(self: &Rc<Self>) {
        if let Some(timer) = self.retention_timer.borrow_mut().take() {
            timer.remove();
        }
        if self.settings.int("history-time") == 0 {
            return;
        }
        let weak = Rc::downgrade(self);
        self.retention_timer
            .replace(Some(glib::timeout_add_seconds_local(60, move || {
                let Some(this) = weak.upgrade().filter(|this| !this.stopped.get()) else {
                    return glib::ControlFlow::Break;
                };
                this.prune();
                glib::ControlFlow::Continue
            })));
    }
    pub fn entries(&self) -> std::cell::Ref<'_, Vec<Entry>> {
        self.entries.borrow()
    }
    pub fn error(&self) -> Option<String> {
        self.error.borrow().clone()
    }
    pub fn ready(&self) -> bool {
        self.ready.get()
    }
    pub fn revision(&self) -> u64 {
        self.revision.get()
    }
    pub fn copied(&self) -> u64 {
        self.copied.get()
    }
    pub fn copied_text(&self) -> Option<String> {
        self.copied_text.borrow().clone()
    }
    pub fn entry(&self, id: i64) -> Option<Entry> {
        self.entries
            .borrow()
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
    }
    pub fn on_changed(&self, callback: impl Fn() + 'static) -> u64 {
        let id = self.next.get();
        self.next.set(id + 1);
        self.callbacks.borrow_mut().insert(id, Rc::new(callback));
        id
    }
    pub fn disconnect(&self, id: u64) {
        self.callbacks.borrow_mut().remove(&id);
    }
    fn changed(&self) {
        let callbacks = self
            .callbacks
            .borrow()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for callback in callbacks {
            if !self.stopped.get() {
                callback();
            }
        }
    }
    pub fn report(&self, error: String) {
        self.notify("Clipboard action failed", &error);
        self.error.replace(Some(error));
        self.changed();
    }
    fn notify(&self, title: &str, body: &str) {
        if let Some(notifications) = &self.notifications {
            let _ =
                notifications.send_internal(way_shell_core::notifications::NotificationRequest {
                    app_name: "Clipboard".into(),
                    app_icon: "edit-paste-symbolic".into(),
                    summary: title.into(),
                    body: body.into(),
                    expire_timeout: 4000,
                    ..Default::default()
                });
        }
    }
    fn feedback(self: &Rc<Self>, text: Option<&str>) {
        self.copied.set(self.copied.get().wrapping_add(1));
        let text = text
            .map(|text| text.chars().take(500).collect())
            .unwrap_or_else(|| {
                self.entries
                    .borrow()
                    .first()
                    .map(|entry| {
                        if entry.content.text.is_empty() {
                            entry.content.kind.name().to_owned()
                        } else {
                            entry.content.text.chars().take(500).collect()
                        }
                    })
                    .unwrap_or_default()
            });
        self.copied_text
            .replace(Some(text.chars().take(500).collect()));
        if self.settings.boolean("send-notification") {
            self.notify(
                "Copied to clipboard",
                &glib::markup_escape_text(&text.chars().take(200).collect::<String>()),
            );
        }
        let sound = self.settings.string("sound");
        if sound == "none" {
            return;
        }
        let mut player = self.sound.borrow_mut();
        let player = player.get_or_insert_with(sound::Sound::new);
        player.play(sound.to_string(), self.settings.double("volume"));
    }

    fn policy(&self) -> HistoryPolicy {
        match self.settings.string("clipboard-history").as_str() {
            "clear" => HistoryPolicy::Clear,
            "keep-all" => HistoryPolicy::KeepAll,
            _ => HistoryPolicy::KeepPinnedAndTagged,
        }
    }
    fn retention(&self) -> Retention {
        Retention {
            length: self.settings.int("history-length") as usize,
            minutes: self.settings.int("history-time") as u32,
        }
    }
    pub fn change(self: &Rc<Self>, operation: impl FnOnce(&mut Store) -> Reply + Send + 'static) {
        self.mutate(operation, false);
    }
    fn mutate(
        self: &Rc<Self>,
        operation: impl FnOnce(&mut Store) -> Reply + Send + 'static,
        copied: bool,
    ) {
        if self.stopped.get() {
            return;
        }
        let Some(future) = self
            .worker
            .borrow()
            .as_ref()
            .map(|worker| worker.request(operation))
        else {
            return;
        };
        let generation = self.backend_generation.get();
        let capture_generation = self.generation.load(Ordering::Acquire);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = future.await;
            if let Some(this) = weak
                .upgrade()
                .filter(|this| !this.stopped.get() && this.backend_generation.get() == generation)
            {
                this.ready.set(true);
                match result {
                    Ok(entries) => {
                        let changed = *this.entries.borrow() != entries;
                        this.entries.replace(entries);
                        this.error.borrow_mut().take();
                        if changed {
                            this.revision.set(this.revision.get().wrapping_add(1));
                        }
                        if copied && capture_generation == this.generation.load(Ordering::Acquire) {
                            this.feedback(None);
                        }
                    }
                    Err(error) => {
                        this.error.replace(Some(error));
                    }
                }
                this.changed();
            }
        });
    }
    fn capture(self: &Rc<Self>, offer: ClipboardOffer) {
        if self.stopped.get()
            || !should_record(
                self.settings.boolean("incognito"),
                &offer.mimes,
                self.wayland
                    .focused_toplevel()
                    .and_then(|top| top.app_id)
                    .as_deref(),
                &self
                    .settings
                    .strv("wmclass-exclusions")
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            )
        {
            return;
        }
        let Some(mime) = choose_mime(&offer.mimes) else {
            return;
        };
        let Ok(fd) = self.wayland.read_clipboard(offer.id, &mime) else {
            return;
        };
        let stream = gio::UnixInputStream::take_fd(fd);
        let generation = self.generation.load(Ordering::Acquire);
        let weak = Rc::downgrade(self);
        let id = offer.id;
        let task = glib::spawn_future_local(async move {
            let read = async {
                let mut bytes = Vec::new();
                loop {
                    let part = stream
                        .read_bytes_future(64 * 1024, glib::Priority::DEFAULT)
                        .await
                        .map_err(|e| e.to_string())?;
                    if part.is_empty() {
                        break;
                    }
                    bytes.extend_from_slice(&part);
                }
                Ok::<_, String>(bytes)
            };
            let result = glib::future_with_timeout(Duration::from_secs(10), read).await;
            let Some(this) = weak.upgrade().filter(|this| {
                !this.stopped.get() && this.generation.load(Ordering::Acquire) == generation
            }) else {
                return;
            };
            this.transfers.borrow_mut().remove(&id);
            let Ok(Ok(bytes)) = result else {
                return;
            };
            if bytes.is_empty()
                || (mime.starts_with("text/") && String::from_utf8_lossy(&bytes).trim().is_empty())
            {
                return;
            }
            let guard = this.generation.clone();
            let max = this.settings.child("character-item").int("max-characters") as usize;
            let retention = this.retention();
            this.mutate(
                move |store| {
                    let content = way_shell_clipboard::classify(&mime, bytes, max)?;
                    if guard.load(Ordering::Acquire) != generation {
                        return store.entries();
                    }
                    store.insert(content, now(), retention)?;
                    store.entries()
                },
                true,
            );
        });
        self.transfers.borrow_mut().insert(id, task);
    }
    pub fn update(self: &Rc<Self>, entry: Entry) {
        let retention = self.retention();
        self.change(move |store| {
            store.update(&entry)?;
            store.prune(retention, now())?;
            store.entries()
        });
    }
    pub fn delete(self: &Rc<Self>, id: i64, force: bool) {
        let pinned = self.settings.boolean("protect-pinned");
        let tagged = self.settings.boolean("protect-tagged");
        self.change(move |store| {
            store.delete(id, pinned, tagged, force)?;
            store.entries()
        });
    }
    pub fn clear(self: &Rc<Self>, all: bool) {
        self.cancel_transfers();
        self.sound.borrow_mut().take();
        self.change(move |store| {
            store.clear(if all {
                HistoryPolicy::Clear
            } else {
                HistoryPolicy::KeepPinnedAndTagged
            })?;
            store.entries()
        });
    }
    pub fn incognito(&self, value: bool) -> Result<(), String> {
        self.settings
            .set_boolean("incognito", value)
            .map_err(|e| e.to_string())
    }
    pub fn actions(&self) -> ActionConfig {
        serde_json::from_str(&self.settings.string("action-config")).unwrap_or_default()
    }
    pub async fn cached_preview(&self, id: i64) -> Result<Option<Preview>, String> {
        let (send, receive) = futures_channel::oneshot::channel();
        let future = self
            .worker
            .borrow()
            .as_ref()
            .ok_or("Clipboard worker has stopped")?
            .request(move |store| {
                let result = store.preview(id).and_then(|row| {
                    row.map(|(value, image)| {
                        let mut preview: Preview =
                            serde_json::from_value(value).map_err(|e| e.to_string())?;
                        preview.image = image;
                        Ok(preview)
                    })
                    .transpose()
                });
                let _ = send.send(result);
                Ok(Vec::new())
            });
        future.await?;
        receive
            .await
            .map_err(|_| "Preview worker stopped".to_owned())?
    }
    pub async fn save_preview(&self, id: i64, preview: Preview) -> Result<(), String> {
        let future = self
            .worker
            .borrow()
            .as_ref()
            .ok_or("Clipboard worker has stopped")?
            .request(move |store| {
                store.save_preview(
                    id,
                    &serde_json::to_value(&preview).map_err(|e| e.to_string())?,
                    preview.image.as_deref(),
                )?;
                Ok(Vec::new())
            });
        future.await.map(|_| ())
    }
    pub fn restore(self: &Rc<Self>, id: i64) -> Result<Entry, String> {
        let mut entry = self.entry(id).ok_or("Clipboard item no longer exists")?;
        self.wayland.set_clipboard(
            entry.content.formats(),
            self.settings.boolean("sync-primary")
                && !matches!(entry.content.kind, Kind::Image | Kind::File | Kind::Files),
        )?;
        self.feedback(Some(if entry.content.text.is_empty() {
            entry.content.kind.name()
        } else {
            &entry.content.text
        }));
        self.changed();
        if self.settings.boolean("update-date-on-copy") {
            entry.copied = now();
            self.update(entry.clone());
        }
        Ok(entry)
    }
    pub fn restore_text(self: &Rc<Self>, text: &str) -> Result<(), String> {
        self.wayland.set_clipboard(
            Content::text(text).formats(),
            self.settings.boolean("sync-primary"),
        )?;
        self.feedback(Some(text));
        self.changed();
        Ok(())
    }
    pub fn terminal(&self, target: &Toplevel) -> bool {
        target.app_id.as_deref().is_some_and(|app| {
            self.settings
                .strv("terminal-apps")
                .iter()
                .any(|rule| rule.as_str().eq_ignore_ascii_case(app))
        })
    }
    fn cancel_transfers(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        for (_, task) in self.transfers.take() {
            task.abort();
        }
    }
    pub fn stop(&self) {
        if self.stopped.replace(true) {
            return;
        }
        self.cancel_transfers();
        self.sound.borrow_mut().take();
        self.callbacks.borrow_mut().clear();
        if let Some(timer) = self.retention_timer.borrow_mut().take() {
            timer.remove();
        }
        for (object, handler) in self.handlers.take() {
            object.disconnect(handler);
        }
        if let Some(worker) = self.worker.borrow_mut().take() {
            worker.finish();
        }
    }
}
impl Drop for ClipboardService {
    fn drop(&mut self) {
        self.stop();
    }
}
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
pub fn choose_mime(mimes: &[String]) -> Option<String> {
    [
        "image/png",
        "image/jxl",
        "image/webp",
        "image/avif",
        "image/jpeg",
        "x-special/gnome-copied-files",
        "text/uri-list",
        "text/plain;charset=utf-8",
        "UTF8_STRING",
        "text/plain",
        "STRING",
    ]
    .into_iter()
    .find(|mime| mimes.iter().any(|value| value == mime))
    .map(str::to_owned)
}
pub fn should_record(
    incognito: bool,
    mimes: &[String],
    app: Option<&str>,
    excluded: &[String],
) -> bool {
    !incognito
        && !mimes.iter().any(|mime| {
            matches!(
                mime.as_str(),
                "x-kde-passwordManagerHint" | "application/x-way-shell-clipboard"
            )
        })
        && !app.is_some_and(|app| excluded.iter().any(|value| value == app))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn privacy_is_checked_before_a_transfer_and_rich_formats_win() {
        let mimes = vec!["text/plain".into(), "image/png".into()];
        assert_eq!(choose_mime(&mimes).as_deref(), Some("image/png"));
        assert!(!should_record(true, &mimes, None, &[]));
        assert!(!should_record(
            false,
            &["x-kde-passwordManagerHint".into()],
            None,
            &[]
        ));
        assert!(!should_record(
            false,
            &mimes,
            Some("secret"),
            &["secret".into()]
        ));
        assert!(should_record(
            false,
            &mimes,
            Some("editor"),
            &["secret".into()]
        ));
    }
    #[test]
    fn replacing_a_worker_finishes_the_current_shutdown_policy_before_opening() {
        let directory = std::env::temp_dir().join(format!(
            "way-shell-clipboard-worker-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("history.db");
        let context = glib::MainContext::new();
        let worker = Worker::new(Some(path.clone()), HistoryPolicy::KeepAll, None, false);
        context
            .block_on(worker.request(|store| {
                store.insert(Content::text("retained"), now(), Retention::default())?;
                store.entries()
            }))
            .unwrap();
        worker
            .policy
            .store(HistoryPolicy::Clear as u8, Ordering::Release);
        let replacement = Worker::new(
            Some(path.clone()),
            HistoryPolicy::KeepAll,
            Some(worker),
            false,
        );
        assert!(
            context
                .block_on(replacement.request(|store| store.entries()))
                .unwrap()
                .is_empty()
        );
        context
            .block_on(replacement.request(|store| {
                store.insert(
                    Content::text("kept during retry"),
                    now(),
                    Retention::default(),
                )?;
                store.entries()
            }))
            .unwrap();
        replacement
            .policy
            .store(HistoryPolicy::Clear as u8, Ordering::Release);
        let retry = Worker::new(Some(path), HistoryPolicy::Clear, Some(replacement), true);
        assert_eq!(
            context
                .block_on(retry.request(|store| store.entries()))
                .unwrap()
                .len(),
            1
        );
        retry.finish();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
