//! Application-owned audio inventory. WirePlumber objects remain inside this module.
use glib::{prelude::*, subclass::prelude::*};
use std::{
    cell::{Cell, RefCell},
    sync::OnceLock,
    time::Duration,
};
use wireplumber::{Core, InitFlags};
mod controls;
mod native;
mod routing;
pub use controls::AudioError;
pub use routing::RouteError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Sink,
    Source,
    InputStream,
    OutputStream,
}
impl NodeKind {
    pub fn media_class(self) -> &'static str {
        match self {
            Self::Sink => "Audio/Sink",
            Self::Source => "Audio/Source",
            Self::InputStream => "Stream/Input/Audio",
            Self::OutputStream => "Stream/Output/Audio",
        }
    }
    fn from_class(class: &str) -> Option<Self> {
        match class {
            "Audio/Sink" => Some(Self::Sink),
            "Audio/Source" => Some(Self::Source),
            "Stream/Input/Audio" => Some(Self::InputStream),
            "Stream/Output/Audio" => Some(Self::OutputStream),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeState {
    Error,
    Creating,
    Suspended,
    Idle,
    Running,
    Unknown,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelVolume {
    pub index: u32,
    pub channel: Option<String>,
    pub volume: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Volume {
    pub volume: f64,
    pub mute: bool,
    pub step: f64,
    pub base: f64,
    pub channels: Vec<ChannelVolume>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct AudioNode {
    pub id: u32,
    /// Monotonic PipeWire identity; unlike the bound id, this is not reused after hotplug.
    pub serial: u64,
    pub kind: NodeKind,
    pub name: String,
    pub description: String,
    pub nickname: String,
    pub application: String,
    pub media: String,
    pub state: NodeState,
    pub volume: Option<Volume>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Input,
    Output,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioPort {
    pub id: u32,
    pub node: u32,
    pub index: u32,
    pub direction: Direction,
    pub channel: Option<String>,
    pub monitor: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioLink {
    pub id: u32,
    pub output_node: u32,
    pub output_port: u32,
    pub input_node: u32,
    pub input_port: u32,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioState {
    pub available: bool,
    pub nodes: Vec<AudioNode>,
    pub ports: Vec<AudioPort>,
    pub links: Vec<AudioLink>,
    pub default_sink: Option<u32>,
    pub default_source: Option<u32>,
}
impl AudioState {
    pub fn node(&self, id: u32) -> Option<&AudioNode> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn microphone_active(&self) -> bool {
        self.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Source && n.state == NodeState::Running)
    }
}
mod imp {
    use super::*;
    #[derive(Default)]
    pub struct AudioService {
        pub state: RefCell<AudioState>,
        pub remote: RefCell<Option<String>>,
        pub pulse_server: RefCell<Option<String>>,
        pub routing_enabled: Cell<bool>,
        pub(super) router: RefCell<routing::Router>,
        pub(super) session: RefCell<Option<native::Session>>,
        pub task: RefCell<Option<glib::JoinHandle<()>>>,
        pub refresh_task: RefCell<Option<glib::JoinHandle<()>>>,
        pub generation: Cell<u64>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for AudioService {
        const NAME: &'static str = "WayShellAudioService";
        type Type = super::AudioService;
    }
    impl ObjectImpl for AudioService {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("changed").build()])
        }
        fn dispose(&self) {
            self.obj().reset();
        }
    }
}
glib::wrapper! { pub struct AudioService(ObjectSubclass<imp::AudioService>); }
impl Default for AudioService {
    fn default() -> Self {
        Self::new()
    }
}
impl AudioService {
    pub fn new() -> Self {
        Self::create(None, None, true)
    }
    /// Connect inventory to a named or absolute PipeWire socket. Routing stays disabled;
    /// use `with_remotes` to select an explicit PulseAudio server as well.
    pub fn with_remote(remote: &str) -> Self {
        Self::create(Some(remote.to_owned()), None, false)
    }
    /// Connect both protocols to explicit servers, for isolated operation/tests.
    pub fn with_remotes(remote: &str, pulse_server: &str) -> Self {
        Self::create(Some(remote.to_owned()), Some(pulse_server.to_owned()), true)
    }
    fn create(remote: Option<String>, pulse_server: Option<String>, routing_enabled: bool) -> Self {
        static INITIALIZED: OnceLock<()> = OnceLock::new();
        INITIALIZED.get_or_init(|| Core::init_with_flags(InitFlags::PIPEWIRE));
        let service: Self = glib::Object::new();
        service.imp().remote.replace(remote);
        service.imp().pulse_server.replace(pulse_server);
        service.imp().routing_enabled.set(routing_enabled);
        service.begin(false);
        service
    }
    pub fn state(&self) -> AudioState {
        self.imp().state.borrow().clone()
    }
    pub fn stop(&self) {
        self.reset();
        self.publish(AudioState::default());
    }
    fn reset(&self) {
        self.imp().router.replace(routing::Router::default());
        self.imp()
            .generation
            .set(self.imp().generation.get().wrapping_add(1));
        let refresh = self.imp().refresh_task.borrow_mut().take();
        if let Some(refresh) = refresh {
            refresh.abort();
        }
        let task = self.imp().task.borrow_mut().take();
        if let Some(task) = task {
            task.abort();
        }
        let session = self.imp().session.borrow_mut().take();
        drop(session);
    }
    fn begin(&self, delay: bool) {
        let weak = self.downgrade();
        let generation = self.imp().generation.get();
        let remote = self.imp().remote.borrow().clone();
        let task = glib::MainContext::ref_thread_default().spawn_local(async move {
            if delay {
                glib::timeout_future(Duration::from_secs(1)).await;
            }
            if weak
                .upgrade()
                .is_none_or(|s| s.imp().generation.get() != generation)
            {
                return;
            }
            let result = glib::future_with_timeout(
                Duration::from_secs(10),
                native::Session::connect(remote.as_deref()),
            )
            .await;
            let Some(service) = weak
                .upgrade()
                .filter(|s| s.imp().generation.get() == generation)
            else {
                return;
            };
            service.imp().task.borrow_mut().take();
            match result {
                Ok(Ok(mut session)) => {
                    session.subscribe(&service, generation);
                    service.imp().session.replace(Some(session));
                    service.refresh();
                }
                failure => {
                    eprintln!("Audio connection unavailable: {failure:?}; retrying");
                    service.begin(true);
                }
            }
        });
        self.imp().task.replace(Some(task));
    }
    fn disconnected(&self, generation: u64) {
        if self.imp().generation.get() != generation {
            return;
        }
        self.stop();
        eprintln!("Audio connection closed; reconnecting");
        self.begin(true);
    }
    fn refresh(&self) {
        let state = self
            .imp()
            .session
            .borrow_mut()
            .as_mut()
            .map(|s| s.snapshot(self));
        if let Some(state) = state {
            self.publish(state);
        }
    }
    fn schedule_refresh(&self, context: &glib::MainContext) {
        if self.imp().refresh_task.borrow().is_some() {
            return;
        }
        let weak = self.downgrade();
        let generation = self.imp().generation.get();
        // Application subscribers may stop the service. Publish after the native
        // PipeWire dispatch returns, so stop cannot destroy its current stack.
        // The pending task owns no service or native object and reset cancels it.
        let task = context.spawn_local(async move {
            if let Some(service) = weak.upgrade()
                && service.imp().generation.get() == generation
            {
                service.imp().refresh_task.borrow_mut().take();
                service.refresh();
            }
        });
        self.imp().refresh_task.replace(Some(task));
    }
    fn publish(&self, state: AudioState) {
        if *self.imp().state.borrow() != state {
            self.imp().state.replace(state);
            self.emit_by_name::<()>("changed", &[]);
        }
    }
}
