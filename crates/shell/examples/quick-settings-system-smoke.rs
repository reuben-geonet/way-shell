//! Exercise real system controls and their ownership on isolated Sway/Niri.
use adw::prelude::*;
use glib::variant::ObjectPath;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    fs,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    rc::Rc,
    time::{Duration, Instant},
};
use way_shell::{
    services::{
        brightness::{Apply, BrightnessService, Completion, ControlKind},
        logind::{LogindService, PowerAction, SessionIdentity},
        notifications::NotificationsService,
        power::PowerService,
        power_profiles::PowerProfilesService,
        theme::{Theme, ThemeService},
        wayland::WaylandService,
    },
    ui::{
        quick_settings::{
            controls::{SystemControls, SystemServices},
            grid::{Grid, GridButton},
            header::{HeaderMenu, MixerSlot, SystemHeader},
            power_menu::{Confirmation, SystemAction},
        },
        window::{LayerWindow, WindowRole},
    },
};

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
fn until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "quick-settings fixture timed out"
        );
        glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}
struct Bus(Child);
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn bus() -> (Bus, gio::DBusConnection, gio::DBusConnection) {
    let config = format!(
        "--config-file={}/../../tests/fixtures/session-bus.conf",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut child = Command::new("dbus-daemon")
        .args(["--nofork", "--print-address=1", &config])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut address = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    let connect = || {
        let connection = gio::DBusConnection::for_address_sync(
            address.trim(),
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            gio::Cancellable::NONE,
        )
        .unwrap();
        connection.set_exit_on_close(false);
        connection
    };
    (Bus(child), connect(), connect())
}
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Pending {
    kind: ControlKind,
    value: u32,
    done: Completion,
}
fn name(connection: &gio::DBusConnection, name: &str, acquire: bool) {
    connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            if acquire {
                "RequestName"
            } else {
                "ReleaseName"
            },
            Some(&if acquire {
                (name, 0_u32).to_variant()
            } else {
                (name,).to_variant()
            }),
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        )
        .unwrap();
}
fn profile_button(root: &gtk::Widget, text: &str) -> Option<gtk::Button> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>()
            && let Some(label) = button
                .child()
                .and_then(|contents| contents.last_child())
                .and_downcast::<gtk::Label>()
            && label.text() == text
        {
            return Some(button);
        }
        if let Some(button) = profile_button(&widget, text) {
            return Some(button);
        }
        child = widget.next_sibling();
    }
    None
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    gtk::init()?;
    let window = LayerWindow::new(WindowRole::QuickSettings, None)?;
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    window.window().set_content(Some(&root));
    let (_bus, daemon, client) = bus();
    let calls = Rc::new(RefCell::new(Vec::<String>::new()));
    let mut xml = String::from(
        "<node><interface name='org.freedesktop.login1.Manager'><method name='GetSessionByPID'><arg type='u' direction='in'/><arg type='o' direction='out'/></method><method name='GetSession'><arg type='s' direction='in'/><arg type='o' direction='out'/></method>",
    );
    for method in [
        "Reboot",
        "PowerOff",
        "Suspend",
        "Hibernate",
        "HybridSleep",
        "SuspendThenHibernate",
    ] {
        xml.push_str(&format!("<method name='Can{method}'><arg type='s' direction='out'/></method><method name='{method}'><arg type='b' direction='in'/></method>"));
    }
    xml.push_str("</interface></node>");
    let info = gio::DBusNodeInfo::for_xml(&xml)?;
    let actions = calls.clone();
    let manager = daemon
        .register_object(
            "/org/freedesktop/login1",
            &info
                .lookup_interface("org.freedesktop.login1.Manager")
                .unwrap(),
        )
        .method_call(move |_, _, _, _, method, args, invocation| match method {
            "GetSession" | "GetSessionByPID" => invocation.return_value(Some(
                &(ObjectPath::try_from("/org/freedesktop/login1/session/own").unwrap(),)
                    .to_variant(),
            )),
            method if method.starts_with("Can") => {
                invocation.return_value(Some(&("yes",).to_variant()))
            }
            _ => {
                assert_eq!(args.get::<(bool,)>(), Some((false,)));
                actions.borrow_mut().push(method.into());
                invocation.return_value(None);
            }
        })
        .build()?;
    let info = gio::DBusNodeInfo::for_xml(
        "<node><interface name='org.freedesktop.login1.Session'><property name='Id' type='s' access='read'/><property name='User' type='(uo)' access='read'/><property name='State' type='s' access='read'/><method name='Terminate'/></interface></node>",
    )?;
    let actions = calls.clone();
    let session = daemon
        .register_object(
            "/org/freedesktop/login1/session/own",
            &info
                .lookup_interface("org.freedesktop.login1.Session")
                .unwrap(),
        )
        .property(|_, _, _, _, property| match property {
            "Id" => "own".to_variant(),
            "State" => "active".to_variant(),
            "User" => (
                1000_u32,
                ObjectPath::try_from("/org/freedesktop/login1/user/1000").unwrap(),
            )
                .to_variant(),
            _ => unreachable!(),
        })
        .method_call(move |_, _, _, _, method, _, invocation| {
            actions.borrow_mut().push(method.into());
            invocation.return_value(None);
        })
        .build()?;
    daemon.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "RequestName",
        Some(&("org.freedesktop.login1", 0_u32).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    )?;
    let system = settings("org.ldelossa.way-shell.system");
    let logind = LogindService::on_connection(
        &client,
        system.clone(),
        SessionIdentity {
            uid: 1000,
            pid: std::process::id(),
            session_id: Some("own".into()),
        },
    );
    until(|| logind.can(PowerAction::Suspend) && logind.state().session.is_some());
    let directory = Directory(
        std::env::temp_dir().join(format!("way-shell-system-controls-{}", std::process::id())),
    );
    for (kind, name, current, max) in [
        ("backlight", "panel", "2", "5"),
        ("leds", "keyboard", "3", "3"),
    ] {
        let path = directory.0.join(kind).join(name);
        fs::create_dir_all(&path)?;
        fs::write(path.join("brightness"), current)?;
        fs::write(path.join("max_brightness"), max)?;
    }
    system.set_string("backlight-directory", "panel")?;
    system.set_string("keyboard-backlight-directory", "keyboard")?;
    let pending = Rc::new(RefCell::new(VecDeque::<Pending>::new()));
    let queue = pending.clone();
    let apply: Apply = Rc::new(move |kind, _, value, done| {
        queue.borrow_mut().push_back(Pending { kind, value, done });
        gio::Cancellable::new()
    });
    let brightness = BrightnessService::with_settings_and_root(&system, &directory.0, apply);
    let theme = ThemeService::with_settings(system.clone(), directory.0.clone());
    let active_profile = Rc::new(RefCell::new(String::from("balanced")));
    let active = active_profile.clone();
    let selected = active_profile.clone();
    let profile_info = gio::DBusNodeInfo::for_xml(
        "<node><interface name='net.hadess.PowerProfiles'><property name='ActiveProfile' type='s' access='readwrite'/><property name='Profiles' type='aa{sv}' access='read'/></interface></node>",
    )?;
    let profile_registration = daemon
        .register_object(
            "/net/hadess/PowerProfiles",
            &profile_info
                .lookup_interface("net.hadess.PowerProfiles")
                .unwrap(),
        )
        .property(move |_, _, _, _, property| match property {
            "ActiveProfile" => active.borrow().to_variant(),
            "Profiles" => ["balanced", "power-saver"]
                .map(|name| HashMap::from([("Profile", name.to_variant())]))
                .to_vec()
                .to_variant(),
            _ => unreachable!(),
        })
        .set_property(move |connection, _, _, _, property, value| {
            selected.replace(value.get::<String>().unwrap());
            connection
                .emit_signal(
                    None,
                    "/net/hadess/PowerProfiles",
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    Some(
                        &(
                            "net.hadess.PowerProfiles",
                            HashMap::from([(property, value)]),
                            Vec::<String>::new(),
                        )
                            .to_variant(),
                    ),
                )
                .unwrap();
            true
        })
        .build()?;
    let profiles = PowerProfilesService::on_connection(&client);
    let wayland = WaylandService::with_settings(settings("org.ldelossa.way-shell.window-manager"))?;
    until(|| wayland.is_ready());
    let controls = SystemControls::new(SystemServices {
        theme: theme.clone(),
        logind: logind.clone(),
        profiles: profiles.clone(),
        brightness: brightness.clone(),
        wayland: wayland.clone(),
    });
    let focused = Rc::new(Cell::new(false));
    let state = focused.clone();
    let grid = Grid::new(move |value| state.set(value));
    let tiles = controls.ordered_buttons([], None);
    grid.set_buttons(tiles.clone());
    root.append(grid.widget());
    root.append(controls.brightness_row());
    window.present();
    until(|| window.window().is_mapped());
    assert_eq!(tiles.len(), 5);
    assert!(!tiles[0].widget().is_sensitive());
    assert_eq!(tiles[2].widget().is_sensitive(), wayland.gamma_available());
    assert_eq!(
        controls.temperature_scale().is_sensitive(),
        wayland.gamma_available()
    );
    if wayland.gamma_available() {
        controls.temperature_scale().set_value(2500.0);
        assert!(wayland.gamma_enabled());
        tiles[2].toggle().emit_clicked();
        assert!(!wayland.gamma_enabled());
        assert_eq!(controls.temperature_scale().value(), 3000.0);
    }
    name(&daemon, "net.hadess.PowerProfiles", true);
    until(|| tiles[0].widget().is_sensitive());
    tiles[0].toggle_menu();
    assert!(focused.get());
    let saver = profile_button(&tiles[0].revealer().child().unwrap(), "power-saver").unwrap();
    saver.emit_clicked();
    until(|| profiles.state().active.as_deref() == Some("power-saver"));
    assert!(!tiles[0].revealer().reveals_child());
    assert!(!focused.get());
    name(&daemon, "net.hadess.PowerProfiles", false);
    until(|| !tiles[0].widget().is_sensitive());
    name(&daemon, "net.hadess.PowerProfiles", true);
    until(|| tiles[0].widget().is_sensitive());
    assert_eq!(controls.brightness_scale().value(), 0.4);
    assert_eq!(controls.keyboard_scale().value(), 3.0);
    assert!(pending.borrow().is_empty());
    tiles[4].toggle().emit_clicked();
    assert_eq!(theme.theme(), Theme::Light);
    tiles[4].toggle().emit_clicked();
    assert_eq!(theme.theme(), Theme::Dark);
    tiles[3].toggle().emit_clicked();
    let request = pending.borrow_mut().pop_front().unwrap();
    assert_eq!(request.kind, ControlKind::Keyboard);
    assert_eq!(request.value, 0);
    (request.done)(Err(glib::Error::new(
        gio::IOErrorEnum::PermissionDenied,
        "fixture denied",
    )));
    assert_eq!(controls.keyboard_scale().value(), 3.0);
    assert!(
        tiles[3]
            .widget()
            .tooltip_text()
            .unwrap()
            .contains("fixture denied")
    );
    controls.brightness_scale().set_value(0.8);
    let request = pending.borrow_mut().pop_front().unwrap();
    assert_eq!(request.value, 4);
    fs::write(directory.0.join("backlight/panel/brightness"), "4")?;
    (request.done)(Ok(()));
    until(|| {
        brightness
            .snapshot(ControlKind::Backlight)
            .is_some_and(|device| device.current == 4)
    });
    assert_eq!(controls.brightness_scale().value(), 0.8);
    assert!(
        pending.borrow().is_empty(),
        "observed slider changes must not issue writes"
    );
    brightness.set_backend_available(false);
    assert!(!controls.brightness_row().get_visible());
    assert!(!tiles[3].widget().is_sensitive());
    brightness.set_backend_available(true);
    assert!(controls.brightness_row().get_visible());
    assert!(tiles[3].widget().is_sensitive());
    let menu_a = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let menu_b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let first = GridButton::new("A", None, "test", Some(menu_a.upcast_ref()));
    let second = GridButton::new("B", None, "test", Some(menu_b.upcast_ref()));
    grid.set_buttons(vec![first.clone(), second.clone()]);
    first.toggle_menu();
    assert!(first.revealer().reveals_child());
    first.toggle_menu();
    assert!(
        !first.revealer().reveals_child(),
        "rapid clicks use desired, not finished animation state"
    );
    first.toggle_menu();
    second.toggle_menu();
    assert!(!first.revealer().reveals_child());
    assert!(second.revealer().reveals_child());
    grid.set_buttons(vec![second.clone(), first.clone(), second.clone()]);
    assert!(second.revealer().reveals_child());
    let weak_grid = Rc::downgrade(&grid);
    let replacement = second.clone();
    let nested = Rc::new(Cell::new(false));
    let called = nested.clone();
    let handler = first.widget().connect_parent_notify(move |_| {
        if !called.replace(true)
            && let Some(grid) = weak_grid.upgrade()
        {
            grid.set_buttons(vec![replacement.clone()]);
        }
    });
    grid.set_buttons(vec![first.clone()]);
    first.widget().disconnect(handler);
    assert!(nested.get());
    assert!(first.widget().parent().is_none());
    assert!(second.widget().parent().is_some());
    grid.hide_menus();
    assert!(!focused.get());
    let requests = Rc::new(RefCell::new(Vec::<Confirmation>::new()));
    let output = requests.clone();
    let mixer_button = gtk::Button::with_label("Mixer");
    let mixer_state = Rc::new(Cell::new(false));
    let visible = mixer_state.clone();
    let header = SystemHeader::new(
        glib::Object::new::<PowerService>(),
        glib::Object::new::<NotificationsService>(),
        logind.clone(),
        system,
        Some(MixerSlot {
            button: mixer_button.clone(),
            content: gtk::Box::new(gtk::Orientation::Vertical, 0).upcast(),
            revealed: Rc::new(move |shown| visible.set(shown)),
        }),
        |_| {},
        move |request| output.borrow_mut().push(request),
    );
    root.prepend(header.widget());
    assert!(!header.battery_button().is_sensitive());
    header.power_button().emit_clicked();
    assert_eq!(header.selected_menu(), Some(HeaderMenu::Power));
    mixer_button.emit_clicked();
    assert_eq!(header.selected_menu(), Some(HeaderMenu::Mixer));
    assert!(mixer_state.get());
    header.power_button().emit_clicked();
    assert!(!mixer_state.get());
    header
        .power_menu()
        .action_button(SystemAction::Suspend)
        .emit_clicked();
    assert!(requests.borrow().is_empty());
    assert!(calls.borrow().is_empty());
    header
        .power_menu()
        .confirmation_button(SystemAction::Suspend)
        .emit_clicked();
    assert_eq!(requests.borrow().len(), 1);
    assert!(calls.borrow().is_empty());
    (requests.borrow_mut().pop().unwrap().respond)(false);
    assert!(calls.borrow().is_empty());
    header
        .power_menu()
        .action_button(SystemAction::Suspend)
        .emit_clicked();
    header
        .power_menu()
        .confirmation_button(SystemAction::Suspend)
        .emit_clicked();
    let response = requests.borrow_mut().pop().unwrap();
    assert_eq!(response.action, SystemAction::Suspend);
    (response.respond)(true);
    until(|| calls.borrow().as_slice() == ["Suspend"]);
    header
        .power_menu()
        .action_button(SystemAction::Logout)
        .emit_clicked();
    header
        .power_menu()
        .confirmation_button(SystemAction::Logout)
        .emit_clicked();
    let orphan = requests.borrow_mut().pop().unwrap();
    let retained = header.widget().clone();
    let weak = Rc::downgrade(&header);
    drop(header);
    assert!(weak.upgrade().is_none());
    (orphan.respond)(true);
    mixer_button.emit_clicked();
    assert_eq!(calls.borrow().len(), 1);
    assert!(!retained.is_sensitive());
    window.present();
    until(|| window.window().is_mapped());
    let weak = Rc::downgrade(&controls);
    drop(controls);
    assert!(weak.upgrade().is_none());
    tiles[4].toggle().emit_clicked();
    assert_eq!(theme.theme(), Theme::Dark);
    grid.stop();
    second.toggle_menu();
    assert!(!second.revealer().reveals_child());
    window.close();
    logind.stop();
    daemon.unregister_object(manager)?;
    daemon.unregister_object(session)?;
    daemon.unregister_object(profile_registration)?;
    println!("quick-settings system controls passed");
    Ok(())
}
