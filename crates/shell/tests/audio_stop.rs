//! Stopping from a subscriber must not destroy PipeWire's current dispatch stack.
use glib::prelude::*;
use std::{cell::Cell, rc::Rc, time::Duration};
use way_shell::services::audio::AudioService;
use wireplumber::{Core, InitFlags, prelude::*, pw::Properties};

#[path = "common/audio.rs"]
mod fixture;

#[test]
fn stop_from_native_mixer_notification_clears_state_and_releases_owner() {
    Core::init_with_flags(InitFlags::PIPEWIRE);
    let daemon = fixture::Daemon::new();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let service = AudioService::with_remote(&daemon.remote());
            let properties = Properties::new();
            properties.insert("remote.name", daemon.remote());
            let core = Core::new(Some(&context), None, Some(properties));
            context.block_on(core.connect_future()).unwrap();
            let source = fixture::node(&context, &core, "stop-source", "Audio/Source");
            let sink = fixture::node(&context, &core, "stop-sink", "Audio/Sink");
            let link = fixture::connect_nodes(&context, &core, &source, &sink);
            let id = sink.bound_id();
            fixture::wait(&context, || {
                service
                    .state()
                    .node(id)
                    .is_some_and(|node| node.volume.is_some())
            });
            let stopped = Rc::new(Cell::new(false));
            let captured = stopped.clone();
            let weak = service.downgrade();
            service.connect_local("changed", false, move |_| {
                if let Some(service) = weak.upgrade()
                    && service
                        .state()
                        .node(id)
                        .and_then(|node| node.volume.as_ref())
                        .is_some_and(|volume| (volume.volume - 0.41).abs() < 0.0001)
                    && !captured.replace(true)
                {
                    service.stop();
                }
                None
            });
            service.set_volume(id, 0.41).unwrap();
            fixture::wait(&context, || stopped.get());
            assert!(!service.state().available);
            assert!(service.state().nodes.is_empty());
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
            context.block_on(glib::timeout_future(Duration::from_millis(30)));

            let replacement = AudioService::with_remote(&daemon.remote());
            fixture::wait(&context, || replacement.state().node(id).is_some());
            replacement.stop();
            drop(replacement);
            context.block_on(glib::timeout_future(Duration::from_millis(30)));
            drop((link, sink, source));
            core.disconnect();
        })
        .unwrap();
}
