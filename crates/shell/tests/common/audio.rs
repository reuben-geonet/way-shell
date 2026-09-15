use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
pub struct Daemon {
    child: Option<Child>,
    runtime: PathBuf,
}
impl Daemon {
    pub fn new() -> Self {
        let runtime =
            std::env::temp_dir().join(format!("way-shell-audio-inventory-{}", std::process::id()));
        std::fs::create_dir(&runtime).unwrap();
        let mut daemon = Self {
            child: None,
            runtime,
        };
        daemon.start();
        daemon
    }
    pub fn start(&mut self) {
        self.child = Some(
            Command::new("pipewire")
                .args([
                    "-c",
                    concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../../tests/fixtures/pipewire.conf"
                    ),
                ])
                .env("XDG_RUNTIME_DIR", &self.runtime)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.runtime.join("pipewire-0").exists() {
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "private PipeWire failed"
            );
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    pub fn remote(&self) -> String {
        self.runtime.join("pipewire-0").to_str().unwrap().to_owned()
    }
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.runtime);
    }
}
#[track_caller]
pub fn wait(context: &glib::MainContext, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(Instant::now() < deadline, "audio fixture timed out");
        context.block_on(glib::timeout_future(Duration::from_millis(10)));
    }
}
