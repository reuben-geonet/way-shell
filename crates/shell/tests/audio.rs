use glib::prelude::*;
use std::time::Duration;
use way_shell::services::audio::{AudioError, AudioService, NodeKind};
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

fn controls(context: &glib::MainContext, service: &AudioService, id: u32) {
    let original = service.state().node(id).unwrap().volume.clone().unwrap();
    let channels = original
        .channels
        .iter()
        .map(|channel| (channel.index, channel.channel.clone()))
        .collect::<Vec<_>>();
    assert_eq!(channels.len(), 2);
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.01, 1.01] {
        assert_eq!(
            service.set_volume(id, invalid),
            Err(AudioError::InvalidVolume)
        );
    }
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            service.change_volume(id, invalid),
            Err(AudioError::InvalidDelta)
        );
    }
    assert_eq!(
        service.set_volume(u32::MAX, 0.5),
        Err(AudioError::NodeNotFound(u32::MAX))
    );
    assert_eq!(
        service.set_muted(u32::MAX, true),
        Err(AudioError::NodeNotFound(u32::MAX))
    );
    assert_eq!(
        service.change_volume(u32::MAX, 0.05),
        Err(AudioError::NodeNotFound(u32::MAX))
    );

    let observed = |volume: f64, mute: bool| {
        let state = service.state();
        let actual = state.node(id).unwrap().volume.as_ref().unwrap();
        assert_eq!(
            actual
                .channels
                .iter()
                .map(|c| (c.index, c.channel.clone()))
                .collect::<Vec<_>>(),
            channels
        );
        actual.mute == mute && (actual.volume - volume).abs() < 0.0001
    };
    service.set_muted(id, !original.mute).unwrap();
    wait(context, || observed(original.volume, !original.mute));
    let state = service.state();
    let changed = state.node(id).unwrap().volume.as_ref().unwrap();
    assert_eq!(changed.channels, original.channels);
    service.set_muted(id, original.mute).unwrap();
    wait(context, || observed(original.volume, original.mute));
    assert_eq!(
        service
            .state()
            .node(id)
            .unwrap()
            .volume
            .as_ref()
            .unwrap()
            .channels,
        original.channels
    );
    service.set_volume(id, 0.73).unwrap();
    wait(context, || observed(0.73, original.mute));
    // Preserve the existing master-volume operation: WirePlumber sets every channel
    // to the requested level while retaining the channel indices and positions.
    assert!(
        service
            .state()
            .node(id)
            .unwrap()
            .volume
            .as_ref()
            .unwrap()
            .channels
            .iter()
            .all(|channel| (channel.volume - 0.73).abs() < 0.0001)
    );
    service.set_muted(id, false).unwrap();
    wait(context, || observed(0.73, false));
    service.set_muted(id, true).unwrap();
    wait(context, || observed(0.73, true));
    service.set_muted(id, false).unwrap();
    wait(context, || observed(0.73, false));
    service.set_volume(id, 0.98).unwrap();
    wait(context, || observed(0.98, false));
    service.change_volume(id, 0.05).unwrap();
    wait(context, || observed(1.0, false));
    service.change_volume(id, 0.05).unwrap();
    assert!(observed(1.0, false));
    service.set_volume(id, 0.02).unwrap();
    wait(context, || observed(0.02, false));
    service.change_volume(id, -0.05).unwrap();
    wait(context, || observed(0.0, false));
    service.change_volume(id, -0.05).unwrap();
    assert!(observed(0.0, false));
    service.set_volume(id, 0.5).unwrap();
    wait(context, || observed(0.5, false));
}

fn amplified_controls(
    context: &glib::MainContext,
    service: &AudioService,
    mixer: &Plugin,
    id: u32,
) {
    // Another mixer can amplify beyond the shell's 100% setting limit. Preserve
    // the existing increment/decrement behavior for those externally set levels.
    let values = std::collections::HashMap::from([("volume", 1.728_f64.to_variant())]).to_variant();
    assert!(mixer.emit_by_name::<bool>("set-volume", &[&id, &values]));
    let observed = || {
        service
            .state()
            .node(id)
            .unwrap()
            .volume
            .as_ref()
            .unwrap()
            .volume
    };
    wait(context, || (observed() - 1.2).abs() < 0.0001);
    assert_eq!(service.set_volume(id, 1.2), Err(AudioError::InvalidVolume));
    service.change_volume(id, 0.0).unwrap();
    service.change_volume(id, 0.05).unwrap();
    // Dispatch any native update so a wrongly clamped request cannot pass by
    // observing the still-unchanged local snapshot immediately after the call.
    context.block_on(glib::timeout_future(Duration::from_millis(50)));
    assert!((observed() - 1.2).abs() < 0.0001);
    service.change_volume(id, -0.05).unwrap();
    wait(context, || (observed() - 1.15).abs() < 0.0001);
    service.change_volume(id, -2.0).unwrap();
    wait(context, || observed().abs() < 0.0001);
    service.set_volume(id, 0.5).unwrap();
    wait(context, || (observed() - 0.5).abs() < 0.0001);
}

#[test]
fn inventory_defaults_removal_restart_and_ownership() {
    Core::init_with_flags(InitFlags::PIPEWIRE);
    let mut daemon = Daemon::new();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let service = AudioService::with_remote(&daemon.remote());
            assert_eq!(service.set_volume(0, 0.5), Err(AudioError::Unavailable));
            assert_eq!(service.set_muted(0, true), Err(AudioError::Unavailable));
            assert_eq!(service.change_volume(0, 0.05), Err(AudioError::Unavailable));
            wait(&context, || service.state().available);
            let state = service.state();
            assert!(state.nodes.iter().all(|node| node.serial > 0));
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
            // Fixture inputs below are linear amplitudes. A system WirePlumber
            // configuration may default its separately loaded mixer to cubic.
            let scale =
                glib::EnumClass::with_type(mixer.find_property("scale").unwrap().value_type())
                    .unwrap()
                    .to_value(0)
                    .unwrap();
            mixer.set_property_from_value("scale", &scale);
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
            let channel_values = std::collections::HashMap::from([
                (
                    "0",
                    std::collections::HashMap::from([
                        ("volume", 0.125_f64.to_variant()),
                        ("channel", "FL".to_variant()),
                    ])
                    .to_variant(),
                ),
                (
                    "1",
                    std::collections::HashMap::from([
                        ("volume", 0.064_f64.to_variant()),
                        ("channel", "FR".to_variant()),
                    ])
                    .to_variant(),
                ),
            ])
            .to_variant();
            let values =
                std::collections::HashMap::from([("channelVolumes", channel_values)]).to_variant();
            assert!(mixer.emit_by_name::<bool>("set-volume", &[&playback.bound_id(), &values]));
            wait(&context, || {
                service
                    .state()
                    .node(playback.bound_id())
                    .and_then(|n| n.volume.as_ref())
                    .is_some_and(|v| {
                        v.channels.len() == 2
                            && (v.channels[0].volume - 0.5).abs() < 0.0001
                            && (v.channels[1].volume - 0.4).abs() < 0.0001
                    })
            });
            controls(&context, &service, playback.bound_id());
            amplified_controls(&context, &service, &mixer, playback.bound_id());
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
            assert_eq!(
                service.set_volume(capture_id, 0.5),
                Err(AudioError::NodeNotFound(capture_id))
            );
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
            assert_eq!(service.set_volume(0, 0.5), Err(AudioError::Unavailable));
            assert_eq!(service.set_muted(0, true), Err(AudioError::Unavailable));
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
