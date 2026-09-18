//! Quick-settings device levels and stream routing on the owned audio service.

use super::menu::Menu;
use crate::services::audio::{AudioNode, AudioService, AudioState, NodeKind, NodeState};
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NodeKey {
    id: u32,
    serial: u64,
}
impl NodeKey {
    fn of(node: &AudioNode) -> Self {
        Self {
            id: node.id,
            serial: node.serial,
        }
    }
    fn find(self, state: &AudioState) -> Option<&AudioNode> {
        state
            .node(self.id)
            .filter(|node| node.serial == self.serial)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RouteTarget {
    key: NodeKey,
    label: String,
}
fn device_name(node: &AudioNode) -> &str {
    if !node.nickname.is_empty() {
        &node.nickname
    } else if !node.description.is_empty() {
        &node.description
    } else {
        &node.name
    }
}
fn node_label(node: &AudioNode, default: bool) -> String {
    match node.kind {
        NodeKind::Sink | NodeKind::Source => {
            format!("{}{}", if default { "*" } else { "" }, device_name(node))
        }
        NodeKind::InputStream => format!("{} (Input)", node.media),
        NodeKind::OutputStream => format!("{} (Output)", node.media),
    }
}
fn default_node(state: &AudioState, kind: NodeKind) -> Option<&AudioNode> {
    if !state.available {
        return None;
    }
    let id = match kind {
        NodeKind::Sink => state.default_sink,
        NodeKind::Source => state.default_source,
        _ => None,
    }?;
    state.node(id).filter(|node| node.kind == kind)
}
fn target_kind(kind: NodeKind) -> Option<NodeKind> {
    match kind {
        NodeKind::OutputStream => Some(NodeKind::Sink),
        NodeKind::InputStream => Some(NodeKind::Source),
        _ => None,
    }
}
fn route_targets(state: &AudioState, stream: &AudioNode) -> Vec<RouteTarget> {
    state
        .nodes
        .iter()
        .filter(|node| state.available && Some(node.kind) == target_kind(stream.kind))
        .map(|node| RouteTarget {
            key: NodeKey::of(node),
            label: device_name(node).into(),
        })
        .collect()
}
fn linked_target(state: &AudioState, stream: &AudioNode) -> Option<NodeKey> {
    state.links.iter().rev().find_map(|link| {
        let id = match stream.kind {
            NodeKind::OutputStream if link.output_node == stream.id => link.input_node,
            NodeKind::InputStream if link.input_node == stream.id => link.output_node,
            _ => return None,
        };
        state
            .node(id)
            .filter(|node| Some(node.kind) == target_kind(stream.kind))
            .map(NodeKey::of)
    })
}

struct VolumeView {
    value: f64,
    muted: bool,
    sensitive: bool,
    icon: String,
}
impl VolumeView {
    fn of(node: Option<&AudioNode>, kind: NodeKind) -> Self {
        let volume = node
            .and_then(|node| node.volume.as_ref())
            .filter(|volume| volume.volume.is_finite() && volume.volume >= 0.0);
        let value = volume.map_or(0.0, |volume| volume.volume.min(1.0));
        let muted = volume.is_none_or(|volume| volume.mute);
        let level = if muted {
            "muted"
        } else if value < 0.25 {
            "low"
        } else if value < 0.5 {
            "medium"
        } else {
            "high"
        };
        let prefix = if kind == NodeKind::Source {
            "microphone-sensitivity"
        } else {
            "audio-volume"
        };
        Self {
            value,
            muted,
            sensitive: volume.is_some(),
            icon: format!("{prefix}-{level}-symbolic"),
        }
    }
}

struct MainScale {
    kind: NodeKind,
    key: Cell<Option<NodeKey>>,
    root: gtk::Box,
    button: gtk::Button,
    icon: gtk::Image,
    scale: gtk::Scale,
}
impl MainScale {
    fn new(kind: NodeKind, name: &str) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.set_widget_name(name);
        let button = gtk::Button::new();
        let icon = gtk::Image::from_icon_name(&VolumeView::of(None, kind).icon);
        button.set_child(Some(&icon));
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.05);
        scale.set_hexpand(true);
        root.append(&button);
        root.append(&scale);
        Self {
            kind,
            key: Cell::new(None),
            root,
            button,
            icon,
            scale,
        }
    }
    fn update(&self, state: &AudioState) {
        let node = default_node(state, self.kind);
        let view = VolumeView::of(node, self.kind);
        self.key.set(node.map(NodeKey::of));
        self.root.set_sensitive(view.sensitive);
        if self.kind == NodeKind::Source {
            self.root
                .set_visible(node.is_some_and(|node| node.state == NodeState::Running));
        }
        self.icon.set_icon_name(Some(&view.icon));
        self.scale
            .set_value(if view.muted { 0.0 } else { view.value });
        self.root.set_tooltip_text(Some(
            node.map_or("Audio device is unavailable", device_name),
        ));
        self.scale.set_tooltip_text(None);
    }
}

/// The header owns menu visibility. This controller owns audio subscriptions,
/// per-device widgets and cancellable stream moves; no native service pointer escapes.
pub struct AudioControls {
    service: AudioService,
    button: gtk::Button,
    menu: Menu,
    empty: gtk::Label,
    scales: gtk::Revealer,
    sink: MainScale,
    source: MainScale,
    rows: RefCell<Vec<Rc<MixerRow>>>,
    icons: RefCell<HashMap<String, Option<gio::Icon>>>,
    handler: Cell<Option<glib::SignalHandlerId>>,
    updating: Cell<bool>,
    pending: Cell<bool>,
    closed: Cell<bool>,
}
impl AudioControls {
    pub fn new(service: AudioService) -> Rc<Self> {
        let menu = Menu::new("Mixer", "audio-speakers-symbolic", true);
        menu.widget().set_size_request(-1, 420);
        let empty = gtk::Label::new(Some("Audio service is unavailable"));
        menu.options().append(&empty);
        let button = gtk::Button::from_icon_name("audio-speakers-symbolic");
        button.add_css_class("circular");
        let scales = gtk::Revealer::new();
        scales.set_transition_type(gtk::RevealerTransitionType::SlideUp);
        scales.set_transition_duration(250);
        scales.set_reveal_child(true);
        let contents = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let sink = MainScale::new(NodeKind::Sink, "default-sink-container");
        let source = MainScale::new(NodeKind::Source, "default-source-container");
        contents.append(&sink.root);
        contents.append(&source.root);
        scales.set_child(Some(&contents));
        let this = Rc::new(Self {
            service,
            button,
            menu,
            empty,
            scales,
            sink,
            source,
            rows: RefCell::new(Vec::new()),
            icons: RefCell::new(HashMap::new()),
            handler: Cell::new(None),
            updating: Cell::new(false),
            pending: Cell::new(false),
            closed: Cell::new(false),
        });
        for main in [&this.sink, &this.source] {
            let weak = Rc::downgrade(&this);
            let kind = main.kind;
            main.scale.connect_value_changed(move |scale| {
                if let Some(this) = weak.upgrade() {
                    this.main_volume(kind, Some(scale.value()));
                }
            });
            let weak = Rc::downgrade(&this);
            main.button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.main_volume(kind, None);
                }
            });
        }
        let weak = Rc::downgrade(&this);
        this.handler.set(Some(this.service.connect_local(
            "changed",
            false,
            move |_| {
                if let Some(this) = weak.upgrade() {
                    this.refresh();
                }
                None
            },
        )));
        this.refresh();
        this
    }
    pub fn mixer_button(&self) -> &gtk::Button {
        &self.button
    }
    pub fn menu(&self) -> &Menu {
        &self.menu
    }
    pub fn scales(&self) -> &gtk::Revealer {
        &self.scales
    }
    pub fn set_mixer_revealed(&self, revealed: bool) {
        if !self.closed.get() {
            self.scales.set_reveal_child(!revealed);
        }
    }
    pub fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        if let Some(handler) = self.handler.take() {
            self.service.disconnect(handler);
        }
        let rows = self.rows.take();
        for row in rows {
            row.close();
        }
        self.sink.root.set_sensitive(false);
        self.source.root.set_sensitive(false);
        self.button.set_sensitive(false);
        self.menu.widget().set_sensitive(false);
        self.scales.set_reveal_child(false);
    }
    fn main(&self, kind: NodeKind) -> &MainScale {
        if kind == NodeKind::Source {
            &self.source
        } else {
            &self.sink
        }
    }
    fn main_volume(self: &Rc<Self>, kind: NodeKind, value: Option<f64>) {
        if self.closed.get() || self.updating.get() {
            return;
        }
        let main = self.main(kind);
        let state = self.service.state();
        let Some(node) =
            default_node(&state, kind).filter(|node| Some(NodeKey::of(node)) == main.key.get())
        else {
            return;
        };
        let result = if let Some(value) = value {
            self.service.set_volume(node.id, value)
        } else {
            self.service.set_muted(
                node.id,
                node.volume.as_ref().is_none_or(|volume| !volume.mute),
            )
        };
        if let Err(error) = result {
            self.refresh();
            main.scale.set_tooltip_text(Some(&error.to_string()));
        }
    }
    fn refresh(self: &Rc<Self>) {
        self.pending.set(true);
        if self.updating.replace(true) {
            return;
        }
        while self.pending.replace(false) && !self.closed.get() {
            let state = self.service.state();
            self.button.set_sensitive(state.available);
            if self.closed.get() {
                break;
            }
            self.sink.update(&state);
            if self.closed.get() {
                break;
            }
            self.source.update(&state);
            if self.closed.get() {
                break;
            }
            self.update_rows(&state);
        }
        self.updating.set(false);
    }
    fn update_rows(self: &Rc<Self>, state: &AudioState) {
        let old = self.rows.borrow().clone();
        let mut nodes: Vec<_> = state.nodes.iter().filter(|_| state.available).collect();
        // Preserve the existing stream-first, then outputs, then inputs layout.
        nodes.sort_by_key(|node| match node.kind {
            NodeKind::InputStream | NodeKind::OutputStream => (0, u32::MAX - node.id),
            NodeKind::Sink => (1, node.id),
            NodeKind::Source => (2, node.id),
        });
        let rows: Vec<_> = nodes
            .iter()
            .map(|node| {
                old.iter()
                    .find(|row| row.key == NodeKey::of(node))
                    .cloned()
                    .unwrap_or_else(|| MixerRow::new(self, node))
            })
            .collect();
        self.rows.replace(rows.clone());
        for row in old {
            if !rows.iter().any(|current| Rc::ptr_eq(current, &row)) {
                row.close();
                self.menu.options().remove(&row.root);
            }
        }
        let mut previous: Option<gtk::Widget> = None;
        for (row, node) in rows.iter().zip(nodes) {
            if self.closed.get() {
                break;
            }
            if row.root.parent().is_none() {
                self.menu.options().append(&row.root);
            }
            self.menu
                .options()
                .reorder_child_after(&row.root, previous.as_ref());
            previous = Some(row.root.clone().upcast());
            row.update(self, state, node);
        }
        self.empty.set_label(if state.available {
            "No audio devices or streams"
        } else {
            "Audio service is unavailable"
        });
        self.empty.set_visible(rows.is_empty());
    }
    fn application_icon(&self, application: &str) -> Option<gio::Icon> {
        if application.is_empty() {
            return None;
        }
        if let Some(icon) = self.icons.borrow().get(application) {
            return icon.clone();
        }
        let query = application.to_lowercase();
        let icon = gio::AppInfo::all()
            .into_iter()
            .find(|info| {
                info.id()
                    .is_some_and(|id| id.to_lowercase().contains(&query))
            })
            .and_then(|info| info.icon());
        self.icons
            .borrow_mut()
            .insert(application.into(), icon.clone());
        icon
    }
}
impl Drop for AudioControls {
    fn drop(&mut self) {
        self.close();
    }
}

struct MixerRow {
    key: NodeKey,
    root: gtk::Box,
    button: gtk::Button,
    icon: gtk::Image,
    active: gtk::Image,
    label: gtk::Label,
    revealer: gtk::Revealer,
    volume: gtk::Scale,
    dropdown: gtk::DropDown,
    targets: RefCell<Vec<RouteTarget>>,
    changing: Cell<bool>,
    closed: Cell<bool>,
    route_generation: Cell<u64>,
    route_target: Cell<Option<NodeKey>>,
    route_task: RefCell<Option<glib::JoinHandle<()>>>,
}
impl MixerRow {
    fn new(owner: &Rc<AudioControls>, node: &AudioNode) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("quick-settings-menu-option-mixer");
        let button = gtk::Button::new();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let active = gtk::Image::from_icon_name("media-record-symbolic");
        let icon = gtk::Image::from_icon_name("audio-speakers-symbolic");
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_max_width_chars(120);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        content.append(&active);
        content.append(&icon);
        content.append(&label);
        button.set_child(Some(&content));
        root.append(&button);
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SwingDown);
        revealer.set_transition_duration(350);
        let details = gtk::Box::new(gtk::Orientation::Vertical, 0);
        details.set_focusable(true);
        let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.05);
        volume.set_hexpand(true);
        let dropdown = gtk::DropDown::from_strings(&[]);
        if target_kind(node.kind).is_some() {
            let route = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            route.add_css_class("mixer-link");
            route.append(&gtk::Image::from_icon_name(
                "network-wireless-hotspot-symbolic",
            ));
            route.append(&dropdown);
            details.append(&route);
        } else {
            details.append(&volume);
        }
        revealer.set_child(Some(&details));
        root.append(&revealer);
        let this = Rc::new(Self {
            key: NodeKey::of(node),
            root,
            button,
            icon,
            active,
            label,
            revealer,
            volume,
            dropdown,
            targets: RefCell::new(Vec::new()),
            changing: Cell::new(false),
            closed: Cell::new(false),
            route_generation: Cell::new(0),
            route_target: Cell::new(None),
            route_task: RefCell::new(None),
        });
        let weak = Rc::downgrade(&this);
        this.button.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade().filter(|row| !row.closed.get()) {
                let reveal = !this.revealer.reveals_child();
                this.revealer.set_reveal_child(reveal);
                if reveal && let Some(child) = this.revealer.child() {
                    child.grab_focus();
                }
            }
        });
        let weak = Rc::downgrade(&this);
        let owner_weak = Rc::downgrade(owner);
        this.volume.connect_value_changed(move |scale| {
            if let (Some(row), Some(owner)) = (weak.upgrade(), owner_weak.upgrade())
                && !row.closed.get()
                && !row.changing.get()
                && !owner.closed.get()
                && row.key.find(&owner.service.state()).is_some()
                && let Err(error) = owner.service.set_volume(row.key.id, scale.value())
            {
                owner.refresh();
                row.volume.set_tooltip_text(Some(&error.to_string()));
            }
        });
        let weak = Rc::downgrade(&this);
        let owner_weak = Rc::downgrade(owner);
        this.dropdown.connect_selected_notify(move |_| {
            if let (Some(row), Some(owner)) = (weak.upgrade(), owner_weak.upgrade()) {
                row.route(&owner);
            }
        });
        this
    }
    fn update(&self, owner: &AudioControls, state: &AudioState, node: &AudioNode) {
        if self.closed.get() {
            return;
        }
        self.changing.set(true);
        let default = Some(node.id)
            == match node.kind {
                NodeKind::Sink => state.default_sink,
                NodeKind::Source => state.default_source,
                _ => None,
            };
        self.label.set_label(&node_label(node, default));
        if node.state == NodeState::Running {
            self.active.add_css_class("active-icon-activated");
        } else {
            self.active.remove_css_class("active-icon-activated");
        }
        if target_kind(node.kind).is_some() {
            self.button.set_tooltip_text(Some(&format!(
                "{}: {}",
                node.application,
                node_label(node, false)
            )));
            if let Some(icon) = owner.application_icon(&node.application) {
                let theme = gtk::IconTheme::for_display(&self.icon.display());
                let paintable = theme.lookup_by_gicon(
                    &icon,
                    64,
                    1,
                    gtk::TextDirection::Rtl,
                    gtk::IconLookupFlags::empty(),
                );
                self.icon.set_paintable(Some(&paintable));
            } else {
                self.icon
                    .set_icon_name(Some("applications-multimedia-symbolic"));
            }
            let targets = route_targets(state, node);
            if self
                .route_target
                .get()
                .is_some_and(|key| !targets.iter().any(|target| target.key == key))
            {
                self.cancel_route();
            }
            if *self.targets.borrow() != targets {
                let names: Vec<_> = targets.iter().map(|target| target.label.as_str()).collect();
                let model = gtk::StringList::new(&names);
                self.targets.replace(targets.clone());
                self.dropdown.set_model(Some(&model));
            }
            let selected = self
                .route_target
                .get()
                .or_else(|| linked_target(state, node));
            let index = targets
                .iter()
                .position(|target| Some(target.key) == selected)
                .and_then(|index| u32::try_from(index).ok())
                .unwrap_or(gtk::INVALID_LIST_POSITION);
            self.dropdown.set_selected(index);
            self.dropdown.set_sensitive(!targets.is_empty());
        } else {
            self.button.set_tooltip_text(Some(&node.description));
            let view = VolumeView::of(Some(node), node.kind);
            self.icon.set_icon_name(Some(&view.icon));
            self.volume.set_sensitive(view.sensitive);
            self.volume.set_value(view.value);
        }
        self.changing.set(false);
    }
    fn route(self: &Rc<Self>, owner: &Rc<AudioControls>) {
        if self.closed.get() || self.changing.get() || owner.closed.get() {
            return;
        }
        let target = self
            .targets
            .borrow()
            .get(self.dropdown.selected() as usize)
            .cloned();
        let Some(target) = target else {
            return;
        };
        let state = owner.service.state();
        if self.key.find(&state).is_none() || target.key.find(&state).is_none() {
            owner.refresh();
            return;
        }
        self.cancel_route();
        self.route_target.set(Some(target.key));
        self.dropdown.set_tooltip_text(None);
        let request = owner.service.route(self.key.id, target.key.id);
        let generation = self.route_generation.get();
        let weak = Rc::downgrade(self);
        let owner_weak = Rc::downgrade(owner);
        let task = glib::MainContext::ref_thread_default().spawn_local(async move {
            let result = request.await;
            if let (Some(row), Some(owner)) = (weak.upgrade(), owner_weak.upgrade())
                && !row.closed.get()
                && row.route_generation.get() == generation
            {
                row.route_task.borrow_mut().take();
                row.route_target.set(None);
                owner.refresh();
                if let Err(error) = result {
                    row.dropdown.set_tooltip_text(Some(&error.to_string()));
                }
            }
        });
        self.route_task.replace(Some(task));
    }
    fn cancel_route(&self) {
        self.route_generation
            .set(self.route_generation.get().wrapping_add(1));
        self.route_target.set(None);
        let task = self.route_task.borrow_mut().take();
        if let Some(task) = task {
            task.abort();
        }
    }
    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.cancel_route();
        self.root.set_sensitive(false);
    }
}
impl Drop for MixerRow {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::audio::{AudioLink, Volume};

    fn node(id: u32, kind: NodeKind) -> AudioNode {
        AudioNode {
            id,
            serial: u64::from(id) + 100,
            kind,
            name: format!("node-{id}"),
            description: format!("Device {id}"),
            nickname: String::new(),
            application: "Player".into(),
            media: "Track".into(),
            state: NodeState::Idle,
            volume: Some(Volume {
                volume: 0.6,
                mute: false,
                step: 0.05,
                base: 1.0,
                channels: vec![],
            }),
        }
    }

    #[test]
    fn labels_preserve_default_marker_nickname_and_stream_direction() {
        let mut sink = node(1, NodeKind::Sink);
        assert_eq!(node_label(&sink, false), "Device 1");
        sink.nickname = "Speakers".into();
        assert_eq!(node_label(&sink, true), "*Speakers");
        assert_eq!(
            node_label(&node(2, NodeKind::InputStream), false),
            "Track (Input)"
        );
        assert_eq!(
            node_label(&node(3, NodeKind::OutputStream), false),
            "Track (Output)"
        );
    }

    #[test]
    fn defaults_require_matching_devices_and_available_service() {
        let sink = node(1, NodeKind::Sink);
        let source = node(2, NodeKind::Source);
        let mut state = AudioState {
            available: true,
            nodes: vec![sink, source],
            default_sink: Some(1),
            default_source: Some(2),
            ..AudioState::default()
        };
        assert_eq!(default_node(&state, NodeKind::Sink).unwrap().id, 1);
        assert_eq!(default_node(&state, NodeKind::Source).unwrap().id, 2);
        state.default_source = Some(1);
        assert!(default_node(&state, NodeKind::Source).is_none());
        state.available = false;
        assert!(default_node(&state, NodeKind::Sink).is_none());
    }

    #[test]
    fn routing_uses_serial_identity_and_matching_direction_with_duplicate_names() {
        let mut first = node(1, NodeKind::Sink);
        let mut second = node(2, NodeKind::Sink);
        first.nickname = "Same".into();
        second.nickname = "Same".into();
        let source = node(3, NodeKind::Source);
        let playback = node(4, NodeKind::OutputStream);
        let capture = node(5, NodeKind::InputStream);
        let mut state = AudioState {
            available: true,
            nodes: vec![
                first.clone(),
                second.clone(),
                source.clone(),
                playback.clone(),
                capture.clone(),
            ],
            links: vec![
                AudioLink {
                    id: 30,
                    output_node: 4,
                    output_port: 40,
                    input_node: 2,
                    input_port: 20,
                },
                AudioLink {
                    id: 31,
                    output_node: 3,
                    output_port: 30,
                    input_node: 5,
                    input_port: 50,
                },
            ],
            ..AudioState::default()
        };
        let choices = route_targets(&state, &playback);
        assert_eq!(
            choices.iter().map(|target| target.key).collect::<Vec<_>>(),
            vec![NodeKey::of(&first), NodeKey::of(&second)]
        );
        assert_eq!(linked_target(&state, &playback), Some(NodeKey::of(&second)));
        assert_eq!(route_targets(&state, &capture).len(), 1);
        assert_eq!(linked_target(&state, &capture), Some(NodeKey::of(&source)));
        let old = NodeKey::of(&second);
        state.nodes[1].serial += 1;
        assert!(
            old.find(&state).is_none(),
            "a reused numeric id must not target a replacement device"
        );
        state.nodes.retain(|node| node.id != source.id);
        assert!(linked_target(&state, &capture).is_none());
        assert!(route_targets(&state, &capture).is_empty());
    }

    #[test]
    fn volume_mapping_preserves_levels_and_disables_invalid_values() {
        let mut sink = node(1, NodeKind::Sink);
        for (level, suffix) in [
            (0.0, "low"),
            (0.249, "low"),
            (0.25, "medium"),
            (0.499, "medium"),
            (0.5, "high"),
            (1.3, "high"),
        ] {
            sink.volume.as_mut().unwrap().volume = level;
            let view = VolumeView::of(Some(&sink), NodeKind::Sink);
            assert!(view.sensitive);
            assert_eq!(view.icon, format!("audio-volume-{suffix}-symbolic"));
            assert_eq!(view.value, level.min(1.0));
        }
        sink.volume.as_mut().unwrap().mute = true;
        let view = VolumeView::of(Some(&sink), NodeKind::Sink);
        assert_eq!(view.icon, "audio-volume-muted-symbolic");
        assert_eq!(
            view.value, 1.0,
            "mixer volume stays at the observed level while muted"
        );
        assert!(view.muted);
        sink.volume.as_mut().unwrap().volume = f64::NAN;
        assert!(!VolumeView::of(Some(&sink), NodeKind::Sink).sensitive);
        sink.volume = None;
        assert!(!VolumeView::of(Some(&sink), NodeKind::Sink).sensitive);
        assert_eq!(
            VolumeView::of(None, NodeKind::Source).icon,
            "microphone-sensitivity-muted-symbolic"
        );
    }
}
