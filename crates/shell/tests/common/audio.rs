use std::{
    os::unix::{fs::DirBuilderExt, fs::FileTypeExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use wireplumber::{
    Core,
    core::ObjectFeatures,
    prelude::*,
    pw::{Direction, Link, LinkState, Node, NodeState, Properties},
};
pub struct Daemon {
    child: Option<Child>,
    pulse_child: Option<Child>,
    runtime: PathBuf,
}
impl Daemon {
    pub fn new() -> Self {
        static NEXT_DAEMON: AtomicU64 = AtomicU64::new(0);
        let runtime = std::env::temp_dir().join(format!(
            "way-shell-audio-{}-{}",
            std::process::id(),
            NEXT_DAEMON.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&runtime)
            .unwrap();
        let mut daemon = Self {
            child: None,
            pulse_child: None,
            runtime,
        };
        daemon.start();
        daemon
    }
    pub fn start(&mut self) {
        assert!(self.child.is_none(), "private PipeWire is already started");
        self.child = Some(
            self.command("pipewire")
                .args([
                    "-c",
                    concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../../tests/fixtures/pipewire.conf"
                    ),
                ])
                .spawn()
                .unwrap(),
        );
        wait_socket(
            self.child.as_mut().unwrap(),
            &self.runtime.join("pipewire-0"),
            &self.runtime.join("pipewire.log"),
        );
    }
    pub fn remote(&self) -> String {
        self.runtime.join("pipewire-0").to_str().unwrap().to_owned()
    }
    #[allow(dead_code)] // The bridge inventory tests also include this helper.
    pub fn start_pulse(&mut self) {
        assert!(self.child.is_some(), "private PipeWire is not started");
        assert!(
            self.pulse_child.is_none(),
            "private Pulse is already started"
        );
        std::fs::create_dir_all(self.runtime.join("pulse")).unwrap();
        self.pulse_child = Some(
            self.command("pipewire-pulse")
                .args([
                    "-c",
                    concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../../tests/fixtures/pipewire-pulse.conf"
                    ),
                ])
                .env("PIPEWIRE_REMOTE", self.remote())
                .spawn()
                .unwrap(),
        );
        wait_socket(
            self.pulse_child.as_mut().unwrap(),
            &self.runtime.join("pulse/native"),
            &self.runtime.join("pipewire-pulse.log"),
        );
    }
    #[allow(dead_code)]
    pub fn pulse_server(&self) -> String {
        format!("unix:{}", self.runtime.join("pulse/native").display())
    }
    pub fn stop_pulse(&mut self) {
        stop_child(&mut self.pulse_child);
        let _ = std::fs::remove_file(self.runtime.join("pulse/native"));
        let _ = std::fs::remove_file(self.runtime.join("pulse/native.lock"));
    }
    pub fn stop(&mut self) {
        self.stop_pulse();
        stop_child(&mut self.child);
        let _ = std::fs::remove_file(self.runtime.join("pipewire-0"));
        let _ = std::fs::remove_file(self.runtime.join("pipewire-0.lock"));
    }
    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("PIPEWIRE_RUNTIME_DIR", &self.runtime)
            .env("PULSE_RUNTIME_PATH", self.runtime.join("pulse"))
            .env("XDG_CONFIG_HOME", self.runtime.join("config"))
            .env_remove("PIPEWIRE_CONFIG_DIR")
            .env_remove("PIPEWIRE_CONFIG_PREFIX")
            .env_remove("PIPEWIRE_CONFIG_NAME")
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(self.runtime.join(format!("{program}.log"))).unwrap());
        command
    }
}
fn stop_child(child: &mut Option<Child>) {
    if let Some(mut child) = child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}
fn wait_socket(child: &mut Child, socket: &Path, log: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            child.try_wait().unwrap().is_none() && Instant::now() < deadline,
            "private audio daemon failed to create {}: {}",
            socket.display(),
            std::fs::read_to_string(log).unwrap_or_default()
        );
        if socket
            .metadata()
            .is_ok_and(|metadata| metadata.file_type().is_socket())
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.runtime);
    }
}
#[track_caller]
pub fn wait(context: &glib::MainContext, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(Instant::now() < deadline, "audio fixture timed out");
        context.block_on(glib::timeout_future(Duration::from_millis(10)));
    }
}

// Shared with the bridge; routing fixtures do not require these graph helpers.
#[allow(dead_code)]
pub fn node(context: &glib::MainContext, core: &Core, name: &str, class: &str) -> Node {
    let properties = Properties::new();
    for (key, value) in [
        ("factory.name", "support.null-audio-sink"),
        ("node.name", name),
        ("node.description", name),
        ("media.class", class),
        ("audio.position", "[ FL FR ]"),
        // PipeWire 1.4 needs a fixed format/rate to create the DSP monitor ports.
        ("audio.format", "F32P"),
        ("audio.rate", "48000"),
        (
            "adapter.auto-port-config",
            "{ mode = dsp monitor = true position = preserve }",
        ),
    ] {
        properties.insert(key, value);
    }
    let node = Node::from_factory(core, "adapter", Some(properties)).unwrap();
    context
        .block_on(node.activate_future(ObjectFeatures::ALL))
        .unwrap();
    node
}

#[allow(dead_code)]
pub fn connect_nodes(
    context: &glib::MainContext,
    core: &Core,
    output: &Node,
    input: &Node,
) -> Link {
    let ports = || {
        let output = output
            .ports()
            .into_iter()
            .find(|port| port.direction() == Direction::Output);
        let input = input
            .ports()
            .into_iter()
            .find(|port| port.direction() == Direction::Input);
        output
            .zip(input)
            .map(|(output, input)| (output.bound_id(), input.bound_id()))
    };
    wait(context, || ports().is_some());
    let (output_port, input_port) = ports().unwrap();
    let properties = Properties::new();
    for (key, value) in [
        ("link.output.node", output.bound_id()),
        ("link.output.port", output_port),
        ("link.input.node", input.bound_id()),
        ("link.input.port", input_port),
    ] {
        properties.insert(key, value);
    }
    let link = Link::from_factory(core, "link-factory", Some(properties)).unwrap();
    context
        .block_on(link.activate_future(ObjectFeatures::ALL))
        .unwrap();
    wait(context, || {
        link.state_result().expect("private audio link failed") == LinkState::Active
            && [output, input].into_iter().all(|node| {
                node.state_result().expect("private audio node failed") == NodeState::Running
            })
    });
    link
}
