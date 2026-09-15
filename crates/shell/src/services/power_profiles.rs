//! The shared power-profiles-daemon / tuned-ppd D-Bus contract.
use crate::platform::bus_watch::Watcher;
use gio::prelude::*;
use glib::subclass::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    sync::OnceLock,
    time::Duration,
};

const NAME: &str = "net.hadess.PowerProfiles";
const PATH: &str = "/net/hadess/PowerProfiles";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfilesState {
    pub profiles: Vec<String>,
    pub active: Option<String>,
}
impl ProfilesState {
    pub fn available(&self) -> bool {
        self.active.is_some() && !self.profiles.is_empty()
    }
}

pub fn profile_icon(profile: Option<&str>) -> &'static str {
    match profile {
        Some("performance") => "power-profile-performance-symbolic",
        Some("power-saver") => "power-profile-power-saver-symbolic",
        _ => "power-profile-balanced-symbolic",
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
mod imp {
    use super::*;
    #[derive(Default)]
    pub struct PowerProfilesService {
        pub state: RefCell<ProfilesState>,
        pub(super) watcher: RefCell<Option<Watcher>>,
        pub owner: RefCell<Option<(gio::DBusConnection, String)>>,
        pub(super) proxy: RefCell<Option<Proxy>>,
        pub cancellable: RefCell<Option<gio::Cancellable>>,
        pub retry: RefCell<Option<glib::JoinHandle<()>>>,
        pub generation: Cell<u64>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for PowerProfilesService {
        const NAME: &'static str = "WayShellPowerProfilesService";
        type Type = super::PowerProfilesService;
    }
    impl ObjectImpl for PowerProfilesService {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("changed").build()])
        }
        fn dispose(&self) {
            self.watcher.borrow_mut().take();
            self.obj().reset();
            self.owner.borrow_mut().take();
        }
    }
}
glib::wrapper! { pub struct PowerProfilesService(ObjectSubclass<imp::PowerProfilesService>); }
impl Default for PowerProfilesService {
    fn default() -> Self {
        Self::new()
    }
}
impl PowerProfilesService {
    pub fn new() -> Self {
        let service: Self = glib::Object::new();
        let appeared = service.downgrade();
        let vanished = service.downgrade();
        let watcher = Watcher::on_bus(
            gio::BusType::System,
            NAME,
            move |connection, owner| {
                if let Some(service) = appeared.upgrade() {
                    service.appeared(connection, owner);
                }
            },
            move || {
                if let Some(service) = vanished.upgrade() {
                    service.vanished();
                }
            },
        );
        service.imp().watcher.replace(Some(watcher));
        service
    }
    pub fn on_connection(connection: &gio::DBusConnection) -> Self {
        let service: Self = glib::Object::new();
        let appeared = service.downgrade();
        let vanished = service.downgrade();
        let watcher = Watcher::on_connection(
            connection,
            NAME,
            move |connection, owner| {
                if let Some(service) = appeared.upgrade() {
                    service.appeared(connection, owner);
                }
            },
            move || {
                if let Some(service) = vanished.upgrade() {
                    service.vanished();
                }
            },
        );
        service.imp().watcher.replace(Some(watcher));
        service
    }
    pub fn state(&self) -> ProfilesState {
        self.imp().state.borrow().clone()
    }
    pub fn set_profile(&self, profile: &str, done: impl FnOnce(Result<(), glib::Error>) + 'static) {
        let state = self.state();
        let proxy = self.imp().proxy.borrow().as_ref().map(|p| p.proxy.clone());
        let Some(proxy) = proxy.filter(|_| state.available()) else {
            done(Err(glib::Error::new(
                gio::IOErrorEnum::NotConnected,
                "Power profiles service is unavailable",
            )));
            return;
        };
        if !state.profiles.iter().any(|value| value == profile) {
            done(Err(glib::Error::new(
                gio::IOErrorEnum::InvalidArgument,
                "Power profile is not offered by this provider",
            )));
            return;
        }
        let parameters = (NAME, "ActiveProfile", profile.to_variant()).to_variant();
        let weak = self.downgrade();
        let generation = self.imp().generation.get();
        proxy.call(
            "org.freedesktop.DBus.Properties.Set",
            Some(&parameters),
            gio::DBusCallFlags::NONE,
            2_000,
            self.imp().cancellable.borrow().as_ref(),
            move |result| {
                let current = weak
                    .upgrade()
                    .is_some_and(|s| s.imp().generation.get() == generation);
                if current {
                    done(result.map(|_| ()));
                } else {
                    done(Err(glib::Error::new(
                        gio::IOErrorEnum::Cancelled,
                        "Power profile provider changed during the request",
                    )));
                }
            },
        );
    }
    fn appeared(&self, connection: gio::DBusConnection, owner: &str) {
        self.reset();
        self.imp()
            .owner
            .replace(Some((connection.clone(), owner.to_owned())));
        let generation = self.imp().generation.get();
        let cancellable = gio::Cancellable::new();
        self.imp().cancellable.replace(Some(cancellable.clone()));
        let weak = self.downgrade();
        gio::DBusProxy::new(
            &connection,
            gio::DBusProxyFlags::DO_NOT_AUTO_START
                | gio::DBusProxyFlags::GET_INVALIDATED_PROPERTIES,
            None,
            Some(owner),
            PATH,
            NAME,
            Some(&cancellable),
            move |result| {
                let Some(service) = weak
                    .upgrade()
                    .filter(|s| s.imp().generation.get() == generation)
                else {
                    return;
                };
                match result {
                    Ok(proxy) => {
                        let weak = service.downgrade();
                        let handler =
                            proxy.connect_local("g-properties-changed", false, move |_| {
                                if let Some(service) = weak.upgrade() {
                                    service.refresh();
                                }
                                None
                            });
                        service.imp().proxy.replace(Some(Proxy {
                            proxy,
                            handler: Some(handler),
                        }));
                        service.refresh();
                    }
                    Err(error) => service.failed(&error),
                }
            },
        );
    }
    fn refresh(&self) {
        let state = self
            .imp()
            .proxy
            .borrow()
            .as_ref()
            .and_then(|p| parse_state(&p.proxy));
        if let Some(state) = state {
            self.update(state);
        } else {
            self.failed(&glib::Error::new(
                gio::IOErrorEnum::InvalidData,
                "Power profiles provider returned invalid properties",
            ));
        }
    }
    fn update(&self, state: ProfilesState) {
        if *self.imp().state.borrow() != state {
            self.imp().state.replace(state);
            self.emit_by_name::<()>("changed", &[]);
        }
    }
    fn failed(&self, error: &glib::Error) {
        glib::g_message!("way-shell", "Power profiles unavailable: {error}; retrying");
        self.reset();
        let weak = self.downgrade();
        let retry = glib::MainContext::ref_thread_default().spawn_local(async move {
            glib::timeout_future(Duration::from_secs(1)).await;
            if let Some(service) = weak.upgrade() {
                service.imp().retry.borrow_mut().take();
                let owner = service.imp().owner.borrow().clone();
                if let Some((connection, owner)) = owner {
                    service.appeared(connection, &owner);
                }
            }
        });
        self.imp().retry.replace(Some(retry));
    }
    fn vanished(&self) {
        self.imp().owner.borrow_mut().take();
        self.reset();
        glib::g_message!(
            "way-shell",
            "Power profiles unavailable; controls wait for a provider"
        );
    }
    fn reset(&self) {
        self.imp()
            .generation
            .set(self.imp().generation.get().wrapping_add(1));
        if let Some(retry) = self.imp().retry.borrow_mut().take() {
            retry.abort();
        }
        if let Some(cancellable) = self.imp().cancellable.borrow_mut().take() {
            cancellable.cancel();
        }
        self.imp().proxy.borrow_mut().take();
        self.update(ProfilesState::default());
    }
}
fn parse_state(proxy: &gio::DBusProxy) -> Option<ProfilesState> {
    let active: String = proxy.cached_property("ActiveProfile")?.get()?;
    let profiles: Vec<HashMap<String, glib::Variant>> = proxy.cached_property("Profiles")?.get()?;
    let mut names = Vec::with_capacity(profiles.len());
    for mut profile in profiles {
        let name: String = profile.remove("Profile")?.get()?;
        if name.is_empty() {
            return None;
        }
        if !names.contains(&name) {
            names.push(name);
        }
    }
    Some(ProfilesState {
        profiles: names,
        active: (!active.is_empty()).then_some(active),
    })
}
