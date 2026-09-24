//! Message-tray fade timing and click-away surfaces.
use crate::ui::window::{
    LayerWindow, Transition, UnderlaySet, Visibility, VisibilityController, WindowRole,
};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageTrayEvent {
    WillShow,
    Visible,
    WillHide,
    Hidden,
}
type Observer = Rc<dyn Fn(MessageTrayEvent)>;
const SCREEN_VERTICAL_MARGIN: i32 = 80;
pub(super) const PREFERRED_HEIGHT: i32 = 600;
// Both bundled tray themes use 6 px padding and a 1 px border on each edge.
pub(super) const TRAY_VERTICAL_INSET: i32 = 14;

fn height_budget(monitor_height: i32) -> i32 {
    (monitor_height - SCREEN_VERTICAL_MARGIN).max(0)
}

pub struct MessageTrayWindow {
    visibility: VisibilityController,
    content: gtk::Box,
    animation: RefCell<Option<adw::TimedAnimation>>,
    observers: RefCell<Vec<Observer>>,
    events: RefCell<VecDeque<MessageTrayEvent>>,
    height_budget: Cell<i32>,
    height_observer: RefCell<Option<Box<dyn Fn(i32)>>>,
    monitor: RefCell<Option<(gtk::gdk::Monitor, glib::SignalHandlerId)>>,
    publishing: Cell<bool>,
    desired: Cell<bool>,
    announced: Cell<bool>,
    revision: Cell<u64>,
    closed: Cell<bool>,
}
impl MessageTrayWindow {
    pub fn new() -> Result<Rc<Self>, String> {
        let main = LayerWindow::new(WindowRole::MessageTray, None)?;
        main.window().set_widget_name("message-tray");
        main.window().set_opacity(0.0);
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        content.set_width_request(700);
        main.window().set_content(Some(&content));
        main.window().set_default_size(700, PREFERRED_HEIGHT);
        let underlays = UnderlaySet::new(WindowRole::MessageTrayUnderlay)?;
        let this = Rc::new(Self {
            visibility: VisibilityController::new(main, Some(underlays)),
            content,
            animation: RefCell::new(None),
            observers: RefCell::new(Vec::new()),
            events: RefCell::new(VecDeque::new()),
            height_budget: Cell::new(PREFERRED_HEIGHT),
            height_observer: RefCell::new(None),
            monitor: RefCell::new(None),
            publishing: Cell::new(false),
            desired: Cell::new(false),
            announced: Cell::new(false),
            revision: Cell::new(0),
            closed: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        this.visibility.underlays().unwrap().on_dismiss(move || {
            if let Some(this) = weak.upgrade() {
                this.hide();
            }
        });
        let weak = Rc::downgrade(&this);
        this.visibility.main().on_closed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.close();
            }
        });
        let weak = Rc::downgrade(&this);
        this.window().connect_realize(move |window| {
            if let Some(this) = weak.upgrade() {
                if let Some(surface) = window.surface() {
                    let weak = Rc::downgrade(&this);
                    surface.connect_enter_monitor(move |_, _| {
                        if let Some(this) = weak.upgrade() {
                            this.refresh_height_budget();
                        }
                    });
                }
                this.refresh_height_budget();
            }
        });
        let weak = Rc::downgrade(&this);
        this.window().connect_map(move |_| {
            if let Some(this) = weak.upgrade() {
                this.refresh_height_budget();
            }
        });
        Ok(this)
    }
    pub fn window(&self) -> &adw::Window {
        self.visibility.main().window()
    }
    pub fn content(&self) -> &gtk::Box {
        &self.content
    }
    pub fn underlay_buttons(&self) -> Vec<gtk::Button> {
        self.visibility.underlays().unwrap().buttons()
    }
    pub fn visibility_controller(&self) -> &VisibilityController {
        &self.visibility
    }
    pub fn on_event(&self, observer: impl Fn(MessageTrayEvent) + 'static) {
        self.observers.borrow_mut().push(Rc::new(observer));
    }
    pub fn on_height_budget(&self, observer: impl Fn(i32) + 'static) {
        observer(self.height_budget.get());
        self.height_observer.replace(Some(Box::new(observer)));
    }
    fn refresh_height_budget(self: &Rc<Self>) {
        let monitor = self
            .window()
            .surface()
            .and_then(|surface| WidgetExt::display(self.window()).monitor_at_surface(&surface));
        let changed =
            self.monitor.borrow().as_ref().map(|(current, _)| current) != monitor.as_ref();
        if changed {
            if let Some((old, handler)) = self.monitor.borrow_mut().take() {
                old.disconnect(handler);
            }
            if let Some(monitor) = monitor.as_ref() {
                let weak = Rc::downgrade(self);
                let handler = monitor.connect_geometry_notify(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.refresh_height_budget();
                    }
                });
                self.monitor.replace(Some((monitor.clone(), handler)));
            }
        }
        if let Some(monitor) = monitor {
            let budget = height_budget(monitor.geometry().height());
            self.height_budget.set(budget);
            self.shrink();
            if let Some(observer) = self.height_observer.borrow().as_ref() {
                observer(budget);
            }
        }
    }
    pub fn show(self: &Rc<Self>) -> bool {
        if self.closed.get() || self.desired.replace(true) {
            return false;
        }
        let revision = self.next_revision();
        self.cancel_animation();
        self.announced.set(true);
        self.publish(MessageTrayEvent::WillShow);
        if !self.current(revision) {
            return false;
        }
        let Some(transition) = self.visibility.begin_show() else {
            return false;
        };
        if !self.current(revision) {
            return false;
        }
        self.animate(transition, true, revision);
        true
    }
    pub fn hide(self: &Rc<Self>) -> bool {
        if self.closed.get() || !self.desired.replace(false) {
            return false;
        }
        let revision = self.next_revision();
        self.cancel_animation();
        if let Some(transition) = self.visibility.begin_hide() {
            self.animate(transition, false, revision);
        } else if self.visibility.visibility() == Visibility::Hidden {
            self.hidden(revision);
        }
        true
    }
    pub fn toggle(self: &Rc<Self>) {
        if self.desired.get() {
            self.hide();
        } else {
            self.show();
        }
    }
    pub fn shrink(&self) {
        self.fit_height(PREFERRED_HEIGHT);
    }
    pub fn fit_height(&self, height: i32) {
        if !self.closed.get() {
            self.window()
                .set_default_size(700, height.min(self.height_budget.get()));
        }
    }
    pub fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.next_revision();
        self.desired.set(false);
        self.cancel_animation();
        if let Some((monitor, handler)) = self.monitor.borrow_mut().take() {
            monitor.disconnect(handler);
        }
        self.height_observer.borrow_mut().take();
        self.visibility.close();
        if self.announced.replace(false) {
            self.publish(MessageTrayEvent::Hidden);
        }
    }
    fn next_revision(&self) -> u64 {
        let revision = self.revision.get().wrapping_add(1);
        self.revision.set(revision);
        revision
    }
    fn current(&self, revision: u64) -> bool {
        !self.closed.get() && self.revision.get() == revision
    }
    fn cancel_animation(&self) {
        let animation = self.animation.borrow_mut().take();
        if let Some(animation) = animation {
            animation.pause();
        }
    }
    fn animate(self: &Rc<Self>, transition: Transition, showing: bool, revision: u64) {
        let weak = Rc::downgrade(self);
        let target = adw::CallbackAnimationTarget::new(move |value| {
            if let Some(this) = weak.upgrade().filter(|this| this.current(revision)) {
                this.window().set_opacity(value);
            }
        });
        let animation = adw::TimedAnimation::new(
            self.window(),
            self.window().opacity(),
            if showing { 1.0 } else { 0.0 },
            250,
            target,
        );
        let weak = Rc::downgrade(self);
        animation.connect_done(move |_| {
            if let Some(this) = weak.upgrade().filter(|this| this.current(revision)) {
                this.animation.borrow_mut().take();
                // Existing groups collapse at the end of the fade, while both
                // surfaces are still mapped. This differs from activities.
                if !showing {
                    this.publish(MessageTrayEvent::WillHide);
                    if !this.current(revision) {
                        return;
                    }
                }
                if this.visibility.finish_transition(&transition) && this.current(revision) {
                    if showing {
                        this.publish(MessageTrayEvent::Visible);
                    } else {
                        this.hidden(revision);
                    }
                }
            }
        });
        if self.current(revision) {
            self.animation.replace(Some(animation.clone()));
            animation.play();
        }
    }
    fn hidden(&self, revision: u64) {
        if self.current(revision) && self.announced.replace(false) {
            self.publish(MessageTrayEvent::Hidden);
            if self.current(revision) {
                self.shrink();
            }
        }
    }
    fn publish(&self, event: MessageTrayEvent) {
        self.events.borrow_mut().push_back(event);
        if self.publishing.replace(true) {
            return;
        }
        loop {
            let Some(event) = self.events.borrow_mut().pop_front() else {
                break;
            };
            let observers = self.observers.borrow().clone();
            for observer in observers {
                observer(event);
            }
        }
        self.publishing.set(false);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_budget_reserves_panel_and_screen_margins() {
        assert_eq!(height_budget(480), 400);
        assert_eq!(height_budget(1080), 1000);
    }
}
impl Drop for MessageTrayWindow {
    fn drop(&mut self) {
        self.observers.get_mut().clear();
        self.events.get_mut().clear();
        self.close();
    }
}
