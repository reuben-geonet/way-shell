use gio::prelude::*;
use std::{
    cell::RefCell,
    collections::VecDeque,
    fs,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};
use way_shell::services::brightness::{Apply, BrightnessService, Completion, ControlKind};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("way-shell-brightness-{}", std::process::id()));
        fs::create_dir_all(path.join("backlight/panel")).unwrap();
        fs::create_dir_all(path.join("leds/keyboard")).unwrap();
        Self(path)
    }
    fn write(&self, kind: &str, device: &str, current: &str, maximum: &str) {
        let path = self.0.join(kind).join(device);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("brightness"), current).unwrap();
        fs::write(path.join("max_brightness"), maximum).unwrap();
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Pending {
    kind: ControlKind,
    name: String,
    value: u32,
    done: Completion,
    cancel: gio::Cancellable,
}
fn wait(context: &glib::MainContext, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "brightness fixture timed out");
        context.block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}
#[test]
fn observed_state_requests_limits_failure_hotplug_settings_and_cancellation() {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let directory = Directory::new();
            directory.write("backlight", "panel", "2\n", "5\n");
            directory.write("leds", "keyboard", "3\n", "3\n");
            let schema = gio::SettingsSchemaSource::from_directory(
                env!("WAY_SHELL_TEST_SCHEMAS"),
                None,
                false,
            )
            .unwrap()
            .lookup("org.ldelossa.way-shell.system", false)
            .unwrap();
            let settings =
                gio::Settings::new_full(&schema, Some(&gio::memory_settings_backend_new()), None);
            settings.set_string("backlight-directory", "panel").unwrap();
            settings
                .set_string("keyboard-backlight-directory", "keyboard")
                .unwrap();
            let pending = Rc::new(RefCell::new(VecDeque::<Pending>::new()));
            let calls = pending.clone();
            let apply: Apply = Rc::new(move |kind, name, value, done| {
                let cancel = gio::Cancellable::new();
                calls.borrow_mut().push_back(Pending {
                    kind,
                    name: name.to_owned(),
                    value,
                    done,
                    cancel: cancel.clone(),
                });
                cancel
            });
            let service = BrightnessService::with_settings_and_root(&settings, &directory.0, apply);
            assert_eq!(
                service.snapshot(ControlKind::Backlight).unwrap().fraction(),
                0.4
            );
            let outcomes = Rc::new(RefCell::new(Vec::new()));
            let outcome = outcomes.clone();
            service.adjust(ControlKind::Backlight, true, move |result| {
                outcome.borrow_mut().push(result.is_ok())
            });
            assert_eq!(
                pending.borrow().front().unwrap().value,
                3,
                "small display ranges must still advance"
            );
            assert_eq!(
                service.snapshot(ControlKind::Backlight).unwrap().current,
                2,
                "queued request must not change observed brightness"
            );
            let request = pending.borrow_mut().pop_front().unwrap();
            assert_eq!(request.kind, ControlKind::Backlight);
            assert_eq!(request.name, "panel");
            (request.done)(Err(glib::Error::new(
                gio::IOErrorEnum::PermissionDenied,
                "fixture denied",
            )));
            assert_eq!(&*outcomes.borrow(), &[false]);
            assert_eq!(service.snapshot(ControlKind::Backlight).unwrap().current, 2);
            // Consecutive commands are serialized and compute from confirmed readback.
            for _ in 0..2 {
                let outcome = outcomes.clone();
                service.adjust(ControlKind::Backlight, true, move |result| {
                    outcome.borrow_mut().push(result.is_ok())
                });
            }
            assert_eq!(pending.borrow().len(), 1);
            for expected in [3, 4] {
                wait(&context, || !pending.borrow().is_empty());
                let request = pending.borrow_mut().pop_front().unwrap();
                assert_eq!(request.value, expected);
                directory.write("backlight", "panel", &expected.to_string(), "5");
                (request.done)(Ok(()));
                wait(&context, || {
                    service.snapshot(ControlKind::Backlight).unwrap().current == expected
                });
            }
            wait(&context, || outcomes.borrow().len() == 3);
            assert_eq!(&*outcomes.borrow(), &[false, true, true]);
            let invalid = Rc::new(RefCell::new(None));
            let result = invalid.clone();
            service.set_backlight(f64::NAN, move |value| {
                result.replace(Some(value.is_err()));
            });
            assert_eq!(*invalid.borrow(), Some(true));
            assert!(pending.borrow().is_empty());
            // Keyboard cycles through off and maximum in either direction.
            service.adjust(ControlKind::Keyboard, true, |_| {});
            let request = pending.borrow_mut().pop_front().unwrap();
            assert_eq!(request.value, 0);
            directory.write("leds", "keyboard", "0", "3");
            (request.done)(Ok(()));
            wait(&context, || {
                service.snapshot(ControlKind::Keyboard).unwrap().current == 0
            });
            service.adjust(ControlKind::Keyboard, false, |_| {});
            let request = pending.borrow_mut().pop_front().unwrap();
            assert_eq!(request.value, 3);
            directory.write("leds", "keyboard", "3", "3");
            (request.done)(Ok(()));
            wait(&context, || {
                service.snapshot(ControlKind::Keyboard).unwrap().current == 3
            });
            let invalid = Rc::new(RefCell::new(None));
            let result = invalid.clone();
            service.set_keyboard(4, move |value| {
                result.replace(Some(value.is_err()));
            });
            assert_eq!(*invalid.borrow(), Some(true));
            // Invalid or missing hardware disables just that control and recovers.
            directory.write("backlight", "panel", "4", "0");
            wait(&context, || {
                service.snapshot(ControlKind::Backlight).is_none()
            });
            assert!(service.snapshot(ControlKind::Keyboard).is_some());
            directory.write("backlight", "panel", "4", "5");
            wait(&context, || {
                service.snapshot(ControlKind::Backlight).is_some()
            });
            fs::remove_dir_all(directory.0.join("backlight/panel")).unwrap();
            wait(&context, || {
                service.snapshot(ControlKind::Backlight).is_none()
            });
            directory.write("backlight", "panel", "1", "5");
            wait(&context, || {
                service.snapshot(ControlKind::Backlight).is_some()
            });
            // Both configured devices are observed; settings cannot escape sysfs roots.
            directory.write("leds", "replacement", "1", "2");
            settings
                .set_string("keyboard-backlight-directory", "replacement")
                .unwrap();
            wait(&context, || {
                service
                    .snapshot(ControlKind::Keyboard)
                    .is_some_and(|d| d.name == "replacement")
            });
            settings
                .set_string("backlight-directory", "../leds/keyboard")
                .unwrap();
            wait(&context, || {
                service.snapshot(ControlKind::Backlight).is_none()
            });
            service.set_backend_available(false);
            assert!(!service.is_available(ControlKind::Keyboard));
            assert!(service.snapshot(ControlKind::Keyboard).is_some());
            service.set_backend_available(true);
            assert!(service.is_available(ControlKind::Keyboard));
            // Disposal cancels pending writes and completes each requester once.
            let cancelled = Rc::new(RefCell::new(Vec::new()));
            for _ in 0..2 {
                let results = cancelled.clone();
                service.adjust(ControlKind::Keyboard, true, move |result| {
                    results.borrow_mut().push(result.is_err())
                });
            }
            let request = pending.borrow_mut().pop_front().unwrap();
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
            assert!(request.cancel.is_cancelled());
            assert_eq!(&*cancelled.borrow(), &[true, true]);
            (request.done)(Ok(()));
            context.block_on(glib::timeout_future(Duration::from_millis(20)));
            assert_eq!(&*cancelled.borrow(), &[true, true]);
        })
        .unwrap();
}
