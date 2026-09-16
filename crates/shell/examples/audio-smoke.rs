//! Exercises WirePlumber connection, plugin activation, inventory and cleanup.
use glib::prelude::*;
use std::{error::Error, time::Duration};
use wireplumber::{
    Core, InitFlags,
    core::ObjectFeatures,
    plugin::{Plugin, PluginFeatures},
    prelude::*,
    pw::{Link, Node, Port},
    registry::{Interest, ObjectManager},
};

struct Connection(Core);

impl Drop for Connection {
    fn drop(&mut self) {
        self.0.disconnect();
    }
}

async fn exercise(context: &glib::MainContext) -> Result<glib::WeakRef<Core>, Box<dyn Error>> {
    let connection = Connection(Core::new(Some(context), None, None));
    let core = &connection.0;
    let weak = core.downgrade();
    eprintln!("connecting to PipeWire");
    core.connect_future().await?;
    let mut plugins = Vec::new();
    for name in ["default-nodes-api", "mixer-api"] {
        eprintln!("loading {name}");
        core.load_component_future(
            Some(format!("libwireplumber-module-{name}").into()),
            "module",
            None,
            None,
        )
        .await?;
        plugins.push(Plugin::find(core, name).ok_or("loaded plugin was not registered")?);
    }
    for plugin in &plugins {
        eprintln!("activating plugin");
        plugin.activate_future(PluginFeatures::ENABLED).await?;
    }
    let manager = ObjectManager::new();
    manager.add_interest(Interest::<Node>::new());
    manager.add_interest(Interest::<Port>::new());
    manager.add_interest(Interest::<Link>::new());
    manager.request_object_features(Node::static_type(), ObjectFeatures::ALL);
    manager.request_object_features(Port::static_type(), ObjectFeatures::ALL);
    manager.request_object_features(Link::static_type(), ObjectFeatures::ALL);
    core.install_object_manager(&manager);
    eprintln!("waiting for object inventory");
    while !manager.is_installed() {
        glib::timeout_future(Duration::from_millis(10)).await;
    }
    if manager.n_objects() == 0 {
        return Err("WirePlumber did not inventory the PipeWire nodes/ports/links".into());
    }
    let default_sink = plugins[0].emit_by_name::<u32>("get-default-node", &[&"Audio/Sink"]);
    let default_source = plugins[0].emit_by_name::<u32>("get-default-node", &[&"Audio/Source"]);
    let volume = plugins[1].emit_by_name::<Option<glib::Variant>>("get-volume", &[&default_sink]);
    println!(
        "connected; plugins active; objects={}; sink={default_sink}; source={default_source}; volume={volume:?}",
        manager.n_objects()
    );
    Ok(weak)
}

fn main() -> Result<(), Box<dyn Error>> {
    Core::init_with_flags(InitFlags::PIPEWIRE);
    let context = glib::MainContext::default();
    context.with_thread_default(|| -> Result<(), Box<dyn Error>> {
        let mut previous_descriptors = None;
        for _ in 0..3 {
            let weak = context.block_on(glib::future_with_timeout(
                Duration::from_secs(5),
                exercise(&context),
            ))??;
            for _ in 0..100 {
                if !context.pending() {
                    break;
                }
                context.iteration(false);
            }
            if weak.upgrade().is_some() {
                return Err("WirePlumber core retained after disconnect and cleanup".into());
            }
            let descriptors = std::fs::read_dir("/proc/self/fd")?.count();
            if previous_descriptors.is_some_and(|previous| previous != descriptors) {
                return Err("descriptor count changed after reconnect and cleanup".into());
            }
            previous_descriptors = Some(descriptors);
        }
        println!("three connection/load/activation/inventory/cleanup cycles passed");
        Ok(())
    })?
}
