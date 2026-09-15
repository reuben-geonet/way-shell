use super::*;
use std::collections::HashMap;
use wireplumber::{
    core::ObjectFeatures,
    plugin::{Plugin, PluginFeatures},
    prelude::*,
    pw::{Link, Node, PipewireObject, Port, Properties},
    registry::{Interest, ObjectManager},
};

pub(super) struct Mixer(Plugin);
impl Mixer {
    pub fn set_volume(&self, id: u32, volume: f64) -> Result<(), AudioError> {
        let linear = way_shell_core::audio::to_linear(volume, way_shell_core::audio::Scale::Cubic);
        self.set(id, "volume", f64::from(linear).to_variant())
    }

    pub fn set_muted(&self, id: u32, muted: bool) -> Result<(), AudioError> {
        self.set(id, "mute", muted.to_variant())
    }

    fn set(&self, id: u32, name: &str, value: glib::Variant) -> Result<(), AudioError> {
        let values = HashMap::from([(name, value)]).to_variant();
        if self.0.emit_by_name::<bool>("set-volume", &[&id, &values]) {
            Ok(())
        } else {
            Err(AudioError::Rejected(id))
        }
    }
}

struct Handler(glib::Object, Option<glib::SignalHandlerId>);
impl Handler {
    fn new(object: &impl IsA<glib::Object>, handler: glib::SignalHandlerId) -> Self {
        Self(object.clone().upcast(), Some(handler))
    }
}
impl Drop for Handler {
    fn drop(&mut self) {
        if let Some(handler) = self.1.take() {
            self.0.disconnect(handler);
        }
    }
}
pub(super) struct Session {
    core: Core,
    manager: ObjectManager,
    mixer: Option<Plugin>,
    defaults: Option<Plugin>,
    handlers: Vec<Handler>,
    objects: Vec<Handler>,
}
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WirePlumber session")
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.objects.clear();
        self.handlers.clear();
        self.core.disconnect();
    }
}
impl Session {
    pub fn mixer(&self) -> Option<Mixer> {
        self.mixer.clone().map(Mixer)
    }

    pub async fn connect(remote: Option<&str>) -> Result<Self, glib::Error> {
        let properties = Properties::new();
        if let Some(remote) = remote {
            properties.insert("remote.name", remote);
        }
        let mut session = Self {
            core: Core::new(
                Some(&glib::MainContext::ref_thread_default()),
                None,
                Some(properties),
            ),
            manager: ObjectManager::new(),
            mixer: None,
            defaults: None,
            handlers: Vec::new(),
            objects: Vec::new(),
        };
        session.core.connect_future().await?;
        for name in ["default-nodes-api", "mixer-api"] {
            session
                .core
                .load_component_future(
                    Some(format!("libwireplumber-module-{name}").into()),
                    "module",
                    None,
                    None,
                )
                .await?;
            let plugin = Plugin::find(&session.core, name).ok_or_else(|| {
                glib::Error::new(
                    gio::IOErrorEnum::Failed,
                    "WirePlumber plugin was not registered",
                )
            })?;
            // The C contract uses linear mixer values and converts to the displayed cubic scale.
            if name == "mixer-api" {
                let linear = plugin
                    .find_property("scale")
                    .and_then(|property| glib::EnumClass::with_type(property.value_type()))
                    .and_then(|class| class.to_value(0))
                    .ok_or_else(|| {
                        glib::Error::new(
                            gio::IOErrorEnum::NotSupported,
                            "WirePlumber mixer has no linear scale",
                        )
                    })?;
                plugin.set_property_from_value("scale", &linear);
            }
            plugin.activate_future(PluginFeatures::ENABLED).await?;
            if name == "mixer-api" {
                session.mixer = Some(plugin);
            } else {
                session.defaults = Some(plugin);
            }
        }
        session.manager.add_interest(Interest::<Node>::new());
        session.manager.add_interest(Interest::<Port>::new());
        session.manager.add_interest(Interest::<Link>::new());
        for type_ in [
            Node::static_type(),
            Port::static_type(),
            Link::static_type(),
        ] {
            session
                .manager
                .request_object_features(type_, ObjectFeatures::ALL);
        }
        session.core.install_object_manager(&session.manager);
        session.manager.installed_future().await?;
        Ok(session)
    }
    pub fn subscribe(&mut self, service: &AudioService, generation: u64) {
        let weak = service.downgrade();
        let handler = self.core.connect_disconnected(move |_| {
            // Tear down after this native signal has returned, on the owning context.
            let weak = weak.clone();
            glib::MainContext::ref_thread_default().spawn_local(async move {
                if let Some(service) = weak.upgrade() {
                    service.disconnected(generation);
                }
            });
        });
        self.handlers.push(Handler::new(&self.core, handler));
        let weak = service.downgrade();
        self.handlers.push(Handler::new(
            &self.manager,
            self.manager.connect_objects_changed(move |_| {
                if let Some(service) = weak.upgrade() {
                    service.refresh();
                }
            }),
        ));
        for plugin in [&self.mixer, &self.defaults].into_iter().flatten() {
            let weak = service.downgrade();
            self.handlers.push(Handler::new(
                plugin,
                plugin.connect_local("changed", false, move |_| {
                    if let Some(service) = weak.upgrade() {
                        service.refresh();
                    }
                    None
                }),
            ));
        }
    }
    pub fn snapshot(&mut self, service: &AudioService) -> AudioState {
        let mut state = AudioState {
            available: true,
            ..AudioState::default()
        };
        self.objects.clear();
        for object in &self.manager {
            if let Some(node) = object.downcast_ref::<Node>() {
                if let Some(kind) = node
                    .get_pw_property("media.class")
                    .as_deref()
                    .and_then(NodeKind::from_class)
                {
                    let name = node.name().unwrap_or_default();
                    let volume = self
                        .mixer
                        .as_ref()
                        .and_then(|m| {
                            m.emit_by_name::<Option<glib::Variant>>(
                                "get-volume",
                                &[&node.bound_id()],
                            )
                        })
                        .as_ref()
                        .and_then(read_volume);
                    state.nodes.push(AudioNode {
                        id: node.bound_id(),
                        serial: node
                            .get_pw_property("object.serial")
                            .and_then(|serial| serial.parse().ok())
                            .unwrap_or_default(),
                        kind,
                        description: node
                            .get_pw_property("node.description")
                            .unwrap_or_else(|| name.clone()),
                        name,
                        nickname: node.get_pw_property("node.nick").unwrap_or_default(),
                        application: node.get_pw_property("application.name").unwrap_or_default(),
                        media: node.get_pw_property("media.name").unwrap_or_default(),
                        state: match node.state() {
                            wireplumber::pw::NodeState::Error => NodeState::Error,
                            wireplumber::pw::NodeState::Creating => NodeState::Creating,
                            wireplumber::pw::NodeState::Suspended => NodeState::Suspended,
                            wireplumber::pw::NodeState::Idle => NodeState::Idle,
                            wireplumber::pw::NodeState::Running => NodeState::Running,
                            _ => NodeState::Unknown,
                        },
                        volume,
                    });
                    let weak = service.downgrade();
                    self.objects.push(Handler::new(
                        node,
                        node.connect_state_changed(move |_, _, _| {
                            if let Some(service) = weak.upgrade() {
                                service.refresh();
                            }
                        }),
                    ));
                }
            } else if let Some(port) = object.downcast_ref::<Port>() {
                if let (Ok(node), Ok(index)) = (port.node_id(), port.port_index()) {
                    let direction = match port.direction() {
                        wireplumber::pw::Direction::Input => Direction::Input,
                        wireplumber::pw::Direction::Output => Direction::Output,
                        _ => continue,
                    };
                    state.ports.push(AudioPort {
                        id: port.bound_id(),
                        node,
                        index,
                        direction,
                        channel: port.get_pw_property("audio.channel"),
                        monitor: port
                            .get_pw_property("port.monitor")
                            .is_some_and(|value| value == "true"),
                    });
                }
            } else if let Some(link) = object.downcast_ref::<Link>() {
                let (output_node, output_port, input_node, input_port) = link.linked_object_ids();
                state.links.push(AudioLink {
                    id: link.bound_id(),
                    output_node,
                    output_port,
                    input_node,
                    input_port,
                });
            }
            if let Some(pw) = object.dynamic_cast_ref::<PipewireObject>() {
                let weak = service.downgrade();
                self.objects.push(Handler::new(
                    pw,
                    pw.connect_properties_notify(move |_| {
                        if let Some(service) = weak.upgrade() {
                            service.refresh();
                        }
                    }),
                ));
            }
        }
        state.nodes.sort_by_key(|n| n.id);
        state.ports.sort_by_key(|p| p.id);
        state.links.sort_by_key(|l| l.id);
        if let Some(defaults) = &self.defaults {
            for (class, kind, target) in [
                ("Audio/Sink", NodeKind::Sink, &mut state.default_sink),
                ("Audio/Source", NodeKind::Source, &mut state.default_source),
            ] {
                let id = defaults.emit_by_name::<u32>("get-default-node", &[&class]);
                *target = state
                    .nodes
                    .iter()
                    .find(|n| n.id == id && n.kind == kind)
                    .map(|n| n.id);
            }
        }
        state
    }
}
fn nonnegative(value: f64) -> Option<f64> {
    (value.is_finite() && value >= 0.0).then_some(value)
}
fn read_volume(value: &glib::Variant) -> Option<Volume> {
    let dictionary = value.get::<HashMap<String, glib::Variant>>()?;
    let number = |name: &str| dictionary.get(name)?.get::<f64>().and_then(nonnegative);
    let volume = number("volume")?.cbrt();
    let mute = dictionary.get("mute")?.get::<bool>()?;
    let mut channels = Vec::new();
    if let Some(values) = dictionary
        .get("channelVolumes")
        .and_then(|v| v.get::<HashMap<String, glib::Variant>>())
    {
        for (index, entry) in values {
            let Some(entry) = entry.get::<HashMap<String, glib::Variant>>() else {
                continue;
            };
            let (Ok(index), Some(volume)) = (
                index.parse(),
                entry
                    .get("volume")
                    .and_then(|v| v.get::<f64>())
                    .and_then(nonnegative),
            ) else {
                continue;
            };
            channels.push(ChannelVolume {
                index,
                channel: entry.get("channel").and_then(|v| v.get()),
                volume: volume.cbrt(),
            });
        }
    }
    channels.sort_by_key(|c| c.index);
    Some(Volume {
        volume,
        mute,
        step: number("step").unwrap_or(0.0),
        base: number("base").unwrap_or(1.0),
        channels,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixer_dictionary_types_and_invalid_values() {
        let mut values = HashMap::from([
            ("volume", 0.125_f64.to_variant()),
            ("mute", true.to_variant()),
            ("step", 0.01_f64.to_variant()),
            ("base", 0.75_f64.to_variant()),
        ]);
        let volume = read_volume(&values.to_variant()).unwrap();
        assert_eq!(volume.volume, 0.5);
        assert_eq!(volume.step, 0.01);
        assert!(volume.mute);
        assert_eq!(volume.base, 0.75);
        for invalid in [f64::NAN, f64::INFINITY, -1.0] {
            values.insert("volume", invalid.to_variant());
            assert!(read_volume(&values.to_variant()).is_none());
        }
        assert!(read_volume(&false.to_variant()).is_none());
    }
}
