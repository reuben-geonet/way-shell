//! Configured sysfs brightness devices with observed state and async writes.
use gio::prelude::*;
use glib::subclass::prelude::*;
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::VecDeque,
    path::{Path, PathBuf},
    rc::Rc,
    sync::OnceLock,
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    Backlight,
    Keyboard,
}
impl ControlKind {
    fn index(self) -> usize {
        match self {
            Self::Backlight => 0,
            Self::Keyboard => 1,
        }
    }
    pub fn subsystem(self) -> &'static str {
        match self {
            Self::Backlight => "backlight",
            Self::Keyboard => "leds",
        }
    }
    fn setting(self) -> &'static str {
        match self {
            Self::Backlight => "backlight-directory",
            Self::Keyboard => "keyboard-backlight-directory",
        }
    }
}
const KINDS: [ControlKind; 2] = [ControlKind::Backlight, ControlKind::Keyboard];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrightnessDevice {
    pub name: String,
    pub current: u32,
    pub maximum: u32,
}
impl BrightnessDevice {
    pub fn fraction(&self) -> f64 {
        if self.maximum == 0 {
            0.0
        } else {
            f64::from(self.current) / f64::from(self.maximum)
        }
    }
}
pub type Completion = Box<dyn FnOnce(Result<(), glib::Error>)>;
/// The implementation copies the device name and reports the actual operation
/// result once. Cancelling the returned handle must complete it with an error.
pub type Apply = Rc<dyn Fn(ControlKind, &str, u32, Completion) -> gio::Cancellable>;

enum Action {
    Adjust(bool),
    Fraction(f64),
    Value(u32),
}
struct Request {
    action: Action,
    done: Completion,
}
struct Active {
    id: u64,
    done: Completion,
    cancel: Option<gio::Cancellable>,
}
struct Monitor {
    monitor: gio::FileMonitor,
    handler: Option<glib::SignalHandlerId>,
}
impl Drop for Monitor {
    fn drop(&mut self) {
        if let Some(handler) = self.handler.take() {
            self.monitor.disconnect(handler);
        }
        self.monitor.cancel();
    }
}
#[derive(Default)]
pub(super) struct Control {
    name: String,
    generation: u64,
    device: Option<BrightnessDevice>,
    error: Option<String>,
    reading: Option<glib::JoinHandle<()>>,
    monitors: Vec<Monitor>,
    queue: VecDeque<Request>,
    active: Option<Active>,
}
impl Control {
    fn cancel(&mut self) -> Vec<Completion> {
        self.generation = self.generation.wrapping_add(1);
        if let Some(reading) = self.reading.take() {
            reading.abort();
        }
        self.monitors.clear();
        self.cancel_operations()
    }
    fn cancel_operations(&mut self) -> Vec<Completion> {
        let mut callbacks: Vec<Completion> = Vec::new();
        if let Some(active) = self.active.take() {
            // Cancellation may invoke a backend callback immediately, so it
            // runs after the owner has released its control-state borrow.
            callbacks.push(Box::new(move |result| {
                if let Some(cancel) = active.cancel {
                    cancel.cancel();
                }
                (active.done)(result);
            }));
        }
        callbacks.extend(self.queue.drain(..).map(|request| request.done));
        callbacks
    }
}

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct BrightnessService {
        pub settings: OnceCell<gio::Settings>,
        pub settings_handlers: RefCell<Vec<glib::SignalHandlerId>>,
        pub root: OnceCell<PathBuf>,
        pub apply: OnceCell<Apply>,
        pub(super) controls: RefCell<[Control; 2]>,
        pub poll: RefCell<Option<glib::JoinHandle<()>>>,
        pub next_request: Cell<u64>,
        pub disposed: Cell<bool>,
        pub backend_available: Cell<bool>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for BrightnessService {
        const NAME: &'static str = "WayShellBrightnessService";
        type Type = super::BrightnessService;
    }
    impl ObjectImpl for BrightnessService {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("brightness-changed")
                        .param_types([f32::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("keyboard-brightness-changed")
                        .param_types([u32::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("availability-changed").build(),
                    glib::subclass::Signal::builder("operation-failed")
                        .param_types([String::static_type()])
                        .build(),
                ]
            })
        }
        fn dispose(&self) {
            self.disposed.set(true);
            if let Some(poll) = self.poll.borrow_mut().take() {
                poll.abort();
            }
            if let Some(settings) = self.settings.get() {
                for handler in self.settings_handlers.take() {
                    settings.disconnect(handler);
                }
            }
            let callbacks: Vec<_> = self
                .controls
                .borrow_mut()
                .iter_mut()
                .flat_map(Control::cancel)
                .collect();
            for done in callbacks {
                done(Err(cancelled()));
            }
        }
    }
}
glib::wrapper! { pub struct BrightnessService(ObjectSubclass<imp::BrightnessService>); }
impl BrightnessService {
    pub fn new(apply: Apply) -> Result<Self, glib::BoolError> {
        let settings = super::settings::open("org.ldelossa.way-shell.system")?;
        Ok(Self::with_settings_and_root(
            &settings,
            Path::new("/sys/class"),
            apply,
        ))
    }
    pub fn with_settings_and_root(settings: &gio::Settings, root: &Path, apply: Apply) -> Self {
        let service: Self = glib::Object::new();
        service.imp().backend_available.set(true);
        service.imp().settings.set(settings.clone()).unwrap();
        service.imp().root.set(root.to_owned()).unwrap();
        assert!(service.imp().apply.set(apply).is_ok());
        for kind in KINDS {
            // Two small local sysfs attributes establish startup availability for
            // existing C widgets. All later reads and privileged writes are async.
            service.configure(kind, true);
            let weak = service.downgrade();
            let handler = settings.connect_changed(Some(kind.setting()), move |_, _| {
                if let Some(service) = weak.upgrade() {
                    service.configure(kind, false);
                }
            });
            service.imp().settings_handlers.borrow_mut().push(handler);
        }
        let weak = service.downgrade();
        let poll = glib::MainContext::ref_thread_default().spawn_local(async move {
            loop {
                glib::timeout_future(Duration::from_secs(1)).await;
                // sysfs hardware updates do not consistently generate inotify
                // events. Async polling also recovers removed/re-added devices.
                let Some(service) = weak.upgrade() else { break };
                service.refresh();
            }
        });
        service.imp().poll.replace(Some(poll));
        service
    }
    pub fn is_available(&self, kind: ControlKind) -> bool {
        self.imp().backend_available.get() && self.snapshot(kind).is_some()
    }
    pub fn set_backend_available(&self, available: bool) {
        if self.imp().backend_available.replace(available) == available {
            return;
        }
        if !available {
            let callbacks: Vec<_> = self
                .imp()
                .controls
                .borrow_mut()
                .iter_mut()
                .flat_map(Control::cancel_operations)
                .collect();
            for done in callbacks {
                done(Err(cancelled()));
            }
        }
        self.emit_by_name::<()>("availability-changed", &[]);
    }
    pub fn snapshot(&self, kind: ControlKind) -> Option<BrightnessDevice> {
        self.imp().controls.borrow()[kind.index()].device.clone()
    }
    pub fn adjust(
        &self,
        kind: ControlKind,
        up: bool,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) {
        self.submit(kind, Action::Adjust(up), Box::new(done));
    }
    pub fn set_backlight(
        &self,
        fraction: f64,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) {
        self.submit(
            ControlKind::Backlight,
            Action::Fraction(fraction),
            Box::new(done),
        );
    }
    pub fn set_keyboard(&self, value: u32, done: impl FnOnce(Result<(), glib::Error>) + 'static) {
        self.submit(ControlKind::Keyboard, Action::Value(value), Box::new(done));
    }
    pub fn refresh(&self) {
        for kind in KINDS {
            self.start_read(kind, None);
        }
    }

    fn configure(&self, kind: ControlKind, initial: bool) {
        let name = self
            .imp()
            .settings
            .get()
            .unwrap()
            .string(kind.setting())
            .to_string();
        let callbacks = {
            let mut controls = self.imp().controls.borrow_mut();
            let control = &mut controls[kind.index()];
            let callbacks = control.cancel();
            control.name = name.clone();
            control.error = None;
            callbacks
        };
        for done in callbacks {
            done(Err(cancelled()));
        }
        self.observe(kind, None, None);
        if name.is_empty() {
            return;
        }
        if !valid_name(&name) {
            self.observe(
                kind,
                None,
                Some(format!("{} must name one sysfs device", kind.setting())),
            );
            return;
        }
        let base = self.imp().root.get().unwrap().join(kind.subsystem());
        let path = base.join(&name);
        for watched in [base, path.clone()] {
            match gio::File::for_path(watched)
                .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
            {
                Ok(monitor) => {
                    let weak = self.downgrade();
                    let handler = monitor.connect_changed(move |_, _, _, _| {
                        if let Some(service) = weak.upgrade() {
                            service.start_read(kind, None);
                        }
                    });
                    self.imp().controls.borrow_mut()[kind.index()]
                        .monitors
                        .push(Monitor {
                            monitor,
                            handler: Some(handler),
                        });
                }
                Err(error) => glib::g_debug!(
                    "way-shell",
                    "Brightness directory monitor unavailable: {error}; using periodic reads"
                ),
            }
        }
        if initial {
            match read_initial(&name, &path) {
                Ok(device) => self.observe(kind, Some(device), None),
                Err(error) => self.observe(kind, None, Some(error.to_string())),
            }
        } else {
            self.start_read(kind, None);
        }
    }

    fn start_read(&self, kind: ControlKind, finish: Option<u64>) {
        if self.imp().disposed.get() {
            return;
        }
        let (name, generation) = {
            let mut controls = self.imp().controls.borrow_mut();
            let control = &mut controls[kind.index()];
            if !valid_name(&control.name) {
                return;
            }
            if control.reading.is_some() {
                if finish.is_none() {
                    return;
                }
                control.reading.take().unwrap().abort();
            }
            (control.name.clone(), control.generation)
        };
        let path = self
            .imp()
            .root
            .get()
            .unwrap()
            .join(kind.subsystem())
            .join(&name);
        let weak = self.downgrade();
        let reading = glib::MainContext::ref_thread_default().spawn_local(async move {
            let result = read_async(&name, &path).await;
            let Some(service) = weak.upgrade() else {
                return;
            };
            if service.imp().controls.borrow()[kind.index()].generation != generation {
                return;
            }
            service.imp().controls.borrow_mut()[kind.index()]
                .reading
                .take();
            let outcome = match result {
                Ok(device) => {
                    service.observe(kind, Some(device), None);
                    Ok(())
                }
                Err(error) => {
                    service.observe(kind, None, Some(error.to_string()));
                    Err(error)
                }
            };
            if let Some(id) = finish {
                service.complete(kind, generation, id, outcome);
            }
        });
        self.imp().controls.borrow_mut()[kind.index()].reading = Some(reading);
    }

    fn observe(&self, kind: ControlKind, device: Option<BrightnessDevice>, error: Option<String>) {
        let (changed, availability, report) = {
            let mut controls = self.imp().controls.borrow_mut();
            let control = &mut controls[kind.index()];
            let changed = control.device != device;
            let availability = control.device.as_ref().map(|d| (&d.name, d.maximum))
                != device.as_ref().map(|d| (&d.name, d.maximum));
            let report = error != control.error;
            control.device = device.clone();
            control.error = error.clone();
            (changed, availability, report)
        };
        if report && let Some(error) = error {
            glib::g_warning!(
                "way-shell",
                "{} brightness unavailable: {error}",
                kind.subsystem()
            );
        }
        if availability {
            self.emit_by_name::<()>("availability-changed", &[]);
        }
        if changed && let Some(device) = device {
            match kind {
                ControlKind::Backlight => {
                    self.emit_by_name::<()>("brightness-changed", &[&(device.fraction() as f32)])
                }
                ControlKind::Keyboard => {
                    self.emit_by_name::<()>("keyboard-brightness-changed", &[&device.current])
                }
            }
        }
    }

    fn submit(&self, kind: ControlKind, action: Action, done: Completion) {
        if self.imp().disposed.get() {
            done(Err(cancelled()));
            return;
        }
        if !self.imp().backend_available.get() {
            done(Err(glib::Error::new(
                gio::IOErrorEnum::NotConnected,
                "The brightness control service is unavailable",
            )));
            return;
        }
        if let Err(error) = target(self.snapshot(kind).as_ref(), kind, &action) {
            done(Err(error));
            return;
        }
        if self.imp().controls.borrow()[kind.index()].queue.len() >= 64 {
            done(Err(glib::Error::new(
                gio::IOErrorEnum::Busy,
                "Too many pending brightness operations",
            )));
            return;
        }
        self.imp().controls.borrow_mut()[kind.index()]
            .queue
            .push_back(Request { action, done });
        self.start_next(kind);
    }
    fn start_next(&self, kind: ControlKind) {
        let (request, device, generation) = {
            let mut controls = self.imp().controls.borrow_mut();
            let control = &mut controls[kind.index()];
            if control.active.is_some() {
                return;
            }
            let Some(request) = control.queue.pop_front() else {
                return;
            };
            (request, control.device.clone(), control.generation)
        };
        let value = match target(device.as_ref(), kind, &request.action) {
            Ok(value) => value,
            Err(error) => {
                (request.done)(Err(error));
                self.start_next(kind);
                return;
            }
        };
        let id = self.imp().next_request.get();
        self.imp().next_request.set(id.wrapping_add(1));
        self.imp().controls.borrow_mut()[kind.index()].active = Some(Active {
            id,
            done: request.done,
            cancel: None,
        });
        let weak = self.downgrade();
        let callback: Completion = Box::new(move |result| {
            let Some(service) = weak.upgrade() else {
                return;
            };
            {
                let controls = service.imp().controls.borrow();
                let control = &controls[kind.index()];
                if control.generation != generation
                    || !control
                        .active
                        .as_ref()
                        .is_some_and(|active| active.id == id)
                {
                    return;
                }
            }
            match result {
                Ok(()) => service.start_read(kind, Some(id)),
                Err(error) => service.complete(kind, generation, id, Err(error)),
            }
        });
        let cancel = self.imp().apply.get().unwrap()(kind, &device.unwrap().name, value, callback);
        let mut controls = self.imp().controls.borrow_mut();
        let control = &mut controls[kind.index()];
        if let Some(active) = control.active.as_mut().filter(|active| active.id == id) {
            active.cancel = Some(cancel);
        }
    }
    fn complete(
        &self,
        kind: ControlKind,
        generation: u64,
        id: u64,
        result: Result<(), glib::Error>,
    ) {
        let active = {
            let mut controls = self.imp().controls.borrow_mut();
            let control = &mut controls[kind.index()];
            if control.generation != generation
                || !control
                    .active
                    .as_ref()
                    .is_some_and(|active| active.id == id)
            {
                return;
            }
            control.active.take().unwrap()
        };
        if let Err(error) = result.as_ref() {
            self.emit_by_name::<()>("operation-failed", &[&error.to_string()]);
        }
        (active.done)(result);
        self.start_next(kind);
    }
}

fn cancelled() -> glib::Error {
    glib::Error::new(
        gio::IOErrorEnum::Cancelled,
        "Brightness device or service changed",
    )
}
fn invalid(message: &str) -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::InvalidData, message)
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.chars().any(char::is_control)
}
fn parse(value: &[u8]) -> Result<u32, glib::Error> {
    if value.len() > 64 {
        return Err(invalid("Oversized sysfs brightness value"));
    }
    std::str::from_utf8(value)
        .ok()
        .and_then(|text| {
            let text = text.trim();
            if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
                None
            } else {
                text.parse().ok()
            }
        })
        .ok_or_else(|| invalid("Invalid sysfs brightness value"))
}
fn device(name: &str, current: &[u8], maximum: &[u8]) -> Result<BrightnessDevice, glib::Error> {
    let maximum = parse(maximum)?;
    let current = parse(current)?;
    if maximum == 0 || current > maximum {
        return Err(invalid("Brightness is outside the device range"));
    }
    Ok(BrightnessDevice {
        name: name.to_owned(),
        current,
        maximum,
    })
}
fn read_initial(name: &str, path: &Path) -> Result<BrightnessDevice, glib::Error> {
    use std::io::Read;
    let read = |path: PathBuf| -> Result<Vec<u8>, glib::Error> {
        let mut value = Vec::new();
        std::fs::File::open(path)
            .and_then(|file| file.take(65).read_to_end(&mut value))
            .map_err(|error| glib::Error::new(gio::IOErrorEnum::Failed, &error.to_string()))?;
        Ok(value)
    };
    device(
        name,
        &read(path.join("brightness"))?,
        &read(path.join("max_brightness"))?,
    )
}
async fn read_async(name: &str, path: &Path) -> Result<BrightnessDevice, glib::Error> {
    let (current, _) = gio::File::for_path(path.join("brightness"))
        .load_contents_future()
        .await?;
    let (maximum, _) = gio::File::for_path(path.join("max_brightness"))
        .load_contents_future()
        .await?;
    device(name, &current, &maximum)
}
fn target(
    device: Option<&BrightnessDevice>,
    kind: ControlKind,
    action: &Action,
) -> Result<u32, glib::Error> {
    let device = device.ok_or_else(|| {
        glib::Error::new(
            gio::IOErrorEnum::NotFound,
            "No configured brightness device is available",
        )
    })?;
    match *action {
        Action::Fraction(fraction) if fraction.is_finite() => {
            Ok((fraction.clamp(0.0, 1.0) * f64::from(device.maximum)) as u32)
        }
        Action::Fraction(_) => Err(invalid("Brightness fraction must be finite")),
        Action::Value(value) if value <= device.maximum => Ok(value),
        Action::Value(_) => Err(invalid("Brightness exceeds the device maximum")),
        Action::Adjust(up) => Ok(match (kind, up) {
            (ControlKind::Backlight, true) => device
                .current
                .saturating_add((device.maximum / 12).max(1))
                .min(device.maximum),
            (ControlKind::Backlight, false) => {
                device.current.saturating_sub((device.maximum / 12).max(1))
            }
            (ControlKind::Keyboard, true) => {
                if device.current >= device.maximum {
                    0
                } else {
                    device.current + 1
                }
            }
            (ControlKind::Keyboard, false) => {
                if device.current == 0 {
                    device.maximum
                } else {
                    device.current - 1
                }
            }
        }),
    }
}
