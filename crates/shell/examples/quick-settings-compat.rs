//! The composed popup shares services, coordinates menus and closes cleanly.
use adw::prelude::*;
use std::{path::PathBuf, rc::Rc, time::Duration};
use way_shell::{
    services::{
        audio::AudioService,
        brightness::BrightnessService,
        logind::{LogindService, SessionIdentity},
        network::NetworkService,
        notifications::NotificationsService,
        power::PowerService,
        power_profiles::PowerProfilesService,
        theme::ThemeService,
        wayland::WaylandService,
    },
    ui::{
        quick_settings::{
            controller::{QuickSettings, QuickSettingsServices},
            controls::SystemServices,
        },
        window::Visibility,
    },
};
#[allow(dead_code)]
#[path = "../tests/common/audio.rs"]
mod audio;
#[path = "../tests/common/bluetooth.rs"]
mod bluetooth_fixture;
#[allow(dead_code)]
#[path = "../tests/common/network.rs"]
mod bus;

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
fn descendants(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut children = Vec::new();
    let mut next = root.first_child();
    while let Some(child) = next {
        next = child.next_sibling();
        children.extend(descendants(&child));
        children.push(child);
    }
    children
}
fn icon_button(root: &impl IsA<gtk::Widget>, icon: &str) -> gtk::Button {
    descendants(root)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| button.icon_name().as_deref() == Some(icon))
        .unwrap()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    adw::init()?;
    let context = glib::MainContext::default();
    let _guard = context.acquire()?;
    let (_bus, address) = bus::Bus::start();
    let client = bus::connect(&address);
    let daemon = audio::Daemon::new();
    let audio = AudioService::with_remote(&daemon.remote());
    bus::wait(&context, || audio.state().available);
    let system = settings("org.ldelossa.way-shell.system");
    // Explicitly unconfigured hardware is unavailable without a failed read.
    system.set_string("backlight-directory", "")?;
    system.set_string("keyboard-backlight-directory", "")?;
    let directory = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").unwrap());
    let theme = ThemeService::with_settings(system.clone(), directory.join("quick-settings-theme"));
    let brightness = BrightnessService::with_settings_and_root(
        &system,
        &directory.join("no-sysfs"),
        Rc::new(|_, _, _, _| panic!("unavailable fixture brightness must not request writes")),
    );
    let logind = LogindService::on_connection(
        &client,
        system.clone(),
        SessionIdentity {
            uid: 1000,
            pid: std::process::id(),
            session_id: Some("fixture".into()),
        },
    );
    let profiles = PowerProfilesService::on_connection(&client);
    let network = NetworkService::on_connection(&client);
    let bluetooth_daemon = bluetooth_fixture::Fake::new(&address);
    let bluetooth = way_shell::services::bluetooth::BluetoothService::on_connection(&client, None);
    bus::wait(&context, || bluetooth.state().ready);
    let power = PowerService::on_connection(&client);
    let notifications = NotificationsService::on_connection(&client);
    let wayland = WaylandService::with_settings(settings("org.ldelossa.way-shell.window-manager"))?;
    for _ in 0..3 {
        let view = QuickSettings::new(
            QuickSettingsServices {
                system: SystemServices {
                    theme: theme.clone(),
                    logind: logind.clone(),
                    profiles: profiles.clone(),
                    brightness: brightness.clone(),
                    wayland: wayland.clone(),
                },
                audio: audio.clone(),
                network: network.clone(),
                bluetooth: bluetooth.clone(),
                power: power.clone(),
                notifications: notifications.clone(),
            },
            system.clone(),
            |_| panic!("unavailable session must not request confirmation"),
        )?;
        view.window().show();
        bus::wait(&context, || {
            view.window().visibility_controller().visibility() == Visibility::Visible
        });
        let bluetooth_label = descendants(view.window().content())
            .into_iter()
            .find_map(|w| {
                w.downcast::<gtk::Label>().ok().filter(|l| {
                    l.label() == "Bluetooth" && l.has_css_class("quick-settings-grid-button-title")
                })
            })
            .unwrap();
        let mut ancestor = bluetooth_label.upcast::<gtk::Widget>();
        let bluetooth_button = loop {
            if let Ok(button) = ancestor.clone().downcast::<gtk::Button>() {
                break button;
            }
            ancestor = ancestor.parent().unwrap();
        };
        bluetooth_button.emit_clicked();
        bus::wait(&context, || {
            !bluetooth.state().powered && !bluetooth.state().busy
        });
        assert!(!bluetooth_daemon.value(bluetooth_fixture::HCI, "Powered"));
        bluetooth_button.emit_clicked();
        bus::wait(&context, || {
            bluetooth.state().powered && !bluetooth.state().busy
        });
        let mixer = icon_button(view.window().content(), "audio-speakers-symbolic");
        let power_button = icon_button(view.window().content(), "system-shutdown-symbolic");
        let sink = descendants(view.window().content())
            .into_iter()
            .find(|widget| widget.widget_name() == "default-sink-container")
            .unwrap();
        let scales = sink
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .downcast::<gtk::Revealer>()
            .unwrap();
        assert!(scales.reveals_child());
        assert!(mixer.is_sensitive());
        mixer.emit_clicked();
        assert!(!scales.reveals_child());
        assert!(view.window().window().has_css_class("focused"));
        power_button.emit_clicked();
        assert!(scales.reveals_child());
        view.window().underlay_button().emit_clicked();
        bus::wait(&context, || {
            view.window().visibility_controller().visibility() == Visibility::Hidden
        });
        assert!(!view.window().window().has_css_class("focused"));
        view.window().show();
        bus::wait(&context, || {
            view.window().visibility_controller().visibility() == Visibility::Visible
        });
        mixer.emit_clicked();
        assert!(
            !scales.reveals_child(),
            "closing the view must reset header selection"
        );
        let retained_window = view.window().window().clone();
        let weak = Rc::downgrade(&view);
        drop(view);
        assert!(weak.upgrade().is_none());
        mixer.emit_clicked();
        power_button.emit_clicked();
        bluetooth_button.emit_clicked();
        assert!(bluetooth.state().powered);
        context.block_on(glib::timeout_future(Duration::from_millis(300)));
        assert!(!retained_window.is_visible());
        assert!(!mixer.is_sensitive());
        assert!(!scales.reveals_child());
        assert!(
            audio.state().available,
            "widget cleanup must not stop a shared service"
        );
    }
    audio.stop();
    network.stop();
    bluetooth.stop();
    logind.stop();
    notifications.stop();
    println!("composed quick-settings menu, shared-service and lifetime checks passed");
    Ok(())
}
