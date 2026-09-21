//! MPRIS inventory and controls on the shell's GLib main context.
use gio::prelude::*;
use glib::subclass::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    sync::OnceLock,
    time::Duration,
};

const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";
const PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT: &str = "org.mpris.MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const DEADLINE: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
    #[default]
    Unknown,
}
impl PlaybackStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Playing => "Playing",
            Self::Paused => "Paused",
            Self::Stopped => "Stopped",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    pub title: Option<String>,
    pub album: Option<String>,
    pub art_url: Option<String>,
    pub artists: Vec<String>,
}
impl Metadata {
    pub fn artist(&self) -> Option<String> {
        (!self.artists.is_empty()).then(|| self.artists.join(", "))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub can_control: bool,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_go_next: bool,
    pub can_go_previous: bool,
    pub can_raise: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaPlayer {
    /// The well-known MPRIS bus name used by existing keybindings and widgets.
    pub name: String,
    /// The unique owner distinguishes replacements of the same player name.
    pub owner: String,
    pub identity: Option<String>,
    pub playback_status: PlaybackStatus,
    pub metadata: Metadata,
    pub capabilities: Capabilities,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaState {
    pub available: bool,
    pub players: Vec<MediaPlayer>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaAction {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    Raise,
}
impl MediaAction {
    fn method(self) -> &'static str {
        match self {
            Self::Play => "Play",
            Self::Pause => "Pause",
            Self::PlayPause => "PlayPause",
            Self::Stop => "Stop",
            Self::Next => "Next",
            Self::Previous => "Previous",
            Self::Raise => "Raise",
        }
    }
}

struct Proxy {
    proxy: gio::DBusProxy,
    handler: Option<glib::SignalHandlerId>,
}
impl Drop for Proxy {
    fn drop(&mut self) {
        if let Some(handler) = self.handler.take() {
            self.proxy.disconnect(handler);
        }
    }
}

#[derive(Default)]
struct Entry {
    owner: String,
    epoch: u64,
    cancellable: Option<gio::Cancellable>,
    deadline: Option<glib::JoinHandle<()>>,
    retry: Option<glib::JoinHandle<()>>,
    root: Option<Proxy>,
    player: Option<Proxy>,
}
impl Entry {
    fn reset(&mut self) {
        self.epoch = 0;
        if let Some(task) = self.deadline.take() {
            task.abort();
        }
        if let Some(task) = self.retry.take() {
            task.abort();
        }
        if let Some(cancellable) = self.cancellable.take() {
            cancellable.cancel();
        }
        self.root.take();
        self.player.take();
    }
}
impl Drop for Entry {
    fn drop(&mut self) {
        self.reset();
    }
}

struct Connection {
    connection: gio::DBusConnection,
    _subscription: gio::SignalSubscription,
    closed: Option<glib::SignalHandlerId>,
    cancellable: gio::Cancellable,
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.cancellable.cancel();
        if let Some(handler) = self.closed.take() {
            self.connection.disconnect(handler);
        }
    }
}

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct MediaService {
        pub state: RefCell<MediaState>,
        pub source: RefCell<Option<gio::DBusConnection>>,
        pub running: Cell<bool>,
        pub connecting: RefCell<Option<gio::Cancellable>>,
        pub connect_deadline: RefCell<Option<glib::JoinHandle<()>>>,
        pub generation: Cell<u64>,
        pub next_epoch: Cell<u64>,
        pub(super) connection: RefCell<Option<Connection>>,
        pub(super) entries: RefCell<BTreeMap<String, Entry>>,
        pub retry: RefCell<Option<glib::JoinHandle<()>>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for MediaService {
        const NAME: &'static str = "WayShellMediaService";
        type Type = super::MediaService;
    }
    impl ObjectImpl for MediaService {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("changed").build()])
        }
        fn dispose(&self) {
            self.obj().stop();
        }
    }
}
glib::wrapper! { pub struct MediaService(ObjectSubclass<imp::MediaService>); }
impl Default for MediaService {
    fn default() -> Self {
        Self::new()
    }
}
impl MediaService {
    pub fn new() -> Self {
        let service: Self = glib::Object::new();
        service.start();
        service
    }

    /// Use an existing connection, including a private bus in application tests.
    pub fn on_connection(connection: &gio::DBusConnection) -> Self {
        let service: Self = glib::Object::new();
        service.imp().source.replace(Some(connection.clone()));
        service.start();
        service
    }

    pub fn start(&self) {
        if self.imp().running.replace(true) {
            return;
        }
        self.connect();
    }

    pub fn stop(&self) {
        self.imp().running.set(false);
        self.reset();
    }

    fn connect(&self) {
        let source = self.imp().source.borrow().clone();
        if let Some(connection) = source {
            if !connection.is_closed() {
                self.attach(&connection);
            }
            return;
        }
        let cancellable = gio::Cancellable::new();
        self.imp().connecting.replace(Some(cancellable.clone()));
        let cancel = cancellable.clone();
        self.imp().connect_deadline.replace(Some(
            glib::MainContext::ref_thread_default().spawn_local(async move {
                glib::timeout_future(DEADLINE).await;
                cancel.cancel();
            }),
        ));
        let generation = self.imp().generation.get();
        let weak = self.downgrade();
        gio::bus_get(gio::BusType::Session, Some(&cancellable), move |result| {
            let Some(service) = weak.upgrade().filter(|service| {
                service.imp().running.get() && service.imp().generation.get() == generation
            }) else {
                return;
            };
            service.imp().connecting.borrow_mut().take();
            if let Some(task) = service.imp().connect_deadline.borrow_mut().take() {
                task.abort();
            }
            match result {
                Ok(connection) => service.attach(&connection),
                Err(error) => {
                    glib::g_message!(
                        "way-shell",
                        "Media session bus unavailable: {error}; retrying"
                    );
                    service.retry_connection();
                }
            }
        });
    }

    fn retry_connection(&self) {
        let weak = self.downgrade();
        self.imp()
            .retry
            .replace(Some(glib::MainContext::ref_thread_default().spawn_local(
                async move {
                    glib::timeout_future(Duration::from_secs(1)).await;
                    if let Some(service) = weak.upgrade() {
                        service.imp().retry.borrow_mut().take();
                        if service.imp().running.get() {
                            service.connect();
                        }
                    }
                },
            )));
    }

    pub fn state(&self) -> MediaState {
        self.imp().state.borrow().clone()
    }

    /// Complete with the player's actual reply. A replacement cannot receive
    /// a request that was selected for the preceding owner of its bus name.
    pub fn command(
        &self,
        name: &str,
        action: MediaAction,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) {
        let request = self.imp().entries.borrow().get(name).and_then(|entry| {
            let proxy = if action == MediaAction::Raise {
                entry.root.as_ref()
            } else {
                entry.player.as_ref()
            }?;
            Some((
                proxy.proxy.clone(),
                entry.cancellable.clone()?,
                entry.epoch,
                entry.owner.clone(),
            ))
        });
        let Some((proxy, cancellable, epoch, owner)) = request else {
            done(Err(glib::Error::new(
                gio::IOErrorEnum::NotConnected,
                "Media player is unavailable",
            )));
            return;
        };
        let name = name.to_owned();
        let generation = self.imp().generation.get();
        let weak = self.downgrade();
        proxy.call(
            action.method(),
            Some(&().to_variant()),
            gio::DBusCallFlags::NONE,
            DEADLINE.as_millis() as i32,
            Some(&cancellable),
            move |result| {
                let current = weak
                    .upgrade()
                    .is_some_and(|service| service.current(&name, &owner, epoch, generation));
                if !current {
                    done(Err(glib::Error::new(
                        gio::IOErrorEnum::Cancelled,
                        "Media player changed during the request",
                    )));
                } else {
                    done(result.and_then(|reply| {
                        if reply.is_type(glib::VariantTy::UNIT) {
                            Ok(())
                        } else {
                            Err(glib::Error::new(
                                gio::IOErrorEnum::InvalidData,
                                "Media player returned an invalid command reply",
                            ))
                        }
                    }));
                }
            },
        );
    }

    fn attach(&self, connection: &gio::DBusConnection) {
        self.reset();
        connection.set_exit_on_close(false);
        let weak = self.downgrade();
        let subscription = connection.subscribe_to_signal(
            Some(BUS),
            Some(BUS),
            Some("NameOwnerChanged"),
            Some(BUS_PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let Some((name, _, new_owner)) =
                    signal.parameters.get::<(String, String, String)>()
                else {
                    return;
                };
                if is_player_name(&name)
                    && let Some(service) = weak.upgrade()
                {
                    if new_owner.is_empty() {
                        service.imp().entries.borrow_mut().remove(&name);
                        service.refresh();
                    } else {
                        service
                            .imp()
                            .entries
                            .borrow_mut()
                            .entry(name.clone())
                            .or_default();
                        service.load_player(signal.connection, &name, &new_owner);
                    }
                }
            },
        );
        let weak = self.downgrade();
        // GIO emits closed on the connection's owning main context.
        let closed = connection.connect_local("closed", false, move |values| {
            if let Some(service) = weak.upgrade() {
                let error = values[2].get::<Option<glib::Error>>().ok().flatten();
                glib::g_message!("way-shell", "Media session bus disconnected: {error:?}");
                service.reset();
                if service.imp().source.borrow().is_none() && service.imp().running.get() {
                    service.retry_connection();
                }
            }
            None
        });
        self.imp().connection.replace(Some(Connection {
            connection: connection.clone(),
            _subscription: subscription,
            closed: Some(closed),
            cancellable: gio::Cancellable::new(),
        }));
        self.refresh();
        self.list_names();
    }

    fn list_names(&self) {
        let source = self
            .imp()
            .connection
            .borrow()
            .as_ref()
            .map(|source| (source.connection.clone(), source.cancellable.clone()));
        let Some((connection, cancellable)) = source else {
            return;
        };
        let generation = self.imp().generation.get();
        let weak = self.downgrade();
        connection.call(
            Some(BUS),
            BUS_PATH,
            BUS,
            "ListNames",
            None,
            Some(glib::VariantTy::new("(as)").unwrap()),
            gio::DBusCallFlags::NONE,
            DEADLINE.as_millis() as i32,
            Some(&cancellable),
            move |result| {
                let Some(service) = weak
                    .upgrade()
                    .filter(|service| service.imp().generation.get() == generation)
                else {
                    return;
                };
                match result.and_then(|reply| {
                    reply.get::<(Vec<String>,)>().ok_or_else(|| {
                        glib::Error::new(
                            gio::IOErrorEnum::InvalidData,
                            "Invalid bus name inventory",
                        )
                    })
                }) {
                    Ok((names,)) => {
                        for name in names.iter().filter(|name| is_player_name(name)) {
                            service.watch_player(name);
                        }
                    }
                    Err(error) => {
                        glib::g_message!("way-shell", "Media discovery failed: {error}; retrying");
                        service.retry_discovery();
                    }
                }
            },
        );
    }

    fn retry_discovery(&self) {
        if self.imp().retry.borrow().is_some() {
            return;
        }
        let weak = self.downgrade();
        let task = glib::MainContext::ref_thread_default().spawn_local(async move {
            glib::timeout_future(Duration::from_secs(1)).await;
            if let Some(service) = weak.upgrade() {
                service.imp().retry.borrow_mut().take();
                service.list_names();
            }
        });
        self.imp().retry.replace(Some(task));
    }

    fn watch_player(&self, name: &str) {
        if self.imp().entries.borrow().contains_key(name) {
            return;
        }
        let source = self
            .imp()
            .connection
            .borrow()
            .as_ref()
            .map(|source| (source.connection.clone(), source.cancellable.clone()));
        let Some((connection, cancellable)) = source else {
            return;
        };
        let epoch = self.next_epoch();
        self.imp().entries.borrow_mut().insert(
            name.to_owned(),
            Entry {
                epoch,
                owner: String::new(),
                cancellable: None,
                deadline: None,
                retry: None,
                root: None,
                player: None,
            },
        );
        let generation = self.imp().generation.get();
        let weak = self.downgrade();
        let callback_name = name.to_owned();
        let callback_connection = connection.clone();
        connection.call(
            Some(BUS),
            BUS_PATH,
            BUS,
            "GetNameOwner",
            Some(&(name,).to_variant()),
            Some(glib::VariantTy::new("(s)").unwrap()),
            gio::DBusCallFlags::NONE,
            DEADLINE.as_millis() as i32,
            Some(&cancellable),
            move |result| {
                let Some(service) = weak
                    .upgrade()
                    .filter(|service| service.current(&callback_name, "", epoch, generation))
                else {
                    return;
                };
                match result.and_then(|reply| {
                    reply.get::<(String,)>().ok_or_else(|| {
                        glib::Error::new(
                            gio::IOErrorEnum::InvalidData,
                            "Invalid media player owner",
                        )
                    })
                }) {
                    Ok((owner,)) => {
                        service.load_player(&callback_connection, &callback_name, &owner)
                    }
                    Err(error) => {
                        service.imp().entries.borrow_mut().remove(&callback_name);
                        // Disappearance between enumeration and owner lookup is
                        // expected; transport failures need a fresh inventory.
                        if !error.matches(gio::DBusError::NameHasNoOwner) {
                            glib::g_message!(
                                "way-shell",
                                "Media player owner lookup failed: {error}; retrying"
                            );
                            service.retry_discovery();
                        }
                    }
                }
            },
        );
    }

    fn next_epoch(&self) -> u64 {
        let epoch = self.imp().next_epoch.get().wrapping_add(1);
        self.imp().next_epoch.set(epoch);
        epoch
    }

    fn load_player(&self, connection: &gio::DBusConnection, name: &str, owner: &str) {
        let cancellable = gio::Cancellable::new();
        let epoch = self.next_epoch();
        {
            let mut entries = self.imp().entries.borrow_mut();
            let Some(entry) = entries.get_mut(name) else {
                return;
            };
            entry.reset();
            entry.epoch = epoch;
            entry.owner = owner.to_owned();
            entry.cancellable = Some(cancellable.clone());
            let cancel = cancellable.clone();
            entry.deadline = Some(glib::MainContext::ref_thread_default().spawn_local(
                async move {
                    glib::timeout_future(DEADLINE).await;
                    cancel.cancel();
                },
            ));
        }
        self.refresh();
        let generation = self.imp().generation.get();
        let weak = self.downgrade();
        let connection = connection.clone();
        let name = name.to_owned();
        let owner = owner.to_owned();
        let callback_owner = owner.clone();
        let callback_connection = connection.clone();
        gio::DBusProxy::new(
            &connection,
            gio::DBusProxyFlags::DO_NOT_AUTO_START
                | gio::DBusProxyFlags::GET_INVALIDATED_PROPERTIES,
            None,
            Some(&owner),
            PATH,
            ROOT,
            Some(&cancellable),
            move |result| {
                let owner = callback_owner;
                let Some(service) = weak
                    .upgrade()
                    .filter(|service| service.current(&name, &owner, epoch, generation))
                else {
                    return;
                };
                match result.and_then(|root| {
                    initial_properties(&root, &[("Identity", glib::VariantTy::STRING)])?;
                    Ok(root)
                }) {
                    Ok(root) => service.load_controls(
                        &callback_connection,
                        &name,
                        &owner,
                        epoch,
                        generation,
                        root,
                    ),
                    Err(error) => service.player_failed(&name, &error),
                }
            },
        );
    }

    fn load_controls(
        &self,
        connection: &gio::DBusConnection,
        name: &str,
        owner: &str,
        epoch: u64,
        generation: u64,
        root: gio::DBusProxy,
    ) {
        let cancellable = self
            .imp()
            .entries
            .borrow()
            .get(name)
            .and_then(|entry| entry.cancellable.clone());
        let weak = self.downgrade();
        let name = name.to_owned();
        let callback_owner = owner.to_owned();
        gio::DBusProxy::new(
            connection,
            gio::DBusProxyFlags::DO_NOT_AUTO_START
                | gio::DBusProxyFlags::GET_INVALIDATED_PROPERTIES,
            None,
            Some(owner),
            PATH,
            PLAYER,
            cancellable.as_ref(),
            move |result| {
                let Some(service) = weak
                    .upgrade()
                    .filter(|service| service.current(&name, &callback_owner, epoch, generation))
                else {
                    return;
                };
                match result.and_then(|player| {
                    initial_properties(
                        &player,
                        &[
                            ("PlaybackStatus", glib::VariantTy::STRING),
                            ("Metadata", glib::VariantTy::VARDICT),
                        ],
                    )?;
                    Ok(player)
                }) {
                    Ok(player) => {
                        let root = service.observe(root);
                        let player = service.observe(player);
                        if let Some(entry) = service.imp().entries.borrow_mut().get_mut(&name) {
                            if let Some(task) = entry.deadline.take() {
                                task.abort();
                            }
                            entry.root = Some(root);
                            entry.player = Some(player);
                        }
                        service.refresh();
                    }
                    Err(error) => service.player_failed(&name, &error),
                }
            },
        );
    }

    fn observe(&self, proxy: gio::DBusProxy) -> Proxy {
        let weak = self.downgrade();
        let handler = proxy.connect_local("g-properties-changed", false, move |_| {
            if let Some(service) = weak.upgrade() {
                service.refresh();
            }
            None
        });
        Proxy {
            proxy,
            handler: Some(handler),
        }
    }

    fn current(&self, name: &str, owner: &str, epoch: u64, generation: u64) -> bool {
        self.imp().generation.get() == generation
            && self
                .imp()
                .entries
                .borrow()
                .get(name)
                .is_some_and(|entry| entry.owner == owner && entry.epoch == epoch)
    }

    fn player_failed(&self, name: &str, error: &glib::Error) {
        glib::g_message!(
            "way-shell",
            "Media player {name} unavailable: {error}; retrying"
        );
        let weak = self.downgrade();
        let name = name.to_owned();
        let callback_name = name.clone();
        if let Some(entry) = self.imp().entries.borrow_mut().get_mut(&name) {
            entry.reset();
            entry.retry = Some(
                glib::MainContext::ref_thread_default().spawn_local(async move {
                    glib::timeout_future(Duration::from_secs(1)).await;
                    if let Some(service) = weak.upgrade() {
                        let owner = {
                            let mut entries = service.imp().entries.borrow_mut();
                            let Some(entry) = entries.get_mut(&callback_name) else {
                                return;
                            };
                            entry.retry.take();
                            entry.owner.clone()
                        };
                        let connection = service
                            .imp()
                            .connection
                            .borrow()
                            .as_ref()
                            .map(|source| source.connection.clone());
                        if let Some(connection) = connection.filter(|_| !owner.is_empty()) {
                            service.load_player(&connection, &callback_name, &owner);
                        }
                    }
                }),
            );
        }
        self.refresh();
    }

    fn refresh(&self) {
        let players = self
            .imp()
            .entries
            .borrow()
            .iter()
            .filter_map(|(name, entry)| {
                Some(parse_player(
                    name,
                    &entry.owner,
                    &entry.root.as_ref()?.proxy,
                    &entry.player.as_ref()?.proxy,
                ))
            })
            .collect();
        let available = self.imp().connection.borrow().is_some();
        self.update(MediaState { available, players });
    }

    fn update(&self, state: MediaState) {
        if *self.imp().state.borrow() != state {
            self.imp().state.replace(state);
            self.emit_by_name::<()>("changed", &[]);
        }
    }

    fn reset(&self) {
        if let Some(cancellable) = self.imp().connecting.borrow_mut().take() {
            cancellable.cancel();
        }
        if let Some(task) = self.imp().connect_deadline.borrow_mut().take() {
            task.abort();
        }
        self.imp()
            .generation
            .set(self.imp().generation.get().wrapping_add(1));
        if let Some(task) = self.imp().retry.borrow_mut().take() {
            task.abort();
        }
        self.imp().connection.borrow_mut().take();
        self.imp().entries.borrow_mut().clear();
        self.update(MediaState::default());
    }
}

fn is_player_name(name: &str) -> bool {
    name.strip_prefix("org.mpris.MediaPlayer2.")
        .is_some_and(|suffix| !suffix.is_empty())
}

fn initial_properties(
    proxy: &gio::DBusProxy,
    properties: &[(&str, &glib::VariantTy)],
) -> Result<(), glib::Error> {
    // GDBusProxy construction can succeed after GetAll fails, leaving an empty
    // cache. Retry that incomplete export instead of publishing a blank player
    // that cannot recover until its well-known name is reacquired.
    for (name, value_type) in properties {
        if !proxy
            .cached_property(name)
            .is_some_and(|value| value.is_type(value_type))
        {
            return Err(glib::Error::new(
                gio::IOErrorEnum::InvalidData,
                &format!(
                    "{} returned missing or invalid {name}",
                    proxy.interface_name()
                ),
            ));
        }
    }
    Ok(())
}

fn parse_player(
    name: &str,
    owner: &str,
    root: &gio::DBusProxy,
    player: &gio::DBusProxy,
) -> MediaPlayer {
    let boolean = |proxy: &gio::DBusProxy, name| {
        proxy
            .cached_property(name)
            .and_then(|value| value.get())
            .unwrap_or(false)
    };
    MediaPlayer {
        name: name.to_owned(),
        owner: owner.to_owned(),
        identity: root
            .cached_property("Identity")
            .and_then(|value| value.get()),
        playback_status: match player
            .cached_property("PlaybackStatus")
            .and_then(|value| value.get::<String>())
            .as_deref()
        {
            Some("Playing") => PlaybackStatus::Playing,
            Some("Paused") => PlaybackStatus::Paused,
            Some("Stopped") => PlaybackStatus::Stopped,
            _ => PlaybackStatus::Unknown,
        },
        metadata: parse_metadata(player.cached_property("Metadata").as_ref()),
        capabilities: Capabilities {
            can_control: boolean(player, "CanControl"),
            can_play: boolean(player, "CanPlay"),
            can_pause: boolean(player, "CanPause"),
            can_go_next: boolean(player, "CanGoNext"),
            can_go_previous: boolean(player, "CanGoPrevious"),
            can_raise: boolean(root, "CanRaise"),
        },
    }
}

fn parse_metadata(metadata: Option<&glib::Variant>) -> Metadata {
    let Some(properties) = metadata.and_then(|value| value.get::<HashMap<String, glib::Variant>>())
    else {
        return Metadata::default();
    };
    let string = |name| properties.get(name).and_then(|value| value.get());
    Metadata {
        title: string("xesam:title"),
        album: string("xesam:album"),
        art_url: string("mpris:artUrl"),
        artists: properties
            .get("xesam:artist")
            .and_then(|value| value.get())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_replaced_and_types_are_checked() {
        let complete = HashMap::from([
            ("xesam:title", "Title".to_variant()),
            ("xesam:album", "Album".to_variant()),
            ("mpris:artUrl", "file:///cover.png".to_variant()),
            ("xesam:artist", vec!["One", "Two"].to_variant()),
        ])
        .to_variant();
        let parsed = parse_metadata(Some(&complete));
        assert_eq!(parsed.title.as_deref(), Some("Title"));
        assert_eq!(parsed.artist().as_deref(), Some("One, Two"));
        let wrong = HashMap::from([
            ("xesam:title", 42_i32.to_variant()),
            ("xesam:album", vec!["Album"].to_variant()),
            ("mpris:artUrl", false.to_variant()),
            ("xesam:artist", "One".to_variant()),
        ])
        .to_variant();
        assert_eq!(parse_metadata(Some(&wrong)), Metadata::default());
        assert_eq!(parse_metadata(None), Metadata::default());
        assert_eq!(
            parse_metadata(Some(&"bad".to_variant())),
            Metadata::default()
        );
        assert_eq!(
            parse_metadata(Some(&HashMap::<String, glib::Variant>::new().to_variant())),
            Metadata::default()
        );
        assert_eq!(PlaybackStatus::default(), PlaybackStatus::Unknown);
    }

    #[test]
    fn only_mpris_player_names_are_discovered() {
        assert!(is_player_name("org.mpris.MediaPlayer2.player"));
        assert!(is_player_name("org.mpris.MediaPlayer2.player.instance1"));
        assert!(!is_player_name("org.mpris.MediaPlayer2"));
        assert!(!is_player_name("org.mpris.MediaPlayer2."));
        assert!(!is_player_name("org.mpris.MediaPlayer2Impostor"));
    }
}
