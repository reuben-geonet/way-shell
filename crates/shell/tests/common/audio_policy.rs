//! A hardware-free WirePlumber policy instance for the private audio daemon.

use std::{
    io::{BufRead, BufReader},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

pub struct Policy {
    child: OwnedChild,
    bus: OwnedChild,
    directory: PathBuf,
}

struct OwnedChild(Child);
impl OwnedChild {
    fn stop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Policy {
    pub fn new(remote: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let remote = Path::new(remote);
        assert!(
            remote.is_absolute(),
            "policy needs the private daemon's absolute socket path"
        );
        let runtime = remote.parent().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "way-shell-policy-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let wireplumber = executable("wireplumber");
        let packaged_config = wireplumber
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("share/wireplumber/wireplumber.conf");
        let packaged_data = packaged_config.parent().unwrap();
        let mut config = std::fs::read_to_string(&packaged_config)
            .unwrap_or_else(|error| panic!("read {}: {error}", packaged_config.display()));
        // Use the version-matched installed policy and scripts, with neither
        // hardware monitors nor persistent user state. No /etc or user config
        // fragments are loaded because this is the sole configuration directory.
        config.push_str(
            r#"
wireplumber.profiles = {
    way-shell-test = {
        inherits = [ policy, mixin.systemwide-session, mixin.stateless ]
        hardware.audio = disabled
        hardware.bluetooth = disabled
        hardware.video-capture = disabled
    }
}
"#,
        );
        std::fs::write(directory.join("wireplumber.conf"), config).unwrap();
        let log = std::fs::File::create(directory.join("wireplumber.log")).unwrap();
        let bus_log = std::fs::File::create(directory.join("dbus.log")).unwrap();
        let mut bus = OwnedChild(
            Command::new("dbus-daemon")
                .args([
                    "--nofork",
                    "--print-address=1",
                    concat!(
                        "--config-file=",
                        env!("CARGO_MANIFEST_DIR"),
                        "/../../tests/fixtures/session-bus.conf"
                    ),
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(bus_log)
                .spawn()
                .expect("start private policy bus"),
        );
        let mut address = String::new();
        BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        assert!(
            !address.trim().is_empty(),
            "private policy bus returned no address: {}",
            std::fs::read_to_string(directory.join("dbus.log")).unwrap_or_default()
        );
        let child = OwnedChild(
            Command::new(wireplumber)
                .args(["--profile", "way-shell-test"])
                .env("WIREPLUMBER_CONFIG_DIR", &directory)
                .env("WIREPLUMBER_DATA_DIR", packaged_data)
                .env_remove("WIREPLUMBER_MODULE_DIR")
                .env_remove("LUA_PATH")
                .env_remove("LUA_CPATH")
                .env("PIPEWIRE_REMOTE", remote)
                .env("PIPEWIRE_RUNTIME_DIR", runtime)
                .env("XDG_RUNTIME_DIR", runtime)
                .env("XDG_CONFIG_HOME", directory.join("config"))
                .env("XDG_CONFIG_DIRS", directory.join("config-dirs"))
                .env("XDG_STATE_HOME", directory.join("state"))
                .env("XDG_CACHE_HOME", directory.join("cache"))
                .env("XDG_DATA_HOME", directory.join("data"))
                .env("XDG_DATA_DIRS", directory.join("data-dirs"))
                .env("DBUS_SESSION_BUS_ADDRESS", address.trim())
                .env(
                    "DBUS_SYSTEM_BUS_ADDRESS",
                    format!("unix:path={}/no-system-bus", directory.display()),
                )
                .env_remove("PIPEWIRE_CONFIG_DIR")
                .env_remove("PIPEWIRE_CONFIG_PREFIX")
                .env_remove("PIPEWIRE_CONFIG_NAME")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(log)
                .spawn()
                .expect("start private WirePlumber policy"),
        );
        Self {
            child,
            bus,
            directory,
        }
    }
}

fn executable(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| panic!("audio policy fixture needs {name} in PATH"))
        .canonicalize()
        .unwrap()
}

impl Drop for Policy {
    fn drop(&mut self) {
        self.child.stop();
        self.bus.stop();
        if std::thread::panicking() {
            eprintln!(
                "private WirePlumber policy log:\n{}",
                std::fs::read_to_string(self.directory.join("wireplumber.log")).unwrap_or_default()
            );
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
