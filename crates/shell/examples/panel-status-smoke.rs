//! Exercise the real status widget with a private NetworkManager and Wayland display.
use gio::prelude::*;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};
use way_shell::{
    services::{
        audio::AudioService,
        logind::{LogindService, SessionIdentity},
        network::NetworkService,
        power::PowerService,
        wayland::WaylandService,
    },
    ui::panel::status::{StatusAction, StatusBar, StatusServices},
};
#[path = "../tests/common/network.rs"]
mod fixture;
use fixture::{NAME, ROOT, connect, inventory, ownership, path, properties, wait};

const AP: &str = "/org/freedesktop/NetworkManager/AccessPoint/1";
const AP_INTERFACE: &str = "org.freedesktop.NetworkManager.AccessPoint";
const POWER: &str = "org.freedesktop.UPower";
const POWER_ROOT: &str = "/org/freedesktop/UPower";
const BATTERY: &str = "/org/freedesktop/UPower/devices/battery_fixture";
const POWER_DEVICE: &str = "org.freedesktop.UPower.Device";

fn power_name(connection: &gio::DBusConnection, own: bool) {
    connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            if own { "RequestName" } else { "ReleaseName" },
            Some(&if own {
                (POWER, 0_u32).to_variant()
            } else {
                (POWER,).to_variant()
            }),
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        )
        .unwrap();
}

fn settings(id: &str) -> gio::Settings {
    let source =
        gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)
            .unwrap();
    gio::Settings::new_full(
        &source.lookup(id, false).unwrap(),
        Some(&gio::memory_settings_backend_new()),
        None,
    )
}

fn children(widget: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut result = Vec::new();
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        result.push(widget);
    }
    result
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    gtk::init()?;
    assert!(gtk4_layer_shell::is_supported());
    let context = glib::MainContext::default();
    let _guard = context.acquire()?;
    let (_bus, address) = fixture::Bus::start();
    let daemon = connect(&address);
    let client = connect(&address);
    let mut inventory = inventory();
    inventory
        .get_mut(&path(fixture::DEVICE))
        .unwrap()
        .get_mut(&format!("{}.Wireless", fixture::DEVICE_IFACE))
        .unwrap()
        .extend([
            ("AccessPoints".into(), vec![path(AP)].to_variant()),
            ("ActiveAccessPoint".into(), path(AP).to_variant()),
        ]);
    inventory.insert(
        path(AP),
        HashMap::from([(
            AP_INTERFACE.into(),
            HashMap::from([
                ("Ssid".into(), b"Status fixture".to_vec().to_variant()),
                ("Strength".into(), 74_u8.to_variant()),
                ("Flags".into(), 0_u32.to_variant()),
                ("WpaFlags".into(), 0_u32.to_variant()),
                ("RsnFlags".into(), 0_u32.to_variant()),
            ]),
        )]),
    );
    let inventory = Rc::new(RefCell::new(inventory));
    let manager = gio::DBusNodeInfo::for_xml(
        "<node><interface name='org.freedesktop.DBus.ObjectManager'><method name='GetManagedObjects'><arg type='a{oa{sa{sv}}}' direction='out'/></method></interface></node>",
    )?;
    let mut registrations = Vec::new();
    for root in ["/org/freedesktop", ROOT] {
        let inventory = inventory.clone();
        registrations.push(
            daemon
                .register_object(root, &manager.interfaces()[0])
                .method_call(move |_, _, _, _, _, _, invocation| {
                    invocation.return_value(Some(&(inventory.borrow().clone(),).to_variant()));
                })
                .build()?,
        );
    }
    let manager = gio::DBusNodeInfo::for_xml(
        "<node><interface name='org.freedesktop.NetworkManager'><method name='GetPermissions'><arg type='a{ss}' direction='out'/></method></interface></node>",
    )?;
    registrations.push(
        daemon
            .register_object(ROOT, &manager.interfaces()[0])
            .method_call(|_, _, _, _, _, _, invocation| {
                invocation.return_value(Some(&(HashMap::<String, String>::new(),).to_variant()));
            })
            .build()?,
    );
    let power_info = gio::DBusNodeInfo::for_xml(
        "<node>
        <interface name='org.freedesktop.UPower'>
            <method name='EnumerateDevices'><arg type='ao' direction='out'/></method>
            <signal name='DeviceRemoved'><arg type='o'/></signal>
        </interface>
        <interface name='org.freedesktop.UPower.Device'>
            <property name='Type' type='u' access='read'/>
            <property name='State' type='u' access='read'/>
            <property name='PowerSupply' type='b' access='read'/>
            <property name='IsPresent' type='b' access='read'/>
            <property name='IsRechargeable' type='b' access='read'/>
            <property name='Percentage' type='d' access='read'/>
        </interface></node>",
    )?;
    registrations.push(
        daemon
            .register_object(POWER_ROOT, &power_info.interfaces()[0])
            .method_call(|_, _, _, _, _, _, invocation| {
                invocation.return_value(Some(&(vec![path(BATTERY)],).to_variant()));
            })
            .build()?,
    );
    registrations.push(
        daemon
            .register_object(BATTERY, &power_info.interfaces()[1])
            .property(|_, _, _, _, name| match name {
                "Type" | "State" => 2_u32.to_variant(),
                "PowerSupply" | "IsPresent" | "IsRechargeable" => true.to_variant(),
                "Percentage" => 47_f64.to_variant(),
                _ => unreachable!(),
            })
            .build()?,
    );

    for iteration in 0..3 {
        let network = NetworkService::on_connection(&client);
        let power = PowerService::on_connection(&client);
        let logind = LogindService::on_connection(
            &client,
            settings("org.ldelossa.way-shell.system"),
            SessionIdentity {
                uid: 1000,
                pid: std::process::id(),
                session_id: None,
            },
        );
        let wayland =
            WaylandService::with_settings(settings("org.ldelossa.way-shell.window-manager"))?;
        // No PipeWire connection is started: deterministic missing-hardware state;
        // audio signal/volume behavior is covered by the pure status tests.
        let audio: AudioService = glib::Object::new();
        let services = StatusServices {
            audio: audio.clone(),
            network: network.clone(),
            power: power.clone(),
            logind: logind.clone(),
            wayland: wayland.clone(),
        };
        let calls = Rc::new(Cell::new(0));
        let recorded = calls.clone();
        let callback_owner = Rc::new(());
        let weak_callback = Rc::downgrade(&callback_owner);
        let bar = StatusBar::new(services, move |action| {
            let _owner = &callback_owner;
            assert_eq!(action, StatusAction::ToggleQuickSettings);
            recorded.set(recorded.get() + 1);
        });
        let root = bar.widget().clone();
        let button = root
            .first_child()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        let icons = children(&button.child().unwrap());
        assert_eq!(icons.len(), 7);
        let images: Vec<_> = icons
            .iter()
            .take(5)
            .map(|widget| widget.clone().downcast::<gtk::Image>().unwrap())
            .collect();
        assert_eq!(
            images[0].icon_name().as_deref(),
            Some("radio-mixed-symbolic")
        );
        assert_eq!(
            images[1].icon_name().as_deref(),
            Some("night-light-symbolic")
        );
        assert_eq!(
            images[2].icon_name().as_deref(),
            Some("airplane-mode-symbolic")
        );
        assert_eq!(
            images[3].icon_name().as_deref(),
            Some("network-vpn-symbolic")
        );
        let sound = children(&icons[5]);
        assert_eq!(sound.len(), 2);
        assert!(!sound[0].is_visible());
        assert!(!sound[1].is_sensitive());
        assert_eq!(
            icons[6].tooltip_text().as_deref(),
            Some("Power information is unavailable")
        );
        let power_icon = icons[6].clone().downcast::<gtk::Image>().unwrap();
        assert!(!images[0].is_visible());
        assert!(!images[2].is_visible());
        assert!(!images[4].is_visible());
        assert!(button.has_css_class("panel-button"));
        assert!(!bar.is_toggled());
        bar.set_toggled(true);
        assert!(bar.is_toggled());
        assert!(button.has_css_class("panel-button-toggled"));
        bar.set_toggled(false);
        assert!(!button.has_css_class("panel-button-toggled"));
        button.emit_clicked();
        assert_eq!(calls.get(), 1);

        let window = gtk::Window::new();
        window.init_layer_shell();
        window.set_layer(Layer::Top);
        window.set_namespace(Some("way-shell-status-test"));
        window.set_anchor(Edge::Top, true);
        window.set_default_size(320, 40);
        window.set_child(Some(&root));
        window.present();
        wait(&context, || window.is_mapped() && wayland.is_ready());

        ownership(&daemon, true);
        wait(&context, || {
            images[4].icon_name().as_deref() == Some("network-wireless-signal-good-symbolic")
        });
        assert!(images[4].is_visible());
        properties(
            &daemon,
            AP,
            AP_INTERFACE,
            HashMap::from([("Strength".into(), 75_u8.to_variant())]),
        );
        wait(&context, || {
            images[4].icon_name().as_deref() == Some("network-wireless-signal-excellent-symbolic")
        });
        properties(
            &daemon,
            ROOT,
            NAME,
            HashMap::from([("NetworkingEnabled".into(), false.to_variant())]),
        );
        wait(&context, || {
            images[2].is_visible() && !images[4].is_visible()
        });
        ownership(&daemon, false);
        wait(&context, || {
            !network.state().available && !images[2].is_visible()
        });
        ownership(&daemon, true);
        wait(&context, || {
            images[4].is_visible() && network.state().available
        });
        power_name(&daemon, true);
        wait(&context, || {
            power_icon.icon_name().as_deref() == Some("battery-level-40-symbolic")
        });
        assert!(power_icon.tooltip_text().is_none());
        properties(
            &daemon,
            BATTERY,
            POWER_DEVICE,
            HashMap::from([
                ("Percentage".into(), 83_f64.to_variant()),
                ("State".into(), 1_u32.to_variant()),
            ]),
        );
        wait(&context, || {
            power_icon.icon_name().as_deref() == Some("battery-level-80-charging-symbolic")
        });
        daemon.emit_signal(
            None,
            POWER_ROOT,
            POWER,
            "DeviceRemoved",
            Some(&(path(BATTERY),).to_variant()),
        )?;
        wait(&context, || {
            power_icon.icon_name().as_deref() == Some("battery-missing-symbolic")
        });
        assert_eq!(
            power_icon.tooltip_text().as_deref(),
            Some("Power information is unavailable")
        );
        power_name(&daemon, false);

        if iteration == 0 {
            // A widget observer may stop its service while a radio update is
            // applying. The outer snapshot must not show connected state again.
            let stopped_network = NetworkService::on_connection(&client);
            let probe = StatusBar::new(
                StatusServices {
                    network: stopped_network.clone(),
                    audio: audio.clone(),
                    power: power.clone(),
                    logind: logind.clone(),
                    wayland: wayland.clone(),
                },
                |_| {},
            );
            let probe_button = probe
                .widget()
                .first_child()
                .unwrap()
                .downcast::<gtk::Button>()
                .unwrap();
            let probe_icons = children(&probe_button.child().unwrap());
            let airplane = probe_icons[2].clone();
            let network_icon = probe_icons[4].clone();
            wait(&context, || stopped_network.state().available);
            properties(
                &daemon,
                ROOT,
                NAME,
                HashMap::from([("NetworkingEnabled".into(), false.to_variant())]),
            );
            wait(&context, || airplane.is_visible());
            let stopped = Rc::new(Cell::new(false));
            let flag = stopped.clone();
            let service = stopped_network.clone();
            let handler = airplane.connect_visible_notify(move |icon| {
                if !icon.is_visible() && !flag.replace(true) {
                    service.stop();
                }
            });
            properties(
                &daemon,
                ROOT,
                NAME,
                HashMap::from([("NetworkingEnabled".into(), true.to_variant())]),
            );
            wait(&context, || {
                stopped.get() && !stopped_network.state().available
            });
            airplane.disconnect(handler);
            assert!(
                !network_icon.is_visible(),
                "outer radio snapshot restored an icon after the service stopped"
            );
            assert!(!airplane.is_visible());
            drop(probe);
            drop(stopped_network);
        }

        let weak_network = network.downgrade();
        let weak_power = power.downgrade();
        let weak_audio = audio.downgrade();
        let weak_logind = logind.downgrade();
        let weak_wayland = wayland.downgrade();
        drop(bar);
        assert!(weak_callback.upgrade().is_none());
        button.emit_clicked();
        assert_eq!(calls.get(), 1);
        // A retained GTK root must stop receiving live service notifications.
        images[4].set_icon_name(Some("window-close-symbolic"));
        properties(
            &daemon,
            AP,
            AP_INTERFACE,
            HashMap::from([("Strength".into(), 1_u8.to_variant())]),
        );
        wait(&context, || {
            network
                .state()
                .access_points
                .iter()
                .any(|ap| ap.strength == 1)
        });
        assert_eq!(
            images[4].icon_name().as_deref(),
            Some("window-close-symbolic")
        );
        network.stop();
        logind.stop();
        audio.stop();
        drop((network, power, audio, logind, wayland));
        wait(&context, || {
            weak_network.upgrade().is_none()
                && weak_power.upgrade().is_none()
                && weak_audio.upgrade().is_none()
                && weak_logind.upgrade().is_none()
                && weak_wayland.upgrade().is_none()
        });
        window.close();
        window.set_child(gtk::Widget::NONE);
        ownership(&daemon, false);
        context.block_on(glib::timeout_future(Duration::from_millis(10)));
    }
    for registration in registrations {
        daemon.unregister_object(registration)?;
    }
    client.close_sync(gio::Cancellable::NONE)?;
    daemon.close_sync(gio::Cancellable::NONE)?;
    println!("three status icon/state/restart/action/cleanup cycles passed");
    Ok(())
}
