use glib::prelude::*;
use std::time::Duration;
use way_shell::services::audio::{AudioService, NodeKind};
use wireplumber::{
    Core, InitFlags,
    core::ObjectFeatures,
    local::ImplMetadata,
    plugin::{Plugin, PluginFeatures},
    prelude::*,
    pw::{Link, Node, Properties},
};

#[path = "common/audio.rs"]
mod fixture;
use fixture::*;

fn node(context: &glib::MainContext, core: &Core, name: &str, class: &str) -> Node {
    let properties = Properties::new();
    for (key, value) in [
        ("factory.name", "support.null-audio-sink"),
        ("node.name", name),
        ("node.description", name),
        ("media.class", class),
        ("audio.position", "[ FL FR ]"),
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

#[test]
fn inventory_defaults_removal_restart_and_ownership() {
    Core::init_with_flags(InitFlags::PIPEWIRE);
    let mut daemon = Daemon::new();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let service = AudioService::with_remote(&daemon.remote());
            wait(&context, || service.state().available);
            let state = service.state();
            assert!(
                state
                    .nodes
                    .iter()
                    .any(|n| n.name == "way-shell-test-sink" && n.kind == NodeKind::Sink)
            );
            assert_eq!(state.default_sink, None);
            let properties = Properties::new();
            properties.insert("remote.name", daemon.remote());
            let core = Core::new(Some(&context), None, Some(properties));
            context.block_on(core.connect_future()).unwrap();
            let capture = node(&context, &core, "test-capture", "Audio/Source");
            let playback = node(&context, &core, "test-playback", "Audio/Sink");
            let stream = node(&context, &core, "test-stream", "Stream/Output/Audio");
            let metadata = ImplMetadata::with_properties(&core, Some("default"), None);
            context
                .block_on(metadata.activate_future(ObjectFeatures::ALL))
                .unwrap();
            metadata.set(
                0,
                Some("default.audio.sink"),
                Some("Spa:String:JSON"),
                Some(r#"{"name":"test-playback"}"#),
            );
            metadata.set(
                0,
                Some("default.audio.source"),
                Some("Spa:String:JSON"),
                Some(r#"{"name":"test-capture"}"#),
            );
            wait(&context, || {
                service.state().default_sink == Some(playback.bound_id())
                    && service.state().default_source == Some(capture.bound_id())
            });
            wait(&context, || {
                service
                    .state()
                    .nodes
                    .iter()
                    .any(|n| n.id == stream.bound_id() && n.kind == NodeKind::OutputStream)
            });
            assert!(
                service
                    .state()
                    .ports
                    .iter()
                    .any(|p| p.node == capture.bound_id())
            );
            // Real plugin property notifications, using a separate client as the writer.
            context
                .block_on(core.load_component_future(
                    Some("libwireplumber-module-mixer-api".into()),
                    "module",
                    None,
                    None,
                ))
                .unwrap();
            let mixer = Plugin::find(&core, "mixer-api").unwrap();
            context
                .block_on(mixer.activate_future(PluginFeatures::ENABLED))
                .unwrap();
            let values = std::collections::HashMap::from([
                ("volume", 0.125_f64.to_variant()),
                ("mute", true.to_variant()),
            ])
            .to_variant();
            assert!(mixer.emit_by_name::<bool>("set-volume", &[&playback.bound_id(), &values]));
            wait(&context, || {
                service
                    .state()
                    .node(playback.bound_id())
                    .and_then(|n| n.volume.as_ref())
                    .is_some_and(|v| v.mute && (v.volume - 0.5).abs() < 0.0001)
            });
            let observed = service.state();
            assert!(
                observed
                    .node(playback.bound_id())
                    .unwrap()
                    .volume
                    .as_ref()
                    .unwrap()
                    .step
                    > 0.0
            );
            assert_eq!(
                observed
                    .node(playback.bound_id())
                    .unwrap()
                    .volume
                    .as_ref()
                    .unwrap()
                    .channels
                    .len(),
                2
            );
            let output = observed
                .ports
                .iter()
                .find(|p| {
                    p.node == capture.bound_id()
                        && p.direction == way_shell::services::audio::Direction::Output
                })
                .unwrap();
            let input = observed
                .ports
                .iter()
                .find(|p| {
                    p.node == playback.bound_id()
                        && p.direction == way_shell::services::audio::Direction::Input
                })
                .unwrap();
            let props = Properties::new();
            for (key, value) in [
                ("link.output.node", output.node),
                ("link.output.port", output.id),
                ("link.input.node", input.node),
                ("link.input.port", input.id),
            ] {
                props.insert(key, value);
            }
            let link = Link::from_factory(&core, "link-factory", Some(props)).unwrap();
            context
                .block_on(link.activate_future(ObjectFeatures::ALL))
                .unwrap();
            wait(&context, || {
                service.state().links.iter().any(|l| {
                    l.output_node == capture.bound_id() && l.input_node == playback.bound_id()
                })
            });
            wait(&context, || service.state().microphone_active());
            drop(link);
            wait(&context, || service.state().links.is_empty());
            drop(mixer);
            let snapshot = service.state();
            metadata.set(
                0,
                Some("default.audio.sink"),
                Some("Spa:String:JSON"),
                Some(r#"{"name":"way-shell-test-sink"}"#),
            );
            wait(&context, || {
                service
                    .state()
                    .default_sink
                    .is_some_and(|id| id != playback.bound_id())
            });
            let capture_id = capture.bound_id();
            drop(capture);
            wait(&context, || {
                service.state().default_source.is_none()
                    && !service.state().nodes.iter().any(|n| n.id == capture_id)
            });
            assert!(snapshot.nodes.iter().any(|n| n.name == "test-capture"));
            stream.request_destroy();
            playback.request_destroy();
            drop(metadata);
            drop(stream);
            drop(playback);
            core.disconnect();
            drop(core);
            daemon.stop();
            wait(&context, || !service.state().available);
            assert!(service.state().nodes.is_empty());
            assert!(service.state().ports.is_empty());
            assert!(service.state().links.is_empty());
            daemon.start();
            wait(&context, || {
                service.state().available && !service.state().nodes.is_empty()
            });
            let weak = service.downgrade();
            service.stop();
            drop(service);
            assert!(weak.upgrade().is_none());
            let mut previous = None;
            for _ in 0..3 {
                let service = AudioService::with_remote(&daemon.remote());
                wait(&context, || service.state().available);
                let weak = service.downgrade();
                drop(service);
                context.block_on(glib::timeout_future(Duration::from_millis(40)));
                assert!(weak.upgrade().is_none());
                let descriptors = std::fs::read_dir("/proc/self/fd").unwrap().count();
                if let Some(before) = previous {
                    assert_eq!(before, descriptors);
                }
                previous = Some(descriptors);
            }
            // Destruction while connection and plugin activation are still pending.
            for _ in 0..3 {
                let service = AudioService::with_remote(&daemon.remote());
                let weak = service.downgrade();
                drop(service);
                assert!(weak.upgrade().is_none());
            }
            context.block_on(glib::timeout_future(Duration::from_millis(40)));
        })
        .unwrap();
}
