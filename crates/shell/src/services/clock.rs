use glib::{prelude::*, subclass::prelude::*};
use std::{
    cell::{Cell, OnceCell, RefCell},
    sync::OnceLock,
    time::Duration,
};

type Now = Box<dyn Fn() -> Result<glib::DateTime, glib::BoolError>>;

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct ClockService {
        pub enabled: Cell<bool>,
        pub timer: RefCell<Option<glib::SourceId>>,
        pub(super) now: OnceCell<Now>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for ClockService {
        const NAME: &'static str = "WayShellClockService";
        type Type = super::ClockService;
    }
    impl ObjectImpl for ClockService {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("tick")
                        .param_types([glib::DateTime::static_type()])
                        .run_first()
                        .build(),
                ]
            })
        }
        fn dispose(&self) {
            self.enabled.set(false);
            if let Some(timer) = self.timer.borrow_mut().take() {
                timer.remove();
            }
        }
    }
}

glib::wrapper! { pub struct ClockService(ObjectSubclass<imp::ClockService>); }

impl Default for ClockService {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockService {
    pub fn new() -> Self {
        Self::with_clock(Box::new(glib::DateTime::now_local))
    }

    fn with_clock(now: Now) -> Self {
        let service: Self = glib::Object::new();
        assert!(service.imp().now.set(now).is_ok());
        service.set_enabled(true);
        service
    }

    pub fn is_enabled(&self) -> bool {
        self.imp().enabled.get()
    }

    pub fn set_enabled(&self, enabled: bool) {
        if self.imp().enabled.replace(enabled) == enabled {
            return;
        }
        if let Some(timer) = self.imp().timer.borrow_mut().take() {
            timer.remove();
        }
        if enabled {
            self.tick_and_schedule();
        }
    }

    fn tick_and_schedule(&self) {
        let delay = match self.imp().now.get().unwrap()() {
            Ok(now) => {
                self.emit_by_name::<()>("tick", &[&now]);
                way_shell_core::clock::until_next_minute(
                    now.to_unix() * 1_000_000 + i64::from(now.microsecond()),
                )
            }
            Err(error) => {
                glib::g_warning!("way-shell", "Could not read local time: {error}");
                Duration::from_secs(1)
            }
        };
        if !self.is_enabled() {
            return;
        }
        let weak = self.downgrade();
        let timer = glib::timeout_add_local_once(delay, move || {
            if let Some(service) = weak.upgrade() {
                // The current source removes itself after the callback.
                service.imp().timer.borrow_mut().take();
                service.tick_and_schedule();
            }
        });
        self.imp().timer.replace(Some(timer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    #[test]
    fn first_minute_stop_from_callback_and_owner_cleanup() {
        let context = glib::MainContext::default();
        context
            .with_thread_default(|| {
                let first = Cell::new(true);
                let service = ClockService::with_clock(Box::new(move || {
                    if first.replace(false) {
                        glib::DateTime::from_utc(2026, 9, 15, 12, 34, 59.99)
                    } else {
                        glib::DateTime::from_utc(2026, 9, 15, 12, 35, 0.0)
                    }
                }));
                let ticks = Rc::new(Cell::new(0));
                let recorded = ticks.clone();
                service.connect_local("tick", false, move |values| {
                    let now = values[1].get::<glib::DateTime>().unwrap();
                    assert_eq!(now.minute(), 35);
                    assert_eq!(now.second(), 0);
                    recorded.set(recorded.get() + 1);
                    values[0].get::<ClockService>().unwrap().set_enabled(false);
                    None
                });
                context.block_on(glib::timeout_future(Duration::from_millis(50)));
                assert_eq!(ticks.get(), 1);
                assert!(!service.is_enabled());
                assert!(service.imp().timer.borrow().is_none());
                let weak = service.downgrade();
                drop(service);
                assert!(weak.upgrade().is_none());
                for _ in 0..20 {
                    let service = ClockService::new();
                    assert!(service.imp().timer.borrow().is_some());
                    let weak = service.downgrade();
                    drop(service);
                    assert!(weak.upgrade().is_none());
                }
            })
            .unwrap();
    }
}
