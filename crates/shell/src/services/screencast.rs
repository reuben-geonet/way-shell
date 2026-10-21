//! A recording outlives its picker and panels. Only wf-recorder handles media.
mod worker;

use super::{notifications::NotificationsService, wm::WindowManager};
use gio::prelude::*;
use glib::subclass::prelude::*;
use std::{
    cell::{Cell, OnceCell, RefCell},
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use way_shell_core::{
    capture::{CaptureLayout, CaptureOutput, Mode, Rect, Selection},
    notifications::NotificationRequest,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Starting,
    Recording,
    Stopping,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Audio {
    #[default]
    Off,
    Microphone,
    Desktop,
}

#[derive(Clone, Debug)]
pub struct Target {
    output: CaptureOutput,
    region: Option<Rect>,
}
impl Target {
    pub fn from_selection(layout: &CaptureLayout, selection: &Selection) -> Result<Self, String> {
        match selection.mode {
            Mode::Screen => Ok(Self {
                output: layout
                    .outputs
                    .get(selection.output)
                    .ok_or("Select a screen")?
                    .clone(),
                region: None,
            }),
            Mode::Area => {
                let area = selection.area;
                let output = layout
                    .outputs
                    .iter()
                    .find(|output| {
                        area.valid()
                            && area.x >= output.bounds.x
                            && area.y >= output.bounds.y
                            && area.right() <= output.bounds.right()
                            && area.bottom() <= output.bounds.bottom()
                    })
                    .ok_or("Choose a recording area entirely inside one screen")?;
                // Round inward: wf-recorder otherwise falls back to the entire
                // output when given a region outside its logical bounds.
                let region = Rect {
                    x: area.x.ceil(),
                    y: area.y.ceil(),
                    width: area.right().floor() - area.x.ceil(),
                    height: area.bottom().floor() - area.y.ceil(),
                };
                if !region.valid() {
                    return Err("Choose a larger recording area".into());
                }
                Ok(Self {
                    output: output.clone(),
                    region: Some(region),
                })
            }
            _ => Err("Video supports Selection or Screen".into()),
        }
    }
    fn present(&self, layout: &CaptureLayout) -> bool {
        layout.outputs.contains(&self.output)
    }
}

pub fn available() -> bool {
    worker::Programs::find().is_ok()
}

struct Worker {
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct ScreencastService {
        pub phase: Cell<Phase>,
        pub started: Cell<Option<Instant>>,
        pub frozen_elapsed: Cell<Duration>,
        pub settings: OnceCell<gio::Settings>,
        pub notifications: OnceCell<NotificationsService>,
        pub manager: OnceCell<WindowManager>,
        pub(super) worker: RefCell<Option<Worker>>,
        pub task: RefCell<Option<glib::JoinHandle<()>>>,
        pub watcher: RefCell<Option<glib::JoinHandle<()>>>,
        pub reason: RefCell<Option<String>>,
        pub stopped: Cell<bool>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for ScreencastService {
        const NAME: &'static str = "WayShellScreencastService";
        type Type = super::ScreencastService;
    }
    impl ObjectImpl for ScreencastService {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("changed").build()])
        }
        fn dispose(&self) {
            self.obj().shutdown();
        }
    }
}
glib::wrapper! { pub struct ScreencastService(ObjectSubclass<imp::ScreencastService>); }
impl ScreencastService {
    pub fn new(
        settings: gio::Settings,
        notifications: NotificationsService,
        manager: WindowManager,
    ) -> Self {
        let service: Self = glib::Object::new();
        let _ = service.imp().settings.set(settings);
        let _ = service.imp().notifications.set(notifications);
        let _ = service.imp().manager.set(manager);
        service
    }
    pub fn phase(&self) -> Phase {
        self.imp().phase.get()
    }
    pub fn is_active(&self) -> bool {
        self.phase() != Phase::Idle
    }
    pub fn elapsed(&self) -> String {
        let seconds = self.elapsed_duration().as_secs();
        if seconds >= 3600 {
            format!(
                "{}:{:02}:{:02}",
                seconds / 3600,
                seconds / 60 % 60,
                seconds % 60
            )
        } else {
            format!("{:02}:{:02}", seconds / 60, seconds % 60)
        }
    }
    fn elapsed_duration(&self) -> Duration {
        if self.phase() == Phase::Recording {
            self.imp()
                .started
                .get()
                .map_or(Duration::ZERO, |start| start.elapsed())
        } else {
            self.imp().frozen_elapsed.get()
        }
    }
    fn changed(&self, phase: Phase) {
        self.imp().phase.set(phase);
        self.emit_by_name::<()>("changed", &[]);
    }
    pub fn start(&self, target: Target, audio: Audio) -> Result<(), String> {
        if self.imp().stopped.get() || self.is_active() {
            return Err("A recording is already running or the recorder has stopped".into());
        }
        let programs = worker::Programs::find()?;
        let directory = destination(
            &self
                .imp()
                .settings
                .get()
                .unwrap()
                .string("screencast-save-directory"),
        )?;
        let stamp = glib::DateTime::now_local()
            .and_then(|time| time.format("%Y-%m-%d-%H%M%S"))
            .map(|s| s.to_string())
            .unwrap_or_else(|_| glib::real_time().to_string());
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let worker_stop = stop.clone();
        let worker_target = target.clone();
        let thread = std::thread::Builder::new()
            .name("way-shell-recording".into())
            .spawn(move || {
                let result = worker::record(
                    &programs,
                    &directory,
                    &stamp,
                    &worker_target,
                    audio,
                    &worker_stop,
                    &sender,
                );
                let _ = sender.send(worker::Event::Finished(result));
            })
            .map_err(|e| format!("Could not start recording worker: {e}"))?;
        self.imp().worker.replace(Some(Worker { stop, thread }));
        self.imp().reason.take();
        self.imp().started.set(None);
        self.imp().frozen_elapsed.set(Duration::ZERO);
        self.changed(Phase::Starting);
        if self.imp().stopped.get() {
            return Ok(());
        }
        let weak = self.downgrade();
        self.imp()
            .task
            .replace(Some(glib::spawn_future_local(async move {
                let mut tick = Instant::now();
                loop {
                    let Some(this) = weak.upgrade() else {
                        return;
                    };
                    match receiver.try_recv() {
                        Ok(worker::Event::Started(start)) => {
                            this.imp().started.set(Some(start));
                            if this.phase() != Phase::Stopping {
                                this.changed(Phase::Recording);
                            }
                        }
                        Ok(worker::Event::Finished(result)) => {
                            this.finish(result);
                            return;
                        }
                        Err(mpsc::TryRecvError::Disconnected) => {
                            this.finish(Err("Recording worker stopped unexpectedly".into()));
                            return;
                        }
                        Err(mpsc::TryRecvError::Empty) => {}
                    }
                    if tick.elapsed() >= Duration::from_secs(1) {
                        this.emit_by_name::<()>("changed", &[]);
                        tick = Instant::now();
                    }
                    drop(this);
                    glib::timeout_future(Duration::from_millis(50)).await;
                }
            })));
        let manager = self.imp().manager.get().unwrap().clone();
        let weak = self.downgrade();
        self.imp().watcher.replace(Some(glib::spawn_future_local(async move {
            loop {
                let valid = matches!(glib::future_with_timeout(Duration::from_secs(2), manager.capture_layout()).await,
                    Ok(Ok(ref layout)) if target.present(layout));
                let Some(this) = weak.upgrade() else { return; };
                if !valid {
                    this.imp().reason.replace(Some("The recorded display changed or became unavailable".into()));
                    this.stop();
                    return;
                }
                drop(this);
                glib::timeout_future(Duration::from_millis(500)).await;
            }
        })));
        Ok(())
    }
    pub fn stop(&self) {
        if let Some(worker) = self.imp().worker.borrow().as_ref() {
            worker.stop.store(true, Ordering::Release);
        }
        if matches!(self.phase(), Phase::Starting | Phase::Recording) {
            self.imp().frozen_elapsed.set(self.elapsed_duration());
            self.changed(Phase::Stopping);
        }
    }
    fn finish(&self, result: Result<Option<PathBuf>, String>) {
        self.imp().task.borrow_mut().take();
        if let Some(task) = self.imp().watcher.take() {
            task.abort();
        }
        if let Some(worker) = self.imp().worker.take() {
            let _ = worker.thread.join();
        }
        self.imp().started.set(None);
        self.imp().frozen_elapsed.set(Duration::ZERO);
        let reason = self.imp().reason.take();
        self.changed(Phase::Idle);
        match result {
            Ok(Some(path)) => {
                let mut body = format!("Saved to {}", path.display());
                if let Some(reason) = reason {
                    body.push_str(&format!(". {reason}"));
                }
                self.notify("Recording saved", &body);
            }
            Ok(None) => {
                if let Some(reason) = reason {
                    self.notify("Recording stopped", &reason);
                }
            }
            Err(error) => self.notify("Recording failed", &error),
        }
    }
    fn notify(&self, title: &str, body: &str) {
        glib::g_message!("way-shell", "{title}: {body}");
        if let Some(notifications) = self.imp().notifications.get() {
            let _ = notifications.send_internal(NotificationRequest {
                app_name: "Screen Capture".into(),
                app_icon: "camera-web-symbolic".into(),
                summary: title.into(),
                body: glib::markup_escape_text(body).into(),
                ..NotificationRequest::default()
            });
        }
    }
    pub fn shutdown(&self) {
        if self.imp().stopped.replace(true) {
            return;
        }
        self.stop();
        if let Some(task) = self.imp().task.take() {
            task.abort();
        }
        if let Some(task) = self.imp().watcher.take() {
            task.abort();
        }
        // The worker owns/reaps every subprocess even after the GLib loop exits.
        // Its stop and probe deadlines bound this join; no detached recorder remains.
        if let Some(worker) = self.imp().worker.take() {
            let _ = worker.thread.join();
        }
        self.imp().phase.set(Phase::Idle);
        self.imp().started.set(None);
        self.imp().frozen_elapsed.set(Duration::ZERO);
    }
}

fn destination(value: &str) -> Result<PathBuf, String> {
    let path = if value.is_empty() {
        glib::user_special_dir(glib::UserDirectory::Videos)
            .unwrap_or_else(glib::home_dir)
            .join("Screencasts")
    } else if let Some(rest) = value.strip_prefix("~/") {
        glib::home_dir().join(rest)
    } else {
        PathBuf::from(value)
    };
    if !path.is_absolute() {
        return Err("Recording directory must be absolute or start with ~/".into());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn elapsed_is_shared_frozen_when_stopping_and_zero_before_startup() {
        let service: ScreencastService = glib::Object::new();
        service.changed(Phase::Starting);
        assert_eq!(service.elapsed(), "00:00");
        service.stop();
        // A late startup event must not start the visible clock after cancellation.
        service
            .imp()
            .started
            .set(Some(Instant::now() - Duration::from_secs(65)));
        assert_eq!(service.elapsed(), "00:00");
        service.changed(Phase::Recording);
        assert_eq!(service.elapsed(), "01:05");
        service.stop();
        let another_panel = service.clone();
        service
            .imp()
            .started
            .set(Some(Instant::now() - Duration::from_secs(130)));
        assert_eq!(another_panel.elapsed(), "01:05");
        service.finish(Err("test failure".into()));
        assert_eq!(service.elapsed(), "00:00");
        assert_eq!(service.phase(), Phase::Idle);
    }
    #[test]
    fn region_is_contained_rounded_inward_and_bound_to_output_geometry() {
        let output = CaptureOutput {
            connector: "DP-1".into(),
            bounds: Rect {
                x: -200.0,
                y: 0.0,
                width: 200.0,
                height: 100.0,
            },
            scale: 1.25,
            transform: "90".into(),
            native_size: (250, 125),
        };
        let mut layout = CaptureLayout {
            outputs: vec![output],
            windows: vec![],
            focused_output: None,
        };
        let mut selection = Selection::new(&layout, 0, None);
        selection.area = Rect {
            x: -150.5,
            y: 10.2,
            width: 81.7,
            height: 51.4,
        };
        let target = Target::from_selection(&layout, &selection).unwrap();
        assert_eq!(
            target.region,
            Some(Rect {
                x: -150.0,
                y: 11.0,
                width: 81.0,
                height: 50.0
            })
        );
        assert!(target.present(&layout));
        layout.outputs[0].scale = 2.0;
        assert!(!target.present(&layout));
        selection.area.width = 250.0;
        assert!(Target::from_selection(&layout, &selection).is_err());
        selection.mode = Mode::All;
        assert!(Target::from_selection(&layout, &selection).is_err());
        selection.mode = Mode::Screen;
        assert!(
            Target::from_selection(&layout, &selection)
                .unwrap()
                .region
                .is_none()
        );
    }
    #[test]
    fn custom_recording_directories_are_not_shell_expanded() {
        assert_eq!(
            destination("~/clips").unwrap(),
            glib::home_dir().join("clips")
        );
        assert_eq!(
            destination("/tmp/a $folder").unwrap(),
            PathBuf::from("/tmp/a $folder")
        );
        for value in ["relative", "~", "$HOME/videos"] {
            assert!(destination(value).is_err());
        }
    }
}
