//! Exercise the system-bus constructor in a child so GIO's shared bus cache and
//! the process environment cannot affect other private D-Bus fixtures.
use gio::prelude::*;
use std::{process::Command, time::Duration};
use way_shell::services::{
    logind::LogindService, power::PowerService, power_profiles::PowerProfilesService,
};

fn remains_unavailable<T: IsA<glib::Object>>(
    context: &glib::MainContext,
    service: T,
    unavailable: impl Fn(&T) -> bool,
) {
    // Allow the asynchronous initial connection attempt and vanished callback.
    context.block_on(glib::timeout_future(Duration::from_millis(250)));
    assert!(unavailable(&service));
    let weak = service.downgrade();
    drop(service);
    assert!(weak.upgrade().is_none());
    context.block_on(glib::timeout_future(Duration::from_millis(20)));
}

#[test]
fn services_start_without_a_system_bus_and_release_watchers() {
    const CHILD: &str = "WAY_SHELL_TEST_UNAVAILABLE_BUS";
    if let Ok(service) = std::env::var(CHILD) {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| match service.as_str() {
                "power" => remains_unavailable(&context, PowerService::new(), |service| {
                    !service.state().available && service.devices().is_empty()
                }),
                "profiles" => {
                    remains_unavailable(&context, PowerProfilesService::new(), |service| {
                        service.state() == Default::default()
                    });
                }
                "logind" => {
                    remains_unavailable(&context, LogindService::new().unwrap(), |service| {
                        service.state() == Default::default()
                    });
                }
                _ => panic!("Unknown service fixture: {service}"),
            })
            .unwrap();
        return;
    }
    for service in ["power", "profiles", "logind"] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "services_start_without_a_system_bus_and_release_watchers",
                "--nocapture",
            ])
            .env(CHILD, service)
            // /dev/null is not a directory, so this can never reach a real bus.
            .env(
                "DBUS_SYSTEM_BUS_ADDRESS",
                "unix:path=/dev/null/way-shell-unavailable-bus",
            )
            .env("GSETTINGS_SCHEMA_DIR", env!("WAY_SHELL_TEST_SCHEMAS"))
            .env("GSETTINGS_BACKEND", "memory")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{service} failed with an unavailable bus: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
