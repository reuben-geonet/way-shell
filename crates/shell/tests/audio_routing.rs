use glib::prelude::*;
use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
    time::Duration,
};
use way_shell::services::audio::{AudioService, NodeKind, RouteError};
use wireplumber::{
    Core, InitFlags,
    core::ObjectFeatures,
    prelude::*,
    pw::{Node, Properties},
};
#[path = "common/audio.rs"]
mod fixture;
#[path = "common/pulse.rs"]
mod pulse_fixture;
use fixture::{Daemon, wait};

fn node(context: &glib::MainContext, core: &Core, name: &str, capture: bool) -> Node {
    let properties = Properties::new();
    for (key, value) in [
        ("factory.name", "support.null-audio-sink"),
        ("node.name", name),
        (
            "media.class",
            if capture {
                "Audio/Source"
            } else {
                "Audio/Sink"
            },
        ),
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
fn core(context: &glib::MainContext, remote: &str) -> Core {
    let properties = Properties::new();
    properties.insert("remote.name", remote);
    let core = Core::new(Some(context), None, Some(properties));
    context.block_on(core.connect_future()).unwrap();
    core
}

#[test]
fn concurrent_playback_capture_failures_cancellation_and_restart() {
    Core::init_with_flags(InitFlags::PIPEWIRE);
    let mut daemon = Daemon::new();
    daemon.start_pulse();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let service = AudioService::with_remotes(&daemon.remote(), &daemon.pulse_server());
            let core = core(&context, &daemon.remote());
            let sink_a = node(&context, &core, "route-sink-a", false);
            let sink_b = node(&context, &core, "route-sink-b", false);
            let source_a = node(&context, &core, "route-source-a", true);
            let source_b = node(&context, &core, "route-source-b", true);
            wait(&context, || {
                service.state().node(source_b.bound_id()).is_some()
            });
            let _policy = pulse_fixture::Policy::new(&daemon.remote());
            let client = pulse_fixture::Client::new(&context, &daemon.pulse_server());
            let playback_a = client.playback("route-playback-a", Some("route-sink-a"));
            let playback_b = client.playback("route-playback-b", Some("route-sink-b"));
            let capture = client.capture("route-capture", Some("route-source-a"));
            let first = playback_a.info();
            let second = playback_b.info();
            let recording = capture.info();
            let first_id = first.node_id.unwrap();
            let second_id = second.node_id.unwrap();
            let recording_id = recording.node_id.unwrap();
            wait(&context, || {
                [first_id, second_id, recording_id]
                    .iter()
                    .all(|id| service.state().node(*id).is_some())
            });
            assert!(first.corked && second.corked && recording.corked);
            assert_eq!(
                service.state().node(first_id).unwrap().serial,
                first.serial.unwrap()
            );
            assert_eq!(
                service.state().node(recording_id).unwrap().kind,
                NodeKind::InputStream
            );

            // Start independent requests before dispatching any query callbacks.
            let first_move = context.spawn_local(service.route(first_id, sink_b.bound_id()));
            let second_move = context.spawn_local(service.route(second_id, sink_a.bound_id()));
            let capture_move =
                context.spawn_local(service.route(recording_id, source_b.bound_id()));
            context.block_on(async {
                first_move.await.unwrap().unwrap();
                second_move.await.unwrap().unwrap();
                capture_move.await.unwrap().unwrap();
            });
            wait(&context, || {
                playback_a.device_name().as_deref() == Some("route-sink-b")
                    && playback_b.device_name().as_deref() == Some("route-sink-a")
                    && capture.device_name().as_deref() == Some("route-source-b")
            });
            assert_ne!(playback_a.info().destination_index, first.destination_index);
            assert_ne!(
                capture.info().destination_index,
                recording.destination_index
            );

            assert_eq!(
                context.block_on(service.route(first_id, source_a.bound_id())),
                Err(RouteError::InvalidEndpoints)
            );
            assert_eq!(
                context.block_on(service.route(u32::MAX, sink_a.bound_id())),
                Err(RouteError::EndpointRemoved)
            );

            // pipewire-pulse also exposes native PipeWire streams for routing.
            let properties = Properties::new();
            properties.insert("factory.name", "support.null-audio-sink");
            properties.insert("node.name", "native-only-stream");
            properties.insert("media.class", "Stream/Output/Audio");
            let native = Node::from_factory(&core, "adapter", Some(properties)).unwrap();
            context
                .block_on(native.activate_future(ObjectFeatures::ALL))
                .unwrap();
            wait(&context, || {
                service.state().node(native.bound_id()).is_some()
            });
            context
                .block_on(service.route(native.bound_id(), sink_a.bound_id()))
                .unwrap();

            // Drop a polled query while callback userdata is still owned by its future.
            {
                let mut pending = pin!(service.route(first_id, sink_a.bound_id()));
                assert!(matches!(
                    pending
                        .as_mut()
                        .poll(&mut Context::from_waker(Waker::noop())),
                    Poll::Pending
                ));
            }
            context.block_on(glib::timeout_future(Duration::from_millis(30)));
            assert_eq!(playback_a.device_name().as_deref(), Some("route-sink-b"));

            // Drop service while a request is in flight: no strong owner cycle.
            let weak = service.downgrade();
            let mut pending = Box::pin(service.route(first_id, sink_a.bound_id()));
            assert!(matches!(
                pending
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Pending
            ));
            drop(service);
            assert!(weak.upgrade().is_none());
            assert_eq!(context.block_on(pending), Err(RouteError::Cancelled));

            let service = AudioService::with_remotes(&daemon.remote(), &daemon.pulse_server());
            wait(&context, || service.state().node(first_id).is_some());
            context
                .block_on(service.route(first_id, sink_a.bound_id()))
                .unwrap();
            daemon.stop_pulse();
            wait(&context, || service.state().node(first_id).is_none());
            assert!(
                service.state().available,
                "Pulse loss must retain PipeWire inventory"
            );
            drop((playback_a, playback_b, capture, client));
            daemon.start_pulse();
            let client = pulse_fixture::Client::new(&context, &daemon.pulse_server());
            let playback = client.playback("route-after-restart", Some("route-sink-a"));
            let id = playback.info().node_id.unwrap();
            wait(&context, || service.state().node(id).is_some());
            context
                .block_on(service.route(id, sink_b.bound_id()))
                .unwrap();
            wait(&context, || {
                playback.device_name().as_deref() == Some("route-sink-b")
            });
            drop((playback, client));
            service.stop();
            drop((native, sink_a, sink_b, source_a, source_b));
            core.disconnect();
        })
        .unwrap();
}

#[test]
fn deadline_absent_server_and_removed_endpoint() {
    use std::os::unix::net::UnixListener;
    Core::init_with_flags(InitFlags::PIPEWIRE);
    let mut daemon = Daemon::new();
    let socket = std::path::PathBuf::from(daemon.remote()).with_file_name("stalled-pulse");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let core = core(&context, &daemon.remote());
            let sink = node(&context, &core, "deadline-sink", false);
            let properties = Properties::new();
            properties.insert("factory.name", "support.null-audio-sink");
            properties.insert("node.name", "deadline-stream");
            properties.insert("media.class", "Stream/Output/Audio");
            let stream = Node::from_factory(&core, "adapter", Some(properties)).unwrap();
            context
                .block_on(stream.activate_future(ObjectFeatures::ALL))
                .unwrap();
            let service =
                AudioService::with_remotes(&daemon.remote(), &format!("unix:{}", socket.display()));
            wait(&context, || {
                service.state().node(stream.bound_id()).is_some()
                    && service.state().node(sink.bound_id()).is_some()
            });
            let started = std::time::Instant::now();
            let pending = service.route(stream.bound_id(), sink.bound_id());
            // Keep the accepted peer open even when the listening path is replaced,
            // so recovery proves the timed-out handshake is explicitly invalidated.
            let peer = std::cell::RefCell::new(None);
            wait(&context, || match listener.accept() {
                Ok((socket, _)) => {
                    peer.replace(Some(socket));
                    true
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
                Err(error) => panic!("accept stalled Pulse client: {error}"),
            });
            assert_eq!(context.block_on(pending), Err(RouteError::Timeout));
            assert!(
                (Duration::from_millis(1900)..Duration::from_secs(4)).contains(&started.elapsed())
            );

            daemon.start_pulse();
            std::fs::remove_file(&socket).unwrap();
            std::os::unix::fs::symlink(
                std::path::PathBuf::from(daemon.remote()).with_file_name("pulse/native"),
                &socket,
            )
            .unwrap();
            // The new server answers, but has no policy metadata for this move.
            assert!(matches!(
                context.block_on(service.route(stream.bound_id(), sink.bound_id())),
                Err(RouteError::Failed(_))
            ));

            let doomed = node(&context, &core, "removed-sink", false);
            let doomed_id = doomed.bound_id();
            wait(&context, || service.state().node(doomed_id).is_some());
            let pending = service.route(stream.bound_id(), doomed_id);
            drop(doomed);
            wait(&context, || service.state().node(doomed_id).is_none());
            assert_eq!(context.block_on(pending), Err(RouteError::EndpointRemoved));
            service.stop();
            drop(listener);
            std::fs::remove_file(&socket).unwrap();
            let absent =
                AudioService::with_remotes(&daemon.remote(), &format!("unix:{}", socket.display()));
            wait(&context, || {
                absent.state().node(stream.bound_id()).is_some()
                    && absent.state().node(sink.bound_id()).is_some()
            });
            assert!(matches!(
                context.block_on(absent.route(stream.bound_id(), sink.bound_id())),
                Err(RouteError::Failed(_))
            ));
            absent.stop();
            drop((stream, sink));
            core.disconnect();
        })
        .unwrap();
}
