//! Optional loading presentation only. Callers retain their real operation guards.
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

const DELAY: Duration = Duration::from_millis(300);
const MINIMUM: Duration = Duration::from_millis(500);

#[derive(Default)]
struct Timing {
    generation: u64,
    busy: bool,
    shown: Option<Instant>,
    deadline: Option<Instant>,
}
impl Timing {
    fn begin(&mut self, now: Instant) -> bool {
        if self.busy {
            return false;
        }
        self.generation = self.generation.wrapping_add(1);
        self.busy = true;
        self.deadline = self.shown.is_none().then_some(now + DELAY);
        true
    }
    fn complete(&mut self, now: Instant) -> bool {
        if !self.busy {
            return false;
        }
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
        self.deadline = self.shown.map(|shown| now.max(shown + MINIMUM));
        true
    }
    fn fire(&mut self, generation: u64, now: Instant) -> Option<bool> {
        if generation != self.generation || self.deadline.is_none_or(|due| now < due) {
            return None;
        }
        self.deadline = None;
        self.shown = self.busy.then_some(now);
        Some(self.busy)
    }
    fn clear(&mut self) -> bool {
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
        self.deadline = None;
        self.shown.take().is_some()
    }
}

struct Inner {
    timing: RefCell<Timing>,
    timer: RefCell<Option<glib::SourceId>>,
    done: RefCell<Option<Box<dyn FnOnce()>>>,
    changed: Box<dyn Fn(bool)>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.get_mut().take() {
            timer.remove();
        }
    }
}
pub(super) struct Progress(Rc<Inner>);
impl Progress {
    pub fn new(changed: impl Fn(bool) + 'static) -> Self {
        Self(Rc::new(Inner {
            timing: RefCell::new(Timing::default()),
            timer: RefCell::new(None),
            done: RefCell::new(None),
            changed: Box::new(changed),
        }))
    }
    pub fn visible(&self) -> bool {
        self.0.timing.borrow().shown.is_some()
    }
    pub fn begin(&self) {
        if self.0.timing.borrow_mut().begin(Instant::now()) {
            self.0.done.borrow_mut().take();
            self.arm();
        }
    }
    pub fn set_busy(&self, busy: bool) {
        if busy {
            self.begin();
        } else {
            self.finish(|| {});
        }
    }
    /// Underlying work has succeeded. Only its presentation may wait.
    pub fn finish(&self, done: impl FnOnce() + 'static) {
        if !self.0.timing.borrow_mut().complete(Instant::now()) {
            return;
        }
        self.0.done.replace(Some(Box::new(done)));
        self.arm();
    }
    /// Errors, cancellation and hiding bypass the minimum and invalidate callbacks.
    pub fn clear(&self) {
        let visible = self.0.timing.borrow_mut().clear();
        self.0.done.borrow_mut().take();
        if let Some(timer) = self.0.timer.borrow_mut().take() {
            timer.remove();
        }
        if visible {
            (self.0.changed)(false);
        }
    }
    fn arm(&self) {
        if let Some(timer) = self.0.timer.borrow_mut().take() {
            timer.remove();
        }
        let (generation, deadline) = {
            let state = self.0.timing.borrow();
            (state.generation, state.deadline)
        };
        let now = Instant::now();
        if let Some(deadline) = deadline.filter(|deadline| *deadline > now) {
            let weak = Rc::downgrade(&self.0);
            self.0.timer.replace(Some(glib::timeout_add_local_once(
                deadline - now,
                move || {
                    if let Some(inner) = weak.upgrade() {
                        inner.timer.borrow_mut().take();
                        Progress(inner).advance(generation);
                    }
                },
            )));
        } else {
            self.advance(generation);
        }
    }
    fn advance(&self, generation: u64) {
        let event = self.0.timing.borrow_mut().fire(generation, Instant::now());
        if let Some(visible) = event {
            (self.0.changed)(visible);
        }
        let completed = {
            let state = self.0.timing.borrow();
            state.generation == generation && !state.busy && state.deadline.is_none()
        };
        if completed {
            let done = self.0.done.borrow_mut().take();
            if let Some(done) = done {
                done();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fast_success_never_shows_progress() {
        let now = Instant::now();
        let mut timing = Timing::default();
        timing.begin(now);
        let old = timing.generation;
        timing.complete(now + DELAY - Duration::from_millis(1));
        assert_eq!(timing.fire(old, now + DELAY), None);
        assert!(timing.shown.is_none());
        assert!(timing.deadline.is_none());
    }
    #[test]
    fn slow_success_honors_minimum_without_restarting_on_repeated_updates() {
        let now = Instant::now();
        let mut timing = Timing::default();
        timing.begin(now);
        assert!(!timing.begin(now + Duration::from_millis(250)));
        assert_eq!(timing.fire(timing.generation, now + DELAY), Some(true));
        timing.complete(now + DELAY + Duration::from_millis(1));
        assert!(!timing.complete(now + DELAY + Duration::from_millis(100)));
        assert_eq!(
            timing.fire(
                timing.generation,
                now + DELAY + MINIMUM - Duration::from_millis(1)
            ),
            None
        );
        assert_eq!(
            timing.fire(timing.generation, now + DELAY + MINIMUM),
            Some(false)
        );
    }
    #[test]
    fn overlapping_operations_invalidate_old_completion_and_errors_and_teardown_clear() {
        let now = Instant::now();
        let mut timing = Timing::default();
        timing.begin(now);
        timing.fire(timing.generation, now + DELAY);
        timing.complete(now + DELAY);
        let old = timing.generation;
        timing.begin(now + DELAY);
        assert_eq!(timing.fire(old, now + DELAY + MINIMUM), None);
        assert!(timing.shown.is_some());
        for _ in 0..2 {
            let old = timing.generation;
            timing.clear();
            assert_eq!(timing.fire(old, now + Duration::from_secs(10)), None);
            assert!(timing.shown.is_none());
            assert!(timing.deadline.is_none());
            timing.begin(now);
        }
    }
}
