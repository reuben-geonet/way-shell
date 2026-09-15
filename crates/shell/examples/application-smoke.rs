//! Exercise supplied packaged executables in the private application-smoke harness.
use gtk::prelude::*;
use std::{
    cell::Cell,
    collections::HashMap,
    fs::{self, File},
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Output, Stdio},
    rc::Rc,
    time::{Duration, Instant},
};
const APP_ID: &str = "org.ldelossa.way-shell";
const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const TRAY: &str = "org.kde.StatusNotifierWatcher";

#[derive(Default)]
struct Surface {
    namespace: String,
    buffer: bool,
    mapped: bool,
}
#[derive(Default)]
struct Trace {
    mapped: HashMap<String, usize>,
}
fn surface_id(text: &str) -> Option<u32> {
    let tail = text.split_once("wl_surface")?.1;
    let tail = tail.strip_prefix('@').or_else(|| tail.strip_prefix('#'))?;
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}
impl Trace {
    fn parse(text: &str) -> Self {
        let mut result = Self::default();
        let mut surfaces = HashMap::<u32, Surface>::new();
        for line in text.lines() {
            if line.contains(".get_layer_surface(") {
                if let Some(id) = surface_id(line)
                    && let Some(namespace) = line.split('"').nth(1)
                {
                    surfaces.insert(
                        id,
                        Surface {
                            namespace: namespace.to_owned(),
                            ..Surface::default()
                        },
                    );
                }
                continue;
            }
            let Some(id) = surface_id(line) else {
                continue;
            };
            if line.contains(".destroy(") {
                surfaces.remove(&id);
                continue;
            }
            let Some(surface) = surfaces.get_mut(&id) else {
                continue;
            };
            if line.contains(".attach(") {
                surface.buffer = line.contains("wl_buffer");
            }
            if line.contains(".commit(") {
                if surface.buffer && !surface.mapped {
                    *result.mapped.entry(surface.namespace.clone()).or_default() += 1;
                }
                surface.mapped = surface.buffer;
            }
        }
        result
    }
    fn count(&self, namespace: &str) -> usize {
        self.mapped.get(namespace).copied().unwrap_or(0)
    }
}

struct Running {
    child: Option<Child>,
}
impl Running {
    fn spawn(command: &mut Command) -> Self {
        Self {
            child: Some(command.spawn().expect("spawn packaged executable")),
        }
    }
    fn status(&mut self) -> Option<ExitStatus> {
        self.child.as_mut().unwrap().try_wait().unwrap()
    }
    fn output(mut self, context: &glib::MainContext, deadline: Duration) -> Output {
        let end = Instant::now() + deadline;
        loop {
            if self.status().is_some() {
                return self.child.take().unwrap().wait_with_output().unwrap();
            }
            assert!(
                Instant::now() < end,
                "packaged command did not exit before its deadline"
            );
            pause(context);
        }
    }
    fn terminate(self, context: &glib::MainContext) -> ExitStatus {
        let pid = self.child.as_ref().unwrap().id().to_string();
        // POSIX shell supplies kill even when no separate kill executable is installed.
        let status = Command::new("sh")
            .args(["-c", "kill -TERM \"$1\"", "way-shell-smoke", &pid])
            .status()
            .unwrap();
        assert!(status.success());
        self.output(context, Duration::from_secs(8)).status
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
fn pause(context: &glib::MainContext) {
    context.block_on(glib::timeout_future(Duration::from_millis(10)));
}
fn wait(context: &glib::MainContext, description: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !done() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        pause(context);
    }
}
fn owner(connection: &gio::DBusConnection, name: &str) -> Option<String> {
    connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            Some(&(name,).to_variant()),
            Some(glib::VariantTy::new("(s)").unwrap()),
            gio::DBusCallFlags::NONE,
            1000,
            gio::Cancellable::NONE,
        )
        .ok()
        .and_then(|reply| reply.get::<(String,)>().map(|value| value.0))
}
struct Fixture {
    application: PathBuf,
    cli: PathBuf,
    runtime: PathBuf,
    logs: PathBuf,
    context: glib::MainContext,
}
impl Fixture {
    fn new() -> Self {
        let path = |name| {
            PathBuf::from(
                std::env::var_os(name)
                    .unwrap_or_else(|| panic!("{name} must be supplied by application-smoke.sh")),
            )
        };
        let this = Self {
            application: path("WAY_SHELL_TEST_APPLICATION"),
            cli: path("WAY_SHELL_TEST_CLI"),
            runtime: path("WAY_SHELL_TEST_PRIVATE_RUNTIME"),
            logs: path("WAY_SHELL_TEST_LOG_DIR"),
            context: glib::MainContext::default(),
        };
        assert_eq!(path("XDG_RUNTIME_DIR"), this.runtime);
        for name in [
            "XDG_CONFIG_HOME",
            "XDG_CACHE_HOME",
            "XDG_DATA_HOME",
            "PIPEWIRE_REMOTE",
        ] {
            assert!(
                path(name).starts_with(&this.runtime),
                "{name} must be isolated"
            );
        }
        for name in ["DBUS_SESSION_BUS_ADDRESS", "DBUS_SYSTEM_BUS_ADDRESS"] {
            assert!(
                std::env::var(name)
                    .unwrap()
                    .contains(this.runtime.to_str().unwrap()),
                "{name} must address the private bus"
            );
        }
        assert_eq!(std::env::var("GSETTINGS_BACKEND").unwrap(), "memory");
        assert!(this.application.is_absolute() && this.application.is_file());
        assert!(this.cli.is_absolute() && this.cli.is_file());
        this
    }
    fn socket(&self) -> PathBuf {
        self.runtime.join("way-shell.sock")
    }
    fn trace(&self, label: &str) -> Trace {
        Trace::parse(
            &fs::read_to_string(self.logs.join(format!("application-{label}.log")))
                .unwrap_or_default(),
        )
    }
    fn start(&self, label: &str, overrides: &[(&str, &str)]) -> Running {
        let mut command = Command::new(&self.application);
        command
            .stdout(Stdio::null())
            .stderr(File::create(self.logs.join(format!("application-{label}.log"))).unwrap())
            .env("WAYLAND_DEBUG", "client");
        for (name, value) in overrides {
            command.env(name, value);
        }
        Running::spawn(&mut command)
    }
    fn cli(&self, args: &[&str]) {
        let output = Running::spawn(
            Command::new(&self.cli)
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .output(&self.context, Duration::from_secs(4));
        assert!(
            output.status.success(),
            "way-sh {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn ready(&self, process: &mut Running, connection: &gio::DBusConnection, label: &str) {
        wait(
            &self.context,
            "packaged shell, IPC, D-Bus services and mapped panel",
            || {
                assert!(
                    process.status().is_none(),
                    "packaged application exited; see application-{label}.log"
                );
                self.socket()
                    .metadata()
                    .is_ok_and(|metadata| metadata.file_type().is_socket())
                    && [APP_ID, NOTIFICATIONS, TRAY]
                        .iter()
                        .all(|name| owner(connection, name).is_some())
                    && self.trace(label).count("way-shell-panel") > 0
            },
        );
    }
    fn mapped_command(&self, process: &mut Running, label: &str, args: &[&str], namespace: &str) {
        let before = self.trace(label).count(namespace);
        self.cli(args);
        wait(
            &self.context,
            &format!("{args:?} attaching and committing a layer-surface buffer"),
            || {
                assert!(
                    process.status().is_none(),
                    "application exited after {args:?}"
                );
                self.trace(label).count(namespace) > before
            },
        );
    }
    fn stopped(&self, process: Running, connection: &gio::DBusConnection) {
        assert!(
            process.terminate(&self.context).success(),
            "SIGTERM shutdown failed"
        );
        assert!(
            !self.socket().exists(),
            "shutdown must remove its owned IPC socket"
        );
        wait(&self.context, "D-Bus name release", || {
            [APP_ID, NOTIFICATIONS, TRAY]
                .iter()
                .all(|name| owner(connection, name).is_none())
        });
    }
}
fn help(fixture: &Fixture, executable: &Path) {
    let output = Running::spawn(
        Command::new(executable)
            .arg("--help")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}/no-session", fixture.runtime.display()),
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .output(&fixture.context, Duration::from_secs(4));
    assert!(
        output.status.success(),
        "help without a display/session failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.stdout.is_empty());
}
fn notification(fixture: &Fixture, process: &mut Running, connection: &gio::DBusConnection) {
    let closed = Rc::new(Cell::new(None));
    let seen = closed.clone();
    let _subscription = connection.subscribe_to_signal(
        Some(NOTIFICATIONS),
        Some(NOTIFICATIONS),
        Some("NotificationClosed"),
        Some("/org/freedesktop/Notifications"),
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| {
            seen.set(signal.parameters.get::<(u32, u32)>());
        },
    );
    let reply = connection
        .call_sync(
            Some(NOTIFICATIONS),
            "/org/freedesktop/Notifications",
            NOTIFICATIONS,
            "Notify",
            Some(
                &(
                    "Way Shell package test",
                    0u32,
                    "dialog-information-symbolic",
                    "Packaged notification",
                    "Private-session smoke coverage",
                    Vec::<String>::new(),
                    HashMap::<String, glib::Variant>::new(),
                    0i32,
                )
                    .to_variant(),
            ),
            Some(glib::VariantTy::new("(u)").unwrap()),
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        )
        .unwrap();
    let id = reply.get::<(u32,)>().unwrap().0;
    assert_ne!(id, 0);
    wait(
        &fixture.context,
        "a real notification OSD layer-surface buffer",
        || {
            assert!(process.status().is_none());
            fixture.trace("first").count("way-shell-notifications-osd") > 0
        },
    );
    connection
        .call_sync(
            Some(NOTIFICATIONS),
            "/org/freedesktop/Notifications",
            NOTIFICATIONS,
            "CloseNotification",
            Some(&(id,).to_variant()),
            Some(glib::VariantTy::UNIT),
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        )
        .unwrap();
    wait(&fixture.context, "notification close signal", || {
        closed.get() == Some((id, 3))
    });
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new();
    help(&fixture, &fixture.cli);
    help(&fixture, &fixture.application);
    assert!(!fixture.socket().exists());
    let bad = fixture
        .start(
            "missing-display",
            &[("WAYLAND_DISPLAY", "wayland-does-not-exist")],
        )
        .output(&fixture.context, Duration::from_secs(8));
    assert!(
        !bad.status.success(),
        "missing Wayland must be a startup failure"
    );
    assert!(!fixture.socket().exists());
    let bad = fixture
        .start(
            "missing-compositor",
            &[
                ("SWAYSOCK", "/way-shell-missing-sway"),
                ("NIRI_SOCKET", "/way-shell-missing-niri"),
            ],
        )
        .output(&fixture.context, Duration::from_secs(8));
    assert!(
        !bad.status.success(),
        "missing compositor IPC must be a startup failure"
    );
    assert!(!fixture.socket().exists());
    gtk::init()?;
    let window = gtk::Window::builder()
        .title("Way Shell packaged smoke client")
        .default_width(300)
        .default_height(100)
        .child(&gtk::Label::new(Some("Private compositor smoke fixture")))
        .build();
    window.present();
    wait(&fixture.context, "fixture window", || window.is_mapped());
    let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
    connection.set_exit_on_close(false);
    let mut application = fixture.start("first", &[]);
    fixture.ready(&mut application, &connection, "first");
    let first_owner = owner(&connection, APP_ID).unwrap();
    let first_socket = fixture.socket().metadata()?;
    let duplicate = fixture
        .start("duplicate", &[])
        .output(&fixture.context, Duration::from_secs(4));
    assert!(
        duplicate.status.success(),
        "GApplication activation should succeed"
    );
    assert!(application.status().is_none());
    assert_eq!(owner(&connection, APP_ID).as_ref(), Some(&first_owner));
    let same_socket = fixture.socket().metadata()?;
    assert_eq!(
        (first_socket.dev(), first_socket.ino()),
        (same_socket.dev(), same_socket.ino())
    );
    notification(&fixture, &mut application, &connection);
    fixture.mapped_command(
        &mut application,
        "first",
        &["activities", "show"],
        "way-shell-activities",
    );
    fixture.cli(&["activities", "hide"]);
    for group in [
        "app-switcher",
        "workspace-switcher",
        "workspace-app-switcher",
        "output-switcher",
        "rename-switcher",
    ] {
        fixture.mapped_command(
            &mut application,
            "first",
            &[group, "show"],
            "way-shell-switcher",
        );
        fixture.cli(&[group, "hide"]);
    }
    fixture.cli(&["theme", "light"]);
    fixture.cli(&["theme", "dark"]);
    fixture.cli(&["theme", "dump-light"]);
    fixture.cli(&["theme", "dump-dark"]);
    for filename in ["way-shell-light.css", "way-shell-dark.css"] {
        assert!(!fs::read(fixture.runtime.join("config/way-shell").join(filename))?.is_empty());
    }
    fixture.mapped_command(
        &mut application,
        "first",
        &["message-tray", "open"],
        "way-shell-message-tray",
    );
    wait(&fixture.context, "message tray underlay buffer", || {
        fixture
            .trace("first")
            .count("way-shell-message-tray-underlay")
            > 0
    });
    fixture.stopped(application, &connection);
    let mut restarted = fixture.start("restart", &[]);
    fixture.ready(&mut restarted, &connection, "restart");
    assert_ne!(owner(&connection, APP_ID).as_ref(), Some(&first_owner));
    fixture.mapped_command(
        &mut restarted,
        "restart",
        &["activities", "show"],
        "way-shell-activities",
    );
    fixture.cli(&["activities", "hide"]);
    fixture.stopped(restarted, &connection);
    window.close();
    println!(
        "Application smoke passed: help, required startup failures, panel/windows with attached Wayland buffers, IPC, notifications/tray, duplicate activation, SIGTERM cleanup, and restart."
    );
    println!(
        "Executable: {}\nCLI: {}\nLogs: {}",
        fixture.application.display(),
        fixture.cli.display(),
        fixture.logs.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapped_evidence_requires_a_layer_surface_with_a_committed_buffer() {
        let setup = "[1] -> zwlr_layer_shell_v1#3.get_layer_surface(new id zwlr_layer_surface_v1#11, wl_surface#7, nil, 2, \"way-shell-panel\")\n[2] -> wl_surface#7.commit()\n";
        assert_eq!(Trace::parse(setup).count("way-shell-panel"), 0);
        let attached = format!("{setup}[3] -> wl_surface#7.attach(wl_buffer#20, 0, 0)\n");
        assert_eq!(Trace::parse(&attached).count("way-shell-panel"), 0);
        let mapped =
            format!("{attached}[4] -> wl_surface#7.commit()\n[5] -> wl_surface#7.commit()\n");
        assert_eq!(Trace::parse(&mapped).count("way-shell-panel"), 1);
        assert_eq!(
            Trace::parse("wl_surface#77.attach(wl_buffer#90,0,0)\nwl_surface#77.commit()\n")
                .count("way-shell-panel"),
            0
        );
        let remap = format!(
            "{mapped}wl_surface#7.attach(nil,0,0)\nwl_surface#7.commit()\nwl_surface#7.attach(wl_buffer#21,0,0)\nwl_surface#7.commit()\n"
        );
        assert_eq!(Trace::parse(&remap).count("way-shell-panel"), 2);
    }
    #[test]
    fn reused_object_ids_and_older_trace_format_keep_namespaces_separate() {
        let text = "zwlr_layer_shell_v1@3.get_layer_surface(new id zwlr_layer_surface_v1@11, wl_surface@7, nil, 2, \"way-shell-panel\")\nwl_surface@7.attach(wl_buffer@20,0,0)\nwl_surface@7.commit()\nwl_surface@7.destroy()\nzwlr_layer_shell_v1@3.get_layer_surface(new id zwlr_layer_surface_v1@12, wl_surface@7, nil, 2, \"way-shell-activities\")\nwl_surface@7.commit()\nwl_surface@7.attach(wl_buffer@22,0,0)\nwl_surface@7.commit()\n";
        let trace = Trace::parse(text);
        assert_eq!(trace.count("way-shell-panel"), 1);
        assert_eq!(trace.count("way-shell-activities"), 1);
    }
}
