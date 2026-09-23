//! Panel status icons backed by owned service state.

use crate::services::{
    audio::{AudioService, AudioState},
    logind::LogindService,
    network::{NetworkService, NetworkState},
    power::{PowerDevice, PowerService},
    wayland::WaylandService,
};
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone)]
pub struct StatusServices {
    pub audio: AudioService,
    pub network: NetworkService,
    pub power: PowerService,
    pub logind: LogindService,
    pub wayland: WaylandService,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusAction {
    ToggleQuickSettings,
}

/// The panel owns this controller; retaining only its widget does not retain subscriptions.
#[derive(Clone)]
pub struct StatusBar(Rc<Inner>);

struct Subscription {
    object: glib::Object,
    handler: Option<glib::SignalHandlerId>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(handler) = self.handler.take() {
            self.object.disconnect(handler);
        }
    }
}

struct Inner {
    subscriptions: RefCell<Vec<Subscription>>,
    services: StatusServices,
    container: gtk::Box,
    button: gtk::Button,
    idle: gtk::Image,
    nightlight: gtk::Image,
    airplane: gtk::Image,
    vpn: gtk::Image,
    network: gtk::Image,
    microphone: gtk::Image,
    speaker: gtk::Image,
    power: gtk::Image,
    toggled: Cell<bool>,
    updating: Cell<bool>,
    pending: Cell<bool>,
    action: Box<dyn Fn(StatusAction)>,
}

impl StatusBar {
    pub fn new(services: StatusServices, action: impl Fn(StatusAction) + 'static) -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let button = gtk::Button::new();
        button.add_css_class("panel-button");
        let icons = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let idle = gtk::Image::from_icon_name("radio-mixed-symbolic");
        let nightlight = gtk::Image::from_icon_name("night-light-symbolic");
        let airplane = gtk::Image::from_icon_name("airplane-mode-symbolic");
        let vpn = gtk::Image::from_icon_name("network-vpn-symbolic");
        let network = gtk::Image::from_icon_name("network-wired-symbolic");
        let microphone = gtk::Image::from_icon_name("microphone-sensitivity-high-symbolic");
        let speaker = gtk::Image::from_icon_name("audio-volume-muted-symbolic");
        let power = gtk::Image::from_icon_name("battery-missing-symbolic");
        for icon in [&idle, &nightlight, &airplane, &vpn, &network] {
            icons.append(icon);
        }
        // Preserve the sound container as well as the icon order for existing CSS.
        let sound = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        sound.append(&microphone);
        sound.append(&speaker);
        icons.append(&sound);
        icons.append(&power);
        button.set_child(Some(&icons));
        container.append(&button);
        let inner = Rc::new(Inner {
            subscriptions: RefCell::new(Vec::new()),
            services,
            container,
            button,
            idle,
            nightlight,
            airplane,
            vpn,
            network,
            microphone,
            speaker,
            power,
            toggled: Cell::new(false),
            updating: Cell::new(false),
            pending: Cell::new(false),
            action: Box::new(action),
        });
        inner.watch(&inner.services.audio, "changed");
        inner.watch(&inner.services.network, "changed");
        inner.watch(&inner.services.power, "changed");
        inner.watch(&inner.services.logind, "changed");
        for signal in [
            "ready",
            "capabilities-changed",
            "gamma-control-enabled",
            "gamma-control-disabled",
        ] {
            inner.watch(&inner.services.wayland, signal);
        }
        let weak = Rc::downgrade(&inner);
        let handler = inner.button.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                (inner.action)(StatusAction::ToggleQuickSettings);
            }
        });
        inner.subscriptions.borrow_mut().push(Subscription {
            object: inner.button.clone().upcast(),
            handler: Some(handler),
        });
        inner.refresh();
        Self(inner)
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.0.container
    }

    pub fn set_toggled(&self, toggled: bool) {
        self.0.toggled.set(toggled);
        if toggled {
            self.0.button.add_css_class("panel-button-toggled");
        } else {
            self.0.button.remove_css_class("panel-button-toggled");
        }
    }

    pub fn is_toggled(&self) -> bool {
        self.0.toggled.get()
    }
}

impl Inner {
    fn watch(self: &Rc<Self>, object: &impl IsA<glib::Object>, signal: &str) {
        let weak = Rc::downgrade(self);
        let handler = object.connect_local(signal, false, move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.refresh();
            }
            None
        });
        self.subscriptions.borrow_mut().push(Subscription {
            object: object.as_ref().clone(),
            handler: Some(handler),
        });
    }

    fn refresh(&self) {
        self.pending.set(true);
        if self.updating.replace(true) {
            return;
        }
        // GTK property notifications may synchronously change or stop a
        // service. Finish each pass, then apply the newest owned state.
        while self.pending.replace(false) {
            self.refresh_audio();
            self.refresh_network();
            self.refresh_power();
            self.refresh_idle();
            self.refresh_nightlight();
        }
        self.updating.set(false);
    }

    fn refresh_network(&self) {
        let state = network_view(&self.services.network.state());
        self.airplane.set_visible(state.airplane);
        self.vpn.set_visible(state.vpn);
        self.network.set_visible(state.visible);
        set_icon(&self.network, state.icon);
    }

    fn refresh_audio(&self) {
        let state = audio_view(&self.services.audio.state());
        self.microphone.set_visible(state.microphone);
        set_icon(&self.speaker, state.icon);
        self.speaker.set_sensitive(state.sensitive);
        self.speaker.set_tooltip_text(state.tooltip);
    }

    fn refresh_power(&self) {
        let state = power_view(self.services.power.primary_device().as_ref());
        set_icon(&self.power, state.icon);
        self.power.set_tooltip_text(state.tooltip);
    }

    fn refresh_idle(&self) {
        let state = self.services.logind.state();
        self.idle.set_visible(state.available && state.inhibited);
    }

    fn refresh_nightlight(&self) {
        self.nightlight.set_visible(
            self.services.wayland.gamma_available() && self.services.wayland.gamma_enabled(),
        );
    }
}

fn set_icon(image: &gtk::Image, name: &str) {
    if image.icon_name().as_deref() != Some(name) {
        image.set_icon_name(Some(name));
    }
}

#[derive(Debug, PartialEq, Eq)]
struct NetworkView {
    visible: bool,
    airplane: bool,
    vpn: bool,
    icon: &'static str,
}

fn network_view(state: &NetworkState) -> NetworkView {
    let primary = state
        .devices
        .iter()
        .find(|device| Some(&device.id) == state.primary.as_ref());
    let primary_wifi = primary.is_some_and(|device| device.kind == 2);
    let icon = match state.state {
        40 if primary_wifi => "network-wireless-acquiring-symbolic",
        40 => "network-wired-acquiring-symbolic",
        50 | 60 if state.has_wifi() && state.wifi_state() == 30 => {
            "network-wireless-offline-symbolic"
        }
        50 | 60 if state.has_wifi() => "network-wireless-no-route-symbolic",
        50 | 60 => "network-wired-no-route-symbolic",
        70 if primary_wifi => state
            .access_points
            .iter()
            .find(|ap| ap.active && Some(&ap.device) == state.primary.as_ref())
            .map_or("network-wireless-offline-symbolic", |ap| {
                match ap.strength {
                    0..25 => "network-wireless-signal-weak-symbolic",
                    25..50 => "network-wireless-signal-ok-symbolic",
                    50..75 => "network-wireless-signal-good-symbolic",
                    _ => "network-wireless-signal-excellent-symbolic",
                }
            }),
        70 => "network-wired-symbolic",
        _ if state.has_wifi() => "network-wireless-offline-symbolic",
        _ => "network-wired-offline-symbolic",
    };
    NetworkView {
        visible: state.available && state.networking_enabled,
        airplane: state.available && !state.networking_enabled,
        // NetworkManager keeps connecting and disconnecting tunnels in this inventory.
        vpn: state.available
            && state
                .active_connections
                .iter()
                .any(|connection| matches!(connection.kind.as_str(), "vpn" | "wireguard")),
        icon,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct AudioView {
    icon: &'static str,
    microphone: bool,
    sensitive: bool,
    tooltip: Option<&'static str>,
}

fn audio_view(state: &AudioState) -> AudioView {
    let volume = state
        .default_sink
        .and_then(|id| state.node(id))
        .and_then(|node| node.volume.as_ref())
        .filter(|volume| state.available && volume.volume.is_finite());
    let icon = match volume {
        Some(volume) if !volume.mute && volume.volume > 0.0 && volume.volume < 0.25 => {
            "audio-volume-low-symbolic"
        }
        Some(volume) if !volume.mute && volume.volume > 0.0 && volume.volume < 0.5 => {
            "audio-volume-medium-symbolic"
        }
        Some(volume) if !volume.mute && volume.volume > 0.0 => "audio-volume-high-symbolic",
        _ => "audio-volume-muted-symbolic",
    };
    AudioView {
        icon,
        microphone: state.available && state.microphone_active(),
        sensitive: volume.is_some(),
        tooltip: volume
            .is_none()
            .then_some("Audio information is unavailable"),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct PowerView {
    icon: &'static str,
    tooltip: Option<&'static str>,
}

fn power_view(device: Option<&PowerDevice>) -> PowerView {
    PowerView {
        icon: device.map_or("battery-missing-symbolic", PowerDevice::preferred_icon_name),
        tooltip: device
            .is_none()
            .then_some("Power information is unavailable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{
        audio::{AudioNode, NodeKind, NodeState, Volume},
        network::{AccessPoint, ActiveConnection, Device},
        power::{DeviceKind, DeviceState},
    };

    fn wireless(state: u32) -> NetworkState {
        NetworkState {
            available: true,
            state,
            networking_enabled: true,
            wireless_enabled: true,
            wireless_hardware_enabled: true,
            primary: Some("wifi".into()),
            devices: vec![Device {
                id: "wifi".into(),
                interface: "wlan0".into(),
                description: "wlan0".into(),
                kind: 2,
                state: 100,
                managed: true,
                carrier: false,
                available_connections: Vec::new(),
                active_connection: None,
            }],
            ..NetworkState::default()
        }
    }

    fn sink(volume: f64, mute: bool) -> AudioState {
        AudioState {
            available: true,
            default_sink: Some(10),
            nodes: vec![AudioNode {
                id: 10,
                serial: 1,
                kind: NodeKind::Sink,
                name: "speaker".into(),
                description: String::new(),
                nickname: String::new(),
                application: String::new(),
                media: String::new(),
                state: NodeState::Idle,
                volume: Some(Volume {
                    volume,
                    mute,
                    step: 0.05,
                    base: 1.0,
                    channels: vec![],
                }),
            }],
            ..AudioState::default()
        }
    }

    #[test]
    fn wireless_and_wired_connection_states_preserve_icons() {
        let mut state = wireless(0);
        for value in [0, 10, 20, 30] {
            state.state = value;
            assert_eq!(
                network_view(&state).icon,
                "network-wireless-offline-symbolic"
            );
        }
        state.state = 40;
        assert_eq!(
            network_view(&state).icon,
            "network-wireless-acquiring-symbolic"
        );
        for value in [50, 60] {
            state.state = value;
            assert_eq!(
                network_view(&state).icon,
                "network-wireless-no-route-symbolic"
            );
            state.devices[0].state = 30;
            assert_eq!(
                network_view(&state).icon,
                "network-wireless-offline-symbolic"
            );
            state.devices[0].state = 100;
        }
        state.devices[0].kind = 1;
        for (value, icon) in [
            (0, "network-wired-offline-symbolic"),
            (10, "network-wired-offline-symbolic"),
            (20, "network-wired-offline-symbolic"),
            (30, "network-wired-offline-symbolic"),
            (40, "network-wired-acquiring-symbolic"),
            (50, "network-wired-no-route-symbolic"),
            (60, "network-wired-no-route-symbolic"),
            (70, "network-wired-symbolic"),
        ] {
            state.state = value;
            assert_eq!(network_view(&state).icon, icon);
        }
    }

    #[test]
    fn wireless_signal_uses_primary_device_and_exact_strength_boundaries() {
        let mut state = wireless(70);
        state.access_points = vec![AccessPoint {
            id: "ap".into(),
            device: "wifi".into(),
            ssid: vec![],
            name: String::new(),
            strength: 0,
            flags: 0,
            wpa_flags: 0,
            rsn_flags: 0,
            active: true,
        }];
        for (strength, level) in [
            (0, "weak"),
            (24, "weak"),
            (25, "ok"),
            (49, "ok"),
            (50, "good"),
            (74, "good"),
            (75, "excellent"),
            (100, "excellent"),
        ] {
            state.access_points[0].strength = strength;
            assert_eq!(
                network_view(&state).icon,
                format!("network-wireless-signal-{level}-symbolic")
            );
        }
        state.access_points[0].device = "other-wifi".into();
        assert_eq!(
            network_view(&state).icon,
            "network-wireless-offline-symbolic"
        );
        state.access_points[0].device = "wifi".into();
        state.access_points[0].active = false;
        assert_eq!(
            network_view(&state).icon,
            "network-wireless-offline-symbolic"
        );
        state.primary = None;
        assert_eq!(network_view(&state).icon, "network-wired-symbolic");
    }

    #[test]
    fn airplane_and_tunnel_visibility_distinguish_service_loss() {
        let mut state = wireless(70);
        assert!(!network_view(&state).airplane);
        assert!(!network_view(&state).vpn);
        state.networking_enabled = false;
        assert!(network_view(&state).airplane);
        assert!(!network_view(&state).visible);
        for kind in ["vpn", "wireguard"] {
            state.active_connections = vec![ActiveConnection {
                id: "tunnel".into(),
                connection: None,
                name: "Tunnel".into(),
                kind: kind.into(),
                state: 1,
                devices: vec![],
            }];
            assert!(network_view(&state).vpn);
        }
        state.active_connections[0].kind = "bridge".into();
        assert!(!network_view(&state).vpn);
        state.active_connections[0].kind = "vpn".into();
        state.available = false;
        assert!(!network_view(&state).airplane);
        assert!(!network_view(&state).vpn);
        assert!(!network_view(&state).visible);
    }

    #[test]
    fn speaker_volume_thresholds_and_mute_match_existing_panel() {
        for (volume, level) in [
            (0.0, "muted"),
            (0.001, "low"),
            (0.249, "low"),
            (0.25, "medium"),
            (0.499, "medium"),
            (0.5, "high"),
            (1.5, "high"),
        ] {
            assert_eq!(
                audio_view(&sink(volume, false)).icon,
                format!("audio-volume-{level}-symbolic")
            );
            assert_eq!(
                audio_view(&sink(volume, true)).icon,
                "audio-volume-muted-symbolic"
            );
        }
    }

    #[test]
    fn audio_removal_clears_old_volume_and_microphone_state() {
        let mut state = sink(1.0, false);
        let mut microphone = state.nodes[0].clone();
        microphone.id = 11;
        microphone.kind = NodeKind::Source;
        microphone.state = NodeState::Running;
        state.nodes.push(microphone);
        assert!(audio_view(&state).microphone);
        assert!(audio_view(&state).sensitive);
        state.default_sink = None;
        assert_eq!(audio_view(&state).icon, "audio-volume-muted-symbolic");
        assert!(!audio_view(&state).sensitive);
        assert!(audio_view(&state).microphone);
        state.available = false;
        assert!(!audio_view(&state).microphone);
        assert_eq!(
            audio_view(&state).tooltip,
            Some("Audio information is unavailable")
        );
        let mut invalid = sink(f64::NAN, false);
        assert!(!audio_view(&invalid).sensitive);
        invalid.nodes[0].volume = None;
        assert!(!audio_view(&invalid).sensitive);
    }

    #[test]
    fn power_uses_service_mapping_and_marks_missing_hardware() {
        assert_eq!(
            power_view(None),
            PowerView {
                icon: "battery-missing-symbolic",
                tooltip: Some("Power information is unavailable"),
            }
        );
        let mut device = PowerDevice {
            path: "/battery".into(),
            native_path: String::new(),
            vendor: String::new(),
            model: String::new(),
            serial: String::new(),
            kind: DeviceKind::Battery,
            state: DeviceState::Discharging,
            power_supply: true,
            present: true,
            rechargeable: true,
            online: false,
            percentage: Some(47.0),
            time_to_empty: 0,
            time_to_full: 0,
            icon_name: String::new(),
        };
        assert_eq!(power_view(Some(&device)).icon, "battery-level-40-symbolic");
        assert!(power_view(Some(&device)).tooltip.is_none());
        device.state = DeviceState::Charging;
        assert_eq!(
            power_view(Some(&device)).icon,
            "battery-level-40-charging-symbolic"
        );
        device.rechargeable = false;
        assert_eq!(power_view(Some(&device)).icon, "ac-adapter-symbolic");
    }
}
