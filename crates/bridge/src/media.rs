//! Temporary stable MPRIS records for the remaining C notification widgets.
use glib::{ffi, prelude::*, subclass::prelude::*, translate::*};
use std::{
    cell::{Cell, RefCell, UnsafeCell},
    collections::BTreeMap,
    ffi::{CStr, CString, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::OnceLock,
};
use way_shell::services::media::{MediaAction, MediaPlayer, MediaService, MediaState};

#[cfg(test)]
use crate::media_fixture as fixture;

/// Layout of the temporary `MediaPlayer` C structure. The proxy slots remain
/// null: native objects are owned privately by the permanent Rust service.
#[repr(C)]
#[derive(Default)]
struct PlayerRecord {
    player: *mut c_void,
    proxy: *mut c_void,
    identity: *const c_char,
    name: *const c_char,
    playback_status: *const c_char,
    art_url: *const c_char,
    album: *const c_char,
    artist: *const c_char,
    title: *const c_char,
}

struct Record {
    // C widgets borrow a stable, historically mutable record address. All
    // updates occur on the owning GLib main thread between signal emissions.
    value: Box<UnsafeCell<PlayerRecord>>,
    strings: [Option<CString>; 7],
}
impl Record {
    fn new(player: &MediaPlayer) -> Self {
        let mut record = Self {
            value: Box::default(),
            strings: Default::default(),
        };
        record.update(player);
        record
    }

    fn pointer(&self) -> *mut c_void {
        self.value.get().cast()
    }

    fn update(&mut self, player: &MediaPlayer) {
        let artist = player.metadata.artist();
        self.strings = [
            player.identity.as_deref(),
            Some(player.name.as_str()),
            Some(player.playback_status.as_str()),
            player.metadata.art_url.as_deref(),
            player.metadata.album.as_deref(),
            artist.as_deref(),
            player.metadata.title.as_deref(),
        ]
        // D-Bus excludes NUL. Still handle invalid manually supplied snapshots
        // without a panic crossing a GObject callback or the C boundary.
        .map(|value| value.and_then(|value| CString::new(value).ok()));
        let [
            identity,
            name,
            playback_status,
            art_url,
            album,
            artist,
            title,
        ] = self
            .strings
            .each_ref()
            .map(|value| value.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()));
        unsafe {
            *self.value.get() = PlayerRecord {
                identity,
                name,
                playback_status,
                art_url,
                album,
                artist,
                title,
                ..PlayerRecord::default()
            };
        }
    }
}

struct PlayerArray(*mut ffi::GPtrArray);
impl Default for PlayerArray {
    fn default() -> Self {
        Self(unsafe { ffi::g_ptr_array_new() })
    }
}
impl Drop for PlayerArray {
    fn drop(&mut self) {
        unsafe { ffi::g_ptr_array_unref(self.0) };
    }
}
impl PlayerArray {
    fn fill(&self, records: &BTreeMap<String, Record>) {
        unsafe {
            ffi::g_ptr_array_set_size(self.0, 0);
            for record in records.values() {
                ffi::g_ptr_array_add(self.0, record.pointer());
            }
        }
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MediaAdapter {
        pub service: RefCell<Option<MediaService>>,
        pub handler: RefCell<Option<glib::SignalHandlerId>>,
        pub state: RefCell<MediaState>,
        pub pending: RefCell<Option<MediaState>>,
        pub applying: Cell<bool>,
        pub(super) records: RefCell<BTreeMap<String, Record>>,
        pub(super) players: PlayerArray,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MediaAdapter {
        const NAME: &'static str = "WayShellMediaAdapter";
        type Type = super::MediaAdapter;
    }

    impl ObjectImpl for MediaAdapter {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                ["media-player-changed", "media-player-removed"]
                    .into_iter()
                    .map(|name| {
                        glib::subclass::Signal::builder(name)
                            .param_types([glib::Type::POINTER])
                            .build()
                    })
                    .collect()
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
            self.pending.borrow_mut().take();
        }
    }
}

glib::wrapper! { pub struct MediaAdapter(ObjectSubclass<imp::MediaAdapter>); }

impl MediaAdapter {
    fn new(service: MediaService) -> Self {
        let adapter: Self = glib::Object::new();
        let weak = adapter.downgrade();
        let handler = service.connect_local("changed", false, move |_| {
            if let Some(adapter) = weak.upgrade() {
                adapter.refresh();
            }
            None
        });
        adapter.imp().service.replace(Some(service));
        adapter.imp().handler.replace(Some(handler));
        adapter.refresh();
        adapter
    }

    fn refresh(&self) {
        let snapshot = self
            .imp()
            .service
            .borrow()
            .as_ref()
            .map(MediaService::state)
            .unwrap_or_default();
        self.apply(snapshot);
    }

    fn apply(&self, snapshot: MediaState) {
        self.imp().pending.replace(Some(snapshot));
        if self.imp().applying.replace(true) {
            return;
        }
        // A C callback may iterate the main context or cause another service
        // update. Defer it until every observer has finished borrowing the
        // current signal record, including records being removed.
        let mut next = self.imp().pending.borrow_mut().take();
        while let Some(snapshot) = next {
            self.apply_one(snapshot);
            next = self.imp().pending.borrow_mut().take();
        }
        self.imp().applying.set(false);
    }

    fn apply_one(&self, snapshot: MediaState) {
        let previous = self.imp().state.replace(snapshot.clone());
        let mut removed = Vec::new();
        let mut changed = Vec::new();
        {
            let mut records = self.imp().records.borrow_mut();
            for (name, record) in std::mem::take(&mut *records) {
                let old = previous.players.iter().find(|player| player.name == name);
                let new = snapshot.players.iter().find(|player| player.name == name);
                if old
                    .zip(new)
                    .is_some_and(|(old, new)| old.owner == new.owner)
                {
                    records.insert(name, record);
                } else {
                    removed.push(record);
                }
            }
            for player in &snapshot.players {
                let old = previous.players.iter().find(|old| old.name == player.name);
                let record = records
                    .entry(player.name.clone())
                    .or_insert_with(|| Record::new(player));
                if old != Some(player) {
                    record.update(player);
                    changed.push(record.pointer());
                }
            }
            self.imp().players.fill(&records);
        }
        // An owner replacement removes the previous widget before publishing
        // the replacement under the same well-known name.
        for record in &removed {
            self.emit_by_name::<()>("media-player-removed", &[&record.pointer()]);
        }
        for pointer in changed {
            self.emit_by_name::<()>("media-player-changed", &[&pointer]);
        }
    }

    fn command(&self, name: &str, action: MediaAction) {
        let service = self.imp().service.borrow().clone();
        if let Some(service) = service {
            service.command(name, action, |result| {
                if let Err(error) = result {
                    glib::g_message!("way-shell", "Media player operation failed: {error}");
                }
            });
        }
    }
}

thread_local! { static GLOBAL: RefCell<Option<MediaAdapter>> = const { RefCell::new(None) }; }

pub(crate) fn service() -> Option<MediaService> {
    let adapter = GLOBAL.with(|global| global.borrow().clone())?;
    adapter.imp().service.borrow().clone()
}

pub fn shutdown() {
    // C widgets borrow the global without retaining a GObject reference. Keep
    // the stopped adapter and its empty array alive until this thread exits.
    // Release both RefCell borrows before stop notifies removal observers;
    // those observers can query the global or reenter shutdown.
    let adapter = GLOBAL.with(|global| global.borrow().clone());
    if let Some(adapter) = adapter {
        let service = adapter.imp().service.borrow().clone();
        if let Some(service) = service {
            service.stop();
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn media_player_service_get_type() -> ffi::GType {
    catch_unwind(|| MediaAdapter::static_type().into_glib()).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn media_player_service_global_init() -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        GLOBAL.with(|global| {
            if global.borrow().is_none() {
                global.replace(Some(MediaAdapter::new(MediaService::new())));
            }
            0
        })
    }))
    .unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub extern "C" fn media_player_service_get_global() -> *mut glib::gobject_ffi::GObject {
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

unsafe fn borrowed(pointer: *mut glib::gobject_ffi::GObject) -> Option<MediaAdapter> {
    if pointer.is_null() {
        return None;
    }
    let object: Borrowed<glib::Object> = unsafe { from_glib_borrow(pointer) };
    object.downcast_ref::<MediaAdapter>().cloned()
}

/// # Safety
/// `pointer` is null or a live borrowed service on the application thread.
/// The result and its records are borrowed and must not be changed or freed.
/// Iterate synchronously; membership can change at the next media signal.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn media_player_service_get_players(
    pointer: *mut glib::gobject_ffi::GObject,
) -> *mut ffi::GPtrArray {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(pointer) }.map_or(std::ptr::null_mut(), |adapter| adapter.imp().players.0)
    }))
    .unwrap_or(std::ptr::null_mut())
}

macro_rules! command {
    ($name:ident, $action:expr) => {
        /// # Safety
        /// `pointer` is null or a live borrowed service on the application
        /// thread. `name` is null or a readable NUL-terminated string, copied
        /// before any asynchronous operation begins.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(
            pointer: *mut glib::gobject_ffi::GObject,
            name: *const c_char,
        ) {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let Some(adapter) = (unsafe { borrowed(pointer) }) else {
                    return;
                };
                if name.is_null() {
                    return;
                }
                let Ok(name) = (unsafe { CStr::from_ptr(name) }).to_str() else {
                    return;
                };
                let name = name.to_owned();
                adapter.command(&name, $action);
            }));
        }
    };
}

command!(media_player_service_player_play, MediaAction::Play);
command!(media_player_service_player_pause, MediaAction::Pause);
command!(
    media_player_service_player_playpause,
    MediaAction::PlayPause
);
command!(media_player_service_player_stop, MediaAction::Stop);
command!(media_player_service_player_next, MediaAction::Next);
command!(media_player_service_player_previous, MediaAction::Previous);
command!(media_player_service_player_raise, MediaAction::Raise);

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        mem::{offset_of, size_of},
        rc::Rc,
    };
    use way_shell::services::media::{Metadata, PlaybackStatus};

    fn player() -> MediaPlayer {
        MediaPlayer {
            name: "org.mpris.MediaPlayer2.fixture".into(),
            owner: ":1.42".into(),
            identity: Some("Fixture player".into()),
            playback_status: PlaybackStatus::Playing,
            metadata: Metadata {
                title: Some("First track".into()),
                album: Some("Fixture album".into()),
                artists: vec!["First artist".into(), "Second artist".into()],
                art_url: Some("file:///tmp/cover.png".into()),
            },
            capabilities: Default::default(),
        }
    }

    fn snapshot(player: MediaPlayer) -> MediaState {
        MediaState {
            available: true,
            players: vec![player],
        }
    }

    fn record(adapter: &MediaAdapter) -> *mut PlayerRecord {
        let array = unsafe { media_player_service_get_players(adapter.as_ptr().cast()) };
        assert!(!array.is_null());
        unsafe {
            assert_eq!((*array).len, 1);
            (*(*array).pdata).cast()
        }
    }

    #[test]
    fn c_layout_matches_media_player_header() {
        let pointer_size = size_of::<*mut c_void>();
        assert_eq!(size_of::<PlayerRecord>(), 9 * pointer_size);
        assert_eq!(offset_of!(PlayerRecord, player), 0);
        assert_eq!(offset_of!(PlayerRecord, proxy), pointer_size);
        assert_eq!(offset_of!(PlayerRecord, identity), 2 * pointer_size);
        assert_eq!(offset_of!(PlayerRecord, name), 3 * pointer_size);
        assert_eq!(offset_of!(PlayerRecord, playback_status), 4 * pointer_size);
        assert_eq!(offset_of!(PlayerRecord, art_url), 5 * pointer_size);
        assert_eq!(offset_of!(PlayerRecord, album), 6 * pointer_size);
        assert_eq!(offset_of!(PlayerRecord, artist), 7 * pointer_size);
        assert_eq!(offset_of!(PlayerRecord, title), 8 * pointer_size);
    }

    #[test]
    fn stable_records_copy_metadata_and_live_through_removal() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let adapter: MediaAdapter = glib::Object::new();
                let mut player = player();
                adapter.apply(snapshot(player.clone()));
                let pointer = record(&adapter);
                unsafe {
                    assert!((*pointer).player.is_null());
                    assert!((*pointer).proxy.is_null());
                    assert_eq!(
                        CStr::from_ptr((*pointer).identity).to_bytes(),
                        b"Fixture player"
                    );
                    assert_eq!(
                        CStr::from_ptr((*pointer).artist).to_bytes(),
                        b"First artist, Second artist"
                    );
                    assert_eq!(
                        CStr::from_ptr((*pointer).playback_status).to_bytes(),
                        b"Playing"
                    );
                }
                let changes = Rc::new(Cell::new(0));
                let observed = changes.clone();
                adapter.connect_local("media-player-changed", false, move |_| {
                    observed.set(observed.get() + 1);
                    None
                });
                adapter.apply(snapshot(player.clone()));
                assert_eq!(changes.get(), 0);
                player.metadata = Metadata {
                    title: Some("Second track".into()),
                    ..Metadata::default()
                };
                player.playback_status = PlaybackStatus::Paused;
                adapter.apply(snapshot(player.clone()));
                assert_eq!(record(&adapter), pointer);
                assert_eq!(changes.get(), 1);
                unsafe {
                    assert!((*pointer).artist.is_null());
                    assert!((*pointer).album.is_null());
                    assert!((*pointer).art_url.is_null());
                    assert_eq!(CStr::from_ptr((*pointer).title).to_bytes(), b"Second track");
                }
                let removed = Rc::new(Cell::new(false));
                let observed = removed.clone();
                adapter.connect_local("media-player-removed", false, move |values| {
                    let value = values[1]
                        .get::<*mut c_void>()
                        .unwrap()
                        .cast::<PlayerRecord>();
                    assert_eq!(value, pointer);
                    unsafe {
                        assert_eq!(CStr::from_ptr((*value).title).to_bytes(), b"Second track");
                        assert_eq!(
                            CStr::from_ptr((*value).name).to_bytes(),
                            b"org.mpris.MediaPlayer2.fixture"
                        );
                    }
                    observed.set(true);
                    None
                });
                adapter.apply(MediaState::default());
                assert!(removed.get());
                let array = unsafe { media_player_service_get_players(adapter.as_ptr().cast()) };
                unsafe {
                    assert_eq!((*array).len, 0);
                }
            })
            .unwrap();
    }

    #[test]
    fn replacement_and_reentrant_updates_preserve_callback_borrows() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let adapter: MediaAdapter = glib::Object::new();
                let mut player = player();
                adapter.apply(snapshot(player.clone()));
                let old_pointer = record(&adapter);
                let events = Rc::new(RefCell::new(Vec::new()));
                let removed_events = events.clone();
                adapter.connect_local("media-player-removed", false, move |values| {
                    let pointer = values[1]
                        .get::<*mut c_void>()
                        .unwrap()
                        .cast::<PlayerRecord>();
                    unsafe {
                        let identity = CStr::from_ptr((*pointer).identity)
                            .to_str()
                            .unwrap()
                            .to_owned();
                        removed_events
                            .borrow_mut()
                            .push(format!("removed {identity}"));
                    }
                    None
                });
                let changed_events = events.clone();
                let weak = adapter.downgrade();
                adapter.connect_local("media-player-changed", false, move |values| {
                    let pointer = values[1]
                        .get::<*mut c_void>()
                        .unwrap()
                        .cast::<PlayerRecord>();
                    assert_ne!(pointer, old_pointer);
                    let adapter = weak.upgrade().unwrap();
                    adapter.apply(MediaState::default());
                    // The nested update must not destroy the current signal data.
                    unsafe {
                        assert_eq!(
                            CStr::from_ptr((*pointer).identity).to_bytes(),
                            b"Replacement"
                        );
                    }
                    changed_events
                        .borrow_mut()
                        .push("changed Replacement".to_owned());
                    None
                });
                let last_events = events.clone();
                adapter.connect_local("media-player-changed", false, move |values| {
                    let pointer = values[1]
                        .get::<*mut c_void>()
                        .unwrap()
                        .cast::<PlayerRecord>();
                    unsafe {
                        assert_eq!(
                            CStr::from_ptr((*pointer).identity).to_bytes(),
                            b"Replacement"
                        );
                    }
                    last_events.borrow_mut().push("last observer".to_owned());
                    None
                });
                player.owner = ":1.43".into();
                player.identity = Some("Replacement".into());
                adapter.apply(snapshot(player));
                assert_eq!(
                    &*events.borrow(),
                    &[
                        "removed Fixture player",
                        "changed Replacement",
                        "last observer",
                        "removed Replacement"
                    ]
                );
                assert!(adapter.imp().state.borrow().players.is_empty());
            })
            .unwrap();
    }

    #[test]
    fn dropping_adapter_disconnects_weak_service_callback() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                // A bare service has no bus watch. The adapter exercises the actual
                // production subscription and disposal without the user's bus.
                let service: MediaService = glib::Object::new();
                let adapter = MediaAdapter::new(service.clone());
                let handler = unsafe { adapter.imp().handler.borrow().as_ref().unwrap().as_raw() };
                unsafe {
                    assert_ne!(
                        glib::gobject_ffi::g_signal_handler_is_connected(
                            service.as_ptr().cast(),
                            handler
                        ),
                        0
                    );
                }
                let weak = adapter.downgrade();
                drop(adapter);
                assert!(weak.upgrade().is_none());
                unsafe {
                    assert_eq!(
                        glib::gobject_ffi::g_signal_handler_is_connected(
                            service.as_ptr().cast(),
                            handler
                        ),
                        0
                    );
                }
                service.emit_by_name::<()>("changed", &[]);
                let weak = service.downgrade();
                drop(service);
                assert!(weak.upgrade().is_none());
            })
            .unwrap();
    }

    #[test]
    fn private_bus_records_and_c_commands_preserve_ownership() {
        use fixture::{Bus, PLAYER, Player, ROOT, connect, wait};
        use std::{collections::HashMap, time::Duration};

        let (_bus, address) = Bus::start();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let player =
                    Player::new(&address, "org.mpris.MediaPlayer2.bridge", "Initial track");
                player.acquire(false);
                let connection = connect(&address);
                let service = MediaService::on_connection(&connection);
                let adapter = MediaAdapter::new(service.clone());
                let object = adapter.as_ptr().cast();
                let array = unsafe { media_player_service_get_players(object) };
                wait(&context, || unsafe { (*array).len == 1 });
                let pointer = record(&adapter);
                unsafe {
                    assert_eq!(
                        CStr::from_ptr((*pointer).title).to_bytes(),
                        b"Initial track"
                    );
                    assert_eq!(CStr::from_ptr((*pointer).artist).to_bytes(), b"One, Two");
                }

                type Command = unsafe extern "C" fn(*mut glib::gobject_ffi::GObject, *const c_char);
                let commands: [(Command, &str, &str); 7] = [
                    (media_player_service_player_play, PLAYER, "Play"),
                    (media_player_service_player_pause, PLAYER, "Pause"),
                    (media_player_service_player_playpause, PLAYER, "PlayPause"),
                    (media_player_service_player_stop, PLAYER, "Stop"),
                    (media_player_service_player_next, PLAYER, "Next"),
                    (media_player_service_player_previous, PLAYER, "Previous"),
                    (media_player_service_player_raise, ROOT, "Raise"),
                ];
                for (command, _, _) in commands {
                    let mut name = CString::new(player.name.clone())
                        .unwrap()
                        .into_bytes_with_nul();
                    unsafe { command(object, name.as_ptr().cast()) };
                    // Async requests may not retain the caller's C buffer.
                    name.fill(b'x');
                }
                wait(&context, || player.calls.borrow().len() == commands.len());
                assert_eq!(
                    &*player.calls.borrow(),
                    &commands
                        .into_iter()
                        .map(|(_, interface, method)| (interface.to_owned(), method.to_owned()))
                        .collect::<Vec<_>>()
                );

                let changed = Rc::new(Cell::new(0));
                let observed = changed.clone();
                adapter.connect_local("media-player-changed", false, move |values| {
                    assert_eq!(values[1].get::<*mut c_void>().unwrap(), pointer.cast());
                    observed.set(observed.get() + 1);
                    None
                });
                player.property(
                    PLAYER,
                    "Metadata",
                    HashMap::from([("xesam:title", "Next track".to_variant())]).to_variant(),
                );
                wait(&context, || changed.get() > 0);
                assert_eq!(record(&adapter), pointer);
                unsafe {
                    assert_eq!(CStr::from_ptr((*pointer).title).to_bytes(), b"Next track");
                    assert!((*pointer).artist.is_null());
                    assert!((*pointer).art_url.is_null());
                }

                let removed = Rc::new(Cell::new(false));
                let observed = removed.clone();
                let handler = adapter.connect_local("media-player-removed", false, move |values| {
                    let removed = values[1]
                        .get::<*mut c_void>()
                        .unwrap()
                        .cast::<PlayerRecord>();
                    assert_eq!(removed, pointer);
                    unsafe {
                        assert_eq!(CStr::from_ptr((*removed).title).to_bytes(), b"Next track");
                        assert_eq!(
                            CStr::from_ptr((*removed).name).to_bytes(),
                            b"org.mpris.MediaPlayer2.bridge"
                        );
                    }
                    observed.set(true);
                    None
                });
                player.release();
                wait(&context, || removed.get());
                unsafe {
                    assert_eq!((*array).len, 0);
                }
                adapter.disconnect(handler);

                let weak_adapter = adapter.downgrade();
                drop(adapter);
                assert!(weak_adapter.upgrade().is_none());
                assert!(!service.state().available);
                let weak_service = service.downgrade();
                drop(service);
                assert!(weak_service.upgrade().is_none());
                player.acquire(false);
                player.property(PLAYER, "Metadata", fixture::metadata("After teardown"));
                context.block_on(glib::timeout_future(Duration::from_millis(30)));
                connection.close_sync(gio::Cancellable::NONE).unwrap();
            })
            .unwrap();
    }

    #[test]
    fn shutdown_retains_borrowed_global_for_reentrant_teardown() {
        use fixture::{Bus, Player, connect, wait};

        let (_bus, address) = Bus::start();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let player =
                    Player::new(&address, "org.mpris.MediaPlayer2.shutdown", "Final track");
                player.acquire(false);
                let connection = connect(&address);
                let service = MediaService::on_connection(&connection);
                let adapter = MediaAdapter::new(service.clone());
                GLOBAL.with(|global| {
                    assert!(global.borrow().is_none());
                    global.replace(Some(adapter.clone()));
                });
                let object = media_player_service_get_global();
                let array = unsafe { media_player_service_get_players(object) };
                wait(&context, || unsafe { (*array).len == 1 });
                let pointer = record(&adapter);
                let removed = Rc::new(Cell::new(false));
                let observed = removed.clone();
                adapter.connect_local("media-player-removed", false, move |values| {
                    assert_eq!(media_player_service_get_global(), object);
                    assert_eq!(values[1].get::<*mut c_void>().unwrap(), pointer.cast());
                    unsafe {
                        assert_eq!(media_player_service_get_players(object), array);
                        assert_eq!((*array).len, 0);
                        assert_eq!(CStr::from_ptr((*pointer).title).to_bytes(), b"Final track");
                    }
                    shutdown();
                    observed.set(true);
                    None
                });
                let weak_adapter = adapter.downgrade();
                drop(adapter);
                shutdown();
                // Check the address first, without dereferencing a pointer the
                // pre-fix shutdown could already have freed.
                assert_eq!(media_player_service_get_global(), object);
                assert!(weak_adapter.upgrade().is_some());
                assert!(removed.get());
                assert!(!service.state().available);
                unsafe {
                    assert_eq!(media_player_service_get_players(object), array);
                    assert_eq!((*array).len, 0);
                    // Null and unrelated live GObjects are rejected without
                    // accessing a MediaAdapter instance layout.
                    assert!(media_player_service_get_players(std::ptr::null_mut()).is_null());
                    assert!(media_player_service_get_players(service.as_ptr().cast()).is_null());
                }
                GLOBAL.with(|global| {
                    global.borrow_mut().take();
                });
                assert!(weak_adapter.upgrade().is_none());
                drop(service);
                connection.close_sync(gio::Cancellable::NONE).unwrap();
            })
            .unwrap();
    }
}
