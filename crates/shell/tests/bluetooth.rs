#[path = "common/bluetooth.rs"]
mod fixture;
use fixture::*;
use gio::prelude::*;
use std::{collections::HashMap, time::Duration};
fn scenario(f: impl FnOnce(&glib::MainContext, &Fixture)) {
    let context = glib::MainContext::default();
    context
        .with_thread_default(|| {
            let fixture = Fixture::new(&context);
            f(&context, &fixture);
        })
        .unwrap();
}
fn devices() {
    scenario(|c, f| {
        let devices = f.service.state().devices;
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].alias, "Zebra mouse");
        f.fake.set(MOUSE, DEVICE, "Connected", false);
        pump(c);
        assert_eq!(f.service.state().devices[0].alias, "Alpha headphones");
        f.fake.set(HEADPHONES, DEVICE, "Trusted", false);
        pump(c);
        assert_eq!(f.service.state().devices.len(), 1);
    });
}
fn connection() {
    scenario(|c, f| {
        f.service.toggle_device(MOUSE);
        f.service.toggle_device(MOUSE);
        pump(c);
        assert_eq!(f.fake.state.calls.borrow().len(), 1);
        assert!(!f.fake.value(MOUSE, "Connected"));
        f.service.toggle_device(MOUSE);
        pump(c);
        assert!(f.fake.value(MOUSE, "Connected"));
        f.fake
            .state
            .device_error
            .replace(Some("org.bluez.Error.Failed".into()));
        f.service.toggle_device(MOUSE);
        pump(c);
        assert_eq!(f.errors.borrow().len(), 1);
        assert!(f.service.state().devices.iter().all(|d| !d.busy));
    });
}
fn airplane() {
    scenario(|c, f| {
        f.service.set_airplane_mode(true);
        f.settle(c);
        assert!(!f.fake.value(HCI, "Powered"));
        f.service.set_airplane_mode(true);
        f.service.set_airplane_mode(false);
        f.settle(c);
        assert!(f.fake.value(HCI, "Powered"));
        f.service.set_powered(false);
        f.settle(c);
        f.service.set_airplane_mode(true);
        f.settle(c);
        f.service.set_airplane_mode(false);
        f.settle(c);
        assert!(!f.fake.value(HCI, "Powered"));
        f.service.set_airplane_mode(true);
        f.settle(c);
        f.service.set_powered(true);
        f.settle(c);
        f.service.set_airplane_mode(false);
        f.settle(c);
        assert!(f.fake.value(HCI, "Powered"));
    });
}
fn external_override() {
    scenario(|c, f| {
        f.service.set_airplane_mode(true);
        f.settle(c);
        f.fake.set(HCI, ADAPTER, "Powered", true);
        pump(c);
        f.fake.set(HCI, ADAPTER, "Powered", false);
        pump(c);
        f.service.set_airplane_mode(false);
        f.settle(c);
        assert!(!f.fake.value(HCI, "Powered"));
    });
}
fn power_cycles() {
    scenario(|c, f| {
        for _ in 0..20 {
            for target in [false, true] {
                f.service.set_powered(target);
                f.settle(c);
                assert_eq!(f.service.state().powered, target);
            }
        }
        f.service.set_powered(false);
        f.service.set_powered(true);
        f.settle(c);
        assert!(f.service.state().powered);
        f.fake.state.hold_power.set(true);
        f.service.set_powered(false);
        wait(c, || !f.fake.state.held.borrow().is_empty());
        f.service.set_powered(true);
        assert!(f.service.state().target_powered);
        f.fake.state.hold_power.set(false);
        f.fake.finish(None, true);
        f.settle(c);
        assert!(f.service.state().powered);
        assert!(f.errors.borrow().is_empty());
        assert!(f.read_radio().is_empty());
    });
}
fn transient_case(error: &str) {
    scenario(|c, f| {
        f.fake
            .state
            .power_errors
            .replace([error.into(), error.into()].into());
        f.service.set_powered(false);
        f.settle(c);
        assert_eq!(f.fake.power_calls(), 3);
        assert!(!f.service.state().powered);
        assert!(f.errors.borrow().is_empty());
    });
}
fn power_busy() {
    transient_case("org.bluez.Error.Busy");
}
fn power_blocked() {
    transient_case("org.bluez.Error.Blocked");
}
fn other_transient_errors() {
    for error in [
        "org.bluez.Error.InProgress",
        "org.bluez.Error.NotReady",
        "org.bluez.Error.Failed",
    ] {
        transient_case(error);
    }
}
fn delayed_adapter() {
    scenario(|c, f| {
        let mut adapter = f.fake.remove(HCI);
        radio(&f.radio, 4, 2, true, false);
        pump(c);
        f.service.set_powered(true);
        pump(c);
        assert!(f.service.state().busy);
        assert_eq!(f.read_radio()[0][6], 0);
        radio(&f.radio, 4, 2, false, false);
        adapter
            .get_mut(ADAPTER)
            .unwrap()
            .insert("Powered".into(), false.to_variant());
        f.fake.add(HCI, adapter);
        f.settle(c);
        assert!(f.service.state().powered);
        assert!(f.errors.borrow().is_empty());
    });
}
fn radio_blocks() {
    scenario(|c, f| {
        radio(&f.radio, 4, 2, false, true);
        pump(c);
        assert!(f.service.state().hardware_blocked);
        f.service.set_powered(true);
        assert_eq!(f.errors.borrow().len(), 1);
        radio(&f.radio, 4, 2, false, false);
        pump(c);
        f.service.set_airplane_mode(true);
        f.settle(c);
        let writes = f.read_radio();
        assert_eq!(&writes[0][4..], [2, 3, 1, 0]);
    });
}
fn owner_and_removal() {
    scenario(|c, f| {
        f.fake.state.hold_device.set(true);
        f.service.toggle_device(MOUSE);
        pump(c);
        f.fake.remove(MOUSE);
        pump(c);
        assert_eq!(f.service.state().devices.len(), 1);
        f.fake.ownership(false);
        wait(c, || !f.service.state().ready);
        assert!(f.service.state().devices.is_empty());
        f.fake.ownership(true);
        wait(c, || f.service.state().ready);
        assert_eq!(f.service.state().devices.len(), 1);
    });
}
fn removed_operation() {
    scenario(|c, f| {
        f.fake.state.hold_device.set(true);
        f.service.toggle_device(MOUSE);
        pump(c);
        f.fake.remove(MOUSE);
        pump(c);
        f.fake
            .finish(Some("org.freedesktop.DBus.Error.UnknownMethod"), false);
        pump(c);
        assert!(f.errors.borrow().is_empty());
    });
}
fn completed_operation_error() {
    scenario(|c, f| {
        f.fake.state.hold_device.set(true);
        f.service.toggle_device(MOUSE);
        pump(c);
        f.fake.set(MOUSE, DEVICE, "Connected", false);
        pump(c);
        f.fake
            .finish(Some("org.freedesktop.DBus.Error.NoReply"), false);
        pump(c);
        assert!(f.errors.borrow().is_empty());
        assert!(f.service.state().devices.iter().all(|d| !d.busy));
    });
}
fn late_connection(target: bool) {
    scenario(|c, f| {
        f.fake.set(MOUSE, DEVICE, "Connected", !target);
        pump(c);
        f.fake.state.hold_device.set(true);
        f.service.toggle_device(MOUSE);
        pump(c);
        f.fake
            .finish(Some("org.freedesktop.DBus.Error.NoReply"), false);
        pump(c);
        assert_eq!(f.errors.borrow().len(), 1);
        assert_eq!(f.successes.get(), 0);
        f.fake.set(HEADPHONES, DEVICE, "Connected", true);
        f.fake.set(MOUSE, DEVICE, "Connected", !target);
        pump(c);
        assert_eq!(f.successes.get(), 0);
        f.fake.set(MOUSE, DEVICE, "Connected", target);
        pump(c);
        assert_eq!(f.successes.get(), 1);
        f.fake.set(MOUSE, DEVICE, "Connected", target);
        pump(c);
        assert_eq!(f.successes.get(), 1);
    });
}
fn late_connect() {
    late_connection(true);
}
fn late_disconnect() {
    late_connection(false);
}
fn external_power() {
    scenario(|c, f| {
        f.fake.set(HCI, ADAPTER, "Powered", false);
        pump(c);
        assert!(!f.service.state().powered);
        f.fake.set(HCI, ADAPTER, "Powered", true);
        pump(c);
        assert!(f.service.state().powered);
        radio(&f.radio, 4, 2, true, false);
        pump(c);
        assert!(!f.service.state().powered);
        assert!(f.service.state().devices.is_empty());
        radio(&f.radio, 4, 2, false, false);
        pump(c);
        assert!(f.service.state().powered);
        assert_eq!(f.fake.power_calls(), 0);
    });
}
fn power_timeout() {
    scenario(|c, f| {
        f.fake.remove(HCI);
        pump(c);
        f.service.set_powered(true);
        assert!(f.service.state().busy);
        f.settle(c);
        assert_eq!(f.errors.borrow().len(), 1);
        f.service.set_powered(false);
        f.settle(c);
        assert_eq!(f.errors.borrow().len(), 1);
    });
}
fn airplane_reversal() {
    scenario(|c, f| {
        f.fake.state.hold_power.set(true);
        f.service.set_airplane_mode(true);
        wait(c, || !f.fake.state.held.borrow().is_empty());
        f.service.set_airplane_mode(false);
        f.fake.state.hold_power.set(false);
        f.fake.finish(None, true);
        f.settle(c);
        assert!(f.service.state().powered);
        assert!(f.errors.borrow().is_empty());
    });
}
fn power_off(external: bool) {
    scenario(|c, f| {
        f.fake.set(MOUSE, DEVICE, "Connected", false);
        pump(c);
        f.fake.state.hold_device.set(true);
        f.service.toggle_device(MOUSE);
        pump(c);
        if external {
            f.fake.set(HCI, ADAPTER, "Powered", false);
        } else {
            f.service.set_powered(false);
        }
        f.settle(c);
        f.fake.finish(Some("org.bluez.Error.Failed"), false);
        pump(c);
        assert!(f.errors.borrow().is_empty());
        assert!(!f.service.state().powered);
    });
}
fn power_off_cancels_connection() {
    power_off(false);
}
fn external_off_cancels_connection() {
    power_off(true);
}
fn settings() {
    use way_shell::services::bluetooth::settings::*;
    let argv = parse_command("true 'a b' '$(false)'").unwrap();
    assert_eq!(argv[1], "a b");
    assert_eq!(argv[2], "$(false)");
    assert!(
        parse_command("no-such-bluetooth-manager-92841")
            .unwrap_err()
            .matches(gio::IOErrorEnum::NotFound)
    );
    assert!(parse_command("'unterminated").is_err());
    assert!(parse_command("").is_err());
    launch("true").unwrap();
    let source =
        gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)
            .unwrap();
    let settings = gio::Settings::new_full(
        &source
            .lookup("org.ldelossa.way-shell.system", false)
            .unwrap(),
        Some(&gio::memory_settings_backend_new()),
        None,
    );
    assert_eq!(
        settings.string("bluetooth-settings-command"),
        "blueman-manager"
    );
}
fn connected_without_profiles() {
    scenario(|c, f| {
        f.fake.set(MOUSE, DEVICE, "UUIDs", Vec::<String>::new());
        pump(c);
        assert_eq!(f.service.state().devices.len(), 2);
        assert_eq!(f.service.state().devices[0].path, MOUSE);
    });
}
fn adapter_hotplug() {
    scenario(|c, f| {
        let adapter = f.fake.remove(HCI);
        radio(&f.radio, 4, 1, false, false);
        pump(c);
        assert!(!f.service.state().available);
        assert!(f.service.state().devices.is_empty());
        f.fake.add(HCI, adapter);
        pump(c);
        assert!(f.service.state().available);
        assert_eq!(f.service.state().devices.len(), 2);
    });
}
fn replaced_object_ignores_old_reply_and_recovery() {
    scenario(|c, f| {
        f.fake.state.hold_device.set(true);
        f.service.toggle_device(MOUSE);
        pump(c);
        let device = f.fake.remove(MOUSE);
        pump(c);
        f.fake.add(MOUSE, device);
        pump(c);
        f.fake.finish(Some("org.bluez.Error.Failed"), false);
        pump(c);
        assert!(f.errors.borrow().is_empty());
        f.service.toggle_device(MOUSE);
        pump(c);
        f.fake.finish(Some("org.bluez.Error.Failed"), false);
        pump(c);
        assert_eq!(f.errors.borrow().len(), 1);
        let device = f.fake.remove(MOUSE);
        pump(c);
        f.fake.add(MOUSE, device);
        pump(c);
        f.fake.set(MOUSE, DEVICE, "Connected", false);
        pump(c);
        assert_eq!(f.successes.get(), 0);
    });
}
fn mixed_adapter_restore() {
    scenario(|c, f| {
        const HCI2: &str = "/org/bluez/hci1";
        f.fake.add(
            HCI2,
            HashMap::from([(
                ADAPTER.into(),
                HashMap::from([("Powered".into(), false.to_variant())]),
            )]),
        );
        radio(&f.radio, 5, 0, true, false);
        pump(c);
        f.service.set_airplane_mode(true);
        f.settle(c);
        f.read_radio();
        f.service.set_airplane_mode(false);
        f.settle(c);
        assert!(f.fake.value(HCI, "Powered"));
        assert!(!f.fake.value(HCI2, "Powered"));
        let writes = f.read_radio();
        assert_eq!(writes.len(), 3);
        assert_eq!(writes[0][5], 3);
        assert_eq!(writes[0][6], 0);
        assert!(
            writes
                .iter()
                .any(|w| u32::from_ne_bytes(w[..4].try_into().unwrap()) == 5 && w[6] == 1)
        );
    });
}
fn retry_exhaustion_recovers() {
    scenario(|c, f| {
        f.fake
            .state
            .power_errors
            .replace(std::iter::repeat_n("org.bluez.Error.Busy".into(), 50).collect());
        f.service.set_powered(false);
        f.settle(c);
        assert_eq!(f.errors.borrow().len(), 1);
        assert!(!f.service.state().busy);
        f.fake.state.power_errors.borrow_mut().clear();
        f.service.set_powered(false);
        f.settle(c);
        assert!(!f.service.state().powered);
    });
}
fn stop_cancels_pending_and_releases_service() {
    scenario(|c, f| {
        f.fake.state.hold_device.set(true);
        f.service.toggle_device(MOUSE);
        pump(c);
        f.service.stop();
        f.fake.finish(Some("org.bluez.Error.Failed"), false);
        pump(c);
        assert!(f.errors.borrow().is_empty());
        assert!(!f.service.state().available);
        let service = way_shell::services::bluetooth::BluetoothService::on_connection(
            &f.fake.connection,
            None,
        );
        let weak = service.downgrade();
        service.stop();
        drop(service);
        pump(c);
        assert!(weak.upgrade().is_none());
    });
}
fn rfkill_write_failure() {
    scenario(|c, f| {
        let (fd, peer) = std::os::unix::net::UnixDatagram::pair().unwrap();
        fd.set_nonblocking(true).unwrap();
        peer.set_nonblocking(true).unwrap();
        radio(&peer, 9, 0, true, false);
        fd.shutdown(std::net::Shutdown::Write).unwrap();
        let service = way_shell::services::bluetooth::BluetoothService::on_connection(
            &f.fake.connection,
            Some(fd.into()),
        );
        let errors = std::rc::Rc::new(std::cell::Cell::new(0));
        let seen = errors.clone();
        service.connect_local("operation-error", false, move |_| {
            seen.set(seen.get() + 1);
            None
        });
        wait(c, || service.state().ready);
        service.set_powered(true);
        pump(c);
        assert_eq!(errors.get(), 1);
        assert!(!service.state().busy);
        service.stop();
        let service = way_shell::services::bluetooth::BluetoothService::on_connection(
            &f.fake.connection,
            None,
        );
        wait(c, || service.state().ready);
        service.set_powered(false);
        wait(c, || !service.state().busy);
        assert!(!service.state().powered);
        service.stop();
    });
}
fn radio_block_cancels_device() {
    scenario(|c, f| {
        f.fake.state.hold_device.set(true);
        f.service.toggle_device(MOUSE);
        pump(c);
        radio(&f.radio, 4, 2, true, false);
        pump(c);
        f.fake.finish(Some("org.bluez.Error.Failed"), false);
        pump(c);
        assert!(f.errors.borrow().is_empty());
    });
}
fn stopping_during_retry_prevents_more_calls() {
    scenario(|c, f| {
        f.fake
            .state
            .power_errors
            .replace(["org.bluez.Error.Busy".into()].into());
        f.service.set_powered(false);
        wait(c, || f.fake.power_calls() == 1);
        f.service.stop();
        c.block_on(glib::timeout_future(Duration::from_millis(150)));
        assert_eq!(f.fake.power_calls(), 1);
    });
}

fn airplane_before_bluez_initializes() {
    scenario(|c, f| {
        let service = way_shell::services::bluetooth::BluetoothService::on_connection(
            &f.fake.connection,
            None,
        );
        service.set_airplane_mode(true);
        wait(c, || {
            service.state().ready && !service.state().powered && !service.state().busy
        });
        service.set_airplane_mode(false);
        wait(c, || service.state().powered && !service.state().busy);
        service.stop();
    });
}
fn callback_shutdown_is_safe() {
    scenario(|c, f| {
        let service = f.service.downgrade();
        f.service.connect_local("operation-error", false, move |_| {
            if let Some(service) = service.upgrade() {
                service.stop();
            }
            None
        });
        f.fake
            .state
            .device_error
            .replace(Some("org.bluez.Error.Failed".into()));
        f.service.toggle_device(MOUSE);
        pump(c);
        assert!(!f.service.state().ready);
    });
}
// GDBusObjectManagerClient's asynchronous initializer runs GInitable in a
// worker without a thread-default context. Its inventory signals therefore
// use the default context, just as the real shell does. Keep every case on
// this one thread; each still receives a fresh private bus and radio.
#[test]
fn bluetooth_reference_and_lifecycle_contract() {
    for (name, run) in [
        (
            "airplane_before_bluez_initializes",
            airplane_before_bluez_initializes as fn(),
        ),
        (
            "callback_shutdown_is_safe",
            callback_shutdown_is_safe as fn(),
        ),
        ("devices", devices as fn()),
        ("connection", connection as fn()),
        ("airplane", airplane as fn()),
        ("external_override", external_override as fn()),
        ("power_cycles", power_cycles as fn()),
        ("power_busy", power_busy as fn()),
        ("power_blocked", power_blocked as fn()),
        ("other_transient_errors", other_transient_errors as fn()),
        ("delayed_adapter", delayed_adapter as fn()),
        ("radio_blocks", radio_blocks as fn()),
        ("owner_and_removal", owner_and_removal as fn()),
        ("removed_operation", removed_operation as fn()),
        (
            "completed_operation_error",
            completed_operation_error as fn(),
        ),
        ("late_connect", late_connect as fn()),
        ("late_disconnect", late_disconnect as fn()),
        ("external_power", external_power as fn()),
        ("power_timeout", power_timeout as fn()),
        ("airplane_reversal", airplane_reversal as fn()),
        (
            "power_off_cancels_connection",
            power_off_cancels_connection as fn(),
        ),
        (
            "external_off_cancels_connection",
            external_off_cancels_connection as fn(),
        ),
        ("settings", settings as fn()),
        (
            "connected_without_profiles",
            connected_without_profiles as fn(),
        ),
        ("adapter_hotplug", adapter_hotplug as fn()),
        (
            "replaced_object_ignores_old_reply_and_recovery",
            replaced_object_ignores_old_reply_and_recovery as fn(),
        ),
        ("mixed_adapter_restore", mixed_adapter_restore as fn()),
        (
            "retry_exhaustion_recovers",
            retry_exhaustion_recovers as fn(),
        ),
        (
            "stop_cancels_pending_and_releases_service",
            stop_cancels_pending_and_releases_service as fn(),
        ),
        ("rfkill_write_failure", rfkill_write_failure as fn()),
        (
            "radio_block_cancels_device",
            radio_block_cancels_device as fn(),
        ),
        (
            "stopping_during_retry_prevents_more_calls",
            stopping_during_retry_prevents_more_calls as fn(),
        ),
    ] {
        eprintln!("Bluetooth scenario: {name}");
        run();
        pump(&glib::MainContext::default());
    }
}
