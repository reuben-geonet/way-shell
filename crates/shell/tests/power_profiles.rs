use gio::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    rc::Rc,
    time::{Duration, Instant},
};
use way_shell::services::power_profiles::{self, PowerProfilesService};

const NAME: &str = "net.hadess.PowerProfiles";
const PATH: &str = "/net/hadess/PowerProfiles";
struct Bus(Child);
impl Bus {
    fn start() -> (Self, String) {
        let config = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/session-bus.conf"
        );
        let mut child = Command::new("dbus-daemon")
            .args([
                "--nofork",
                "--print-address=1",
                &format!("--config-file={config}"),
            ])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        assert!(!address.trim().is_empty());
        (Self(child), address.trim().to_owned())
    }
}
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn connect(address: &str) -> gio::DBusConnection {
    let connection = gio::DBusConnection::for_address_sync(
        address,
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        gio::Cancellable::NONE,
    )
    .unwrap();
    connection.set_exit_on_close(false);
    connection
}
fn name(connection: &gio::DBusConnection, acquire: bool) {
    let parameters = if acquire {
        (NAME, 0_u32).to_variant()
    } else {
        (NAME,).to_variant()
    };
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
            Some(&parameters),
            None,
            gio::DBusCallFlags::NONE,
            2_000,
            gio::Cancellable::NONE,
        )
        .unwrap();
}
fn wait(context: &glib::MainContext, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "Power profiles did not reach expected state"
        );
        context.block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}
fn profiles(names: &[&str], provider: &str) -> glib::Variant {
    names
        .iter()
        .map(|name| {
            HashMap::from([
                ("Profile".to_owned(), name.to_variant()),
                ("Driver".to_owned(), provider.to_variant()),
            ])
        })
        .collect::<Vec<_>>()
        .to_variant()
}
fn changed(connection: &gio::DBusConnection, property: &str, value: glib::Variant) {
    connection
        .emit_signal(
            None,
            PATH,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            Some(
                &(
                    NAME,
                    HashMap::from([(property, value)]),
                    Vec::<String>::new(),
                )
                    .to_variant(),
            ),
        )
        .unwrap();
}

#[test]
fn provider_contract_property_changes_failures_restart_and_cleanup() {
    let (mut bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context.with_thread_default(|| {
        let daemon = connect(&address);
        let client = connect(&address);
        let active = Rc::new(RefCell::new("balanced".to_owned()));
        let inventory = Rc::new(RefCell::new(profiles(&["balanced", "power-saver"], "platform_profile")));
        let allow = Rc::new(Cell::new(true));
        let get_active = active.clone();
        let get_inventory = inventory.clone();
        let set_active = active.clone();
        let set_allow = allow.clone();
        let info = gio::DBusNodeInfo::for_xml("<node><interface name='net.hadess.PowerProfiles'><property name='ActiveProfile' type='s' access='readwrite'/><property name='Profiles' type='aa{sv}' access='read'/></interface></node>").unwrap();
        let registration = daemon.register_object(PATH, &info.lookup_interface(NAME).unwrap())
            .property(move |_, _, _, _, property| match property {
                "Profiles" => get_inventory.borrow().clone(),
                "ActiveProfile" => get_active.borrow().to_variant(),
                _ => unreachable!(),
            })
            .set_property(move |connection, _, _, _, property, value| {
                assert_eq!(property, "ActiveProfile");
                if !set_allow.get() { return false; }
                set_active.replace(value.get::<String>().unwrap());
                changed(&connection, property, value);
                true
            }).build().unwrap();
        let service = PowerProfilesService::on_connection(&client);
        context.block_on(glib::timeout_future(Duration::from_millis(20)));
        assert!(!service.state().available());
        let result = Rc::new(RefCell::new(None));
        let recorded = result.clone();
        service.set_profile("balanced", move |reply| { recorded.replace(Some(reply)); });
        assert!(result.borrow_mut().take().unwrap().unwrap_err().matches(gio::IOErrorEnum::NotConnected));

        name(&daemon, true);
        wait(&context, || service.state().available());
        assert_eq!(service.state().profiles, ["balanced", "power-saver"]);
        let updates = Rc::new(Cell::new(0));
        let recorded = updates.clone();
        service.connect_local("changed", false, move |_| { recorded.set(recorded.get() + 1); None });
        let replacement = profiles(&["balanced", "power-saver", "performance"], "platform_profile");
        inventory.replace(replacement.clone());
        changed(&daemon, "Profiles", replacement);
        wait(&context, || service.state().profiles.len() == 3);
        assert_eq!(updates.get(), 1, "Profiles notification must update inventory exactly once");

        let recorded = result.clone();
        service.set_profile("performance", move |reply| { recorded.replace(Some(reply)); });
        wait(&context, || result.borrow().is_some() && service.state().active.as_deref() == Some("performance"));
        result.borrow_mut().take().unwrap().unwrap();
        allow.set(false);
        let recorded = result.clone();
        service.set_profile("power-saver", move |reply| { recorded.replace(Some(reply)); });
        wait(&context, || result.borrow().is_some());
        assert!(result.borrow_mut().take().unwrap().is_err());
        assert_eq!(service.state().active.as_deref(), Some("performance"));
        let recorded = result.clone();
        service.set_profile("unsupported", move |reply| { recorded.replace(Some(reply)); });
        assert!(result.borrow_mut().take().unwrap().unwrap_err().matches(gio::IOErrorEnum::InvalidArgument));

        // The same advertised interface supports tuned-ppd after provider restart.
        name(&daemon, false);
        wait(&context, || !service.state().available());
        assert!(service.state().profiles.is_empty());
        active.replace("power-saver".into());
        inventory.replace(profiles(&["power-saver", "balanced"], "tuned"));
        name(&daemon, true);
        wait(&context, || service.state().available());
        assert_eq!(service.state().active.as_deref(), Some("power-saver"));
        assert_eq!(service.state().profiles, ["power-saver", "balanced"]);

        // Malformed inventory disables controls, then a retry recovers without a name change.
        let invalid = vec![HashMap::from([("Driver", "tuned".to_variant())])].to_variant();
        inventory.replace(invalid.clone());
        changed(&daemon, "Profiles", invalid);
        wait(&context, || !service.state().available());
        inventory.replace(profiles(&["balanced"], "tuned"));
        wait(&context, || service.state().available());
        assert_eq!(service.state().profiles, ["balanced"]);

        allow.set(true);
        let recorded = result.clone();
        service.set_profile("balanced", move |reply| { recorded.replace(Some(reply)); });
        let weak = service.downgrade();
        drop(service);
        assert!(weak.upgrade().is_none());
        wait(&context, || result.borrow().is_some());
        assert!(result.borrow_mut().take().unwrap().unwrap_err().matches(gio::IOErrorEnum::Cancelled));
        for _ in 0..20 {
            let service = PowerProfilesService::on_connection(&client);
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
        }
        changed(&daemon, "ActiveProfile", "balanced".to_variant());
        context.block_on(glib::timeout_future(Duration::from_millis(20)));
        // A lost bus supplies a null connection to the name-vanished callback.
        let service = PowerProfilesService::on_connection(&client);
        wait(&context, || service.state().available());
        bus.0.kill().unwrap();
        bus.0.wait().unwrap();
        wait(&context, || !service.state().available());
        assert_eq!(service.state(), power_profiles::ProfilesState::default());
        let weak = service.downgrade();
        drop(service);
        assert!(weak.upgrade().is_none());
        daemon.unregister_object(registration).unwrap();
    }).unwrap();
}

#[test]
fn icon_contract() {
    assert_eq!(
        power_profiles::profile_icon(Some("performance")),
        "power-profile-performance-symbolic"
    );
    assert_eq!(
        power_profiles::profile_icon(Some("power-saver")),
        "power-profile-power-saver-symbolic"
    );
    assert_eq!(
        power_profiles::profile_icon(None),
        "power-profile-balanced-symbolic"
    );
    assert_eq!(
        power_profiles::profile_icon(Some("future-profile")),
        "power-profile-balanced-symbolic"
    );
}
