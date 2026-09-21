//! Passive volume and brightness feedback, owned by one GTK main-thread controller.
use super::window::{LayerWindow, WindowRole};
use crate::services::{
    audio::{AudioService, AudioState},
    brightness::{BrightnessService, ControlKind},
};
use adw::prelude::*;
use gtk4_layer_shell::{Edge, LayerShell};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

const SLIDE_MS: u32 = 350;
const DISMISS_AFTER: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Volume,
    Backlight,
    Keyboard,
}
impl Kind {
    fn index(self) -> usize {
        match self {
            Self::Volume => 0,
            Self::Backlight => 1,
            Self::Keyboard => 2,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Hidden,
    Showing,
    Visible,
    Hiding,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct AudioLevel {
    id: u32,
    serial: u64,
    volume: f64,
    muted: bool,
}
impl AudioLevel {
    fn from_state(state: &AudioState) -> Option<Self> {
        if !state.available {
            return None;
        }
        let node = state.default_sink.and_then(|id| state.node(id))?;
        let volume = node.volume.as_ref()?;
        volume.volume.is_finite().then_some(Self {
            id: node.id,
            serial: node.serial,
            volume: volume.volume,
            muted: volume.mute,
        })
    }
    fn same_device(self, other: Self) -> bool {
        (self.id, self.serial) == (other.id, other.serial)
    }
    fn changed_from(self, previous: Option<Self>) -> bool {
        previous.is_some_and(|previous| self.same_device(previous) && self != previous)
    }
}
fn volume_icon(volume: f64, muted: bool) -> &'static str {
    if muted || volume <= 0.0 {
        "audio-volume-muted-symbolic"
    } else if volume < 0.25 {
        "audio-volume-low-symbolic"
    } else if volume < 0.5 {
        "audio-volume-medium-symbolic"
    } else {
        "audio-volume-high-symbolic"
    }
}

struct Row {
    widget: gtk::Box,
    icon: gtk::Image,
    scale: gtk::Scale,
}
impl Row {
    fn new(icon_name: &str, maximum: f64, step: f64) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        widget.set_widget_name("osd-container");
        widget.set_visible(false);
        let icon = gtk::Image::from_icon_name(icon_name);
        icon.set_pixel_size(32);
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, maximum, step);
        scale.set_hexpand(true);
        scale.set_sensitive(false);
        widget.append(&icon);
        widget.append(&scale);
        Self {
            widget,
            icon,
            scale,
        }
    }
}

pub struct LevelOsd {
    surface: LayerWindow,
    rows: [Row; 3],
    audio: AudioService,
    brightness: BrightnessService,
    suppressed: Box<dyn Fn() -> bool>,
    subscriptions: RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>,
    last_audio: Cell<Option<AudioLevel>>,
    active: Cell<Option<Kind>>,
    phase: Cell<Phase>,
    animation: RefCell<Option<adw::TimedAnimation>>,
    timeout: RefCell<Option<glib::SourceId>>,
    revision: Cell<u64>,
    closed: Cell<bool>,
}
impl LevelOsd {
    /// The visibility query should capture the quick-settings controller weakly.
    /// Shared services remain active when this presentation owner is closed.
    pub fn new(
        audio: AudioService,
        brightness: BrightnessService,
        suppressed: impl Fn() -> bool + 'static,
    ) -> Result<Rc<Self>, String> {
        let surface = LayerWindow::new(WindowRole::LevelOsd, None)?;
        surface.window().set_widget_name("osd");
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
        let keyboard_max = brightness
            .snapshot(ControlKind::Keyboard)
            .map_or(1, |device| device.maximum.max(1));
        let rows = [
            Row::new("audio-volume-high-symbolic", 1.0, 0.05),
            Row::new("display-brightness-symbolic", 1.0, 0.05),
            Row::new("keyboard-brightness-symbolic", f64::from(keyboard_max), 1.0),
        ];
        for row in &rows {
            overlay.add_overlay(&row.widget);
        }
        surface.window().set_content(Some(&overlay));
        let last_audio = Cell::new(AudioLevel::from_state(&audio.state()));
        let this = Rc::new(Self {
            surface,
            rows,
            audio,
            brightness,
            suppressed: Box::new(suppressed),
            subscriptions: RefCell::new(Vec::new()),
            last_audio,
            active: Cell::new(None),
            phase: Cell::new(Phase::Hidden),
            animation: RefCell::new(None),
            timeout: RefCell::new(None),
            revision: Cell::new(0),
            closed: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        let handler = this.audio.connect_local("changed", false, move |_| {
            if let Some(this) = weak.upgrade() {
                this.audio_changed();
            }
            None
        });
        this.subscriptions
            .borrow_mut()
            .push((this.audio.clone().upcast(), handler));
        for (signal, kind) in [
            ("brightness-changed", Kind::Backlight),
            ("keyboard-brightness-changed", Kind::Keyboard),
        ] {
            let weak = Rc::downgrade(&this);
            let handler = this.brightness.connect_local(signal, false, move |values| {
                if let Some(this) = weak.upgrade() {
                    let value = match kind {
                        Kind::Backlight => f64::from(values[1].get::<f32>().unwrap()),
                        _ => f64::from(values[1].get::<u32>().unwrap()),
                    };
                    let control = if kind == Kind::Backlight {
                        ControlKind::Backlight
                    } else {
                        ControlKind::Keyboard
                    };
                    if this.brightness.is_available(control) {
                        this.show(kind, value, None);
                    }
                }
                None
            });
            this.subscriptions
                .borrow_mut()
                .push((this.brightness.clone().upcast(), handler));
        }
        let weak = Rc::downgrade(&this);
        let handler = this
            .brightness
            .connect_local("availability-changed", false, move |_| {
                if let Some(this) = weak.upgrade() {
                    this.availability_changed();
                }
                None
            });
        this.subscriptions
            .borrow_mut()
            .push((this.brightness.clone().upcast(), handler));
        let weak = Rc::downgrade(&this);
        this.surface.on_closed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.close();
            }
        });
        Ok(this)
    }
    pub fn window(&self) -> &adw::Window {
        self.surface.window()
    }
    pub fn hide(&self) {
        self.next_revision();
        self.active.set(None);
        self.phase.set(Phase::Hidden);
        self.cancel_pending();
        self.surface.hide();
    }
    pub fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.hide();
        for (object, handler) in self.subscriptions.take() {
            object.disconnect(handler);
        }
        self.surface.close();
    }
    fn next_revision(&self) -> u64 {
        let revision = self.revision.get().wrapping_add(1);
        self.revision.set(revision);
        revision
    }
    fn current(&self, revision: u64) -> bool {
        !self.closed.get() && self.revision.get() == revision
    }
    fn cancel_pending(&self) {
        let timeout = self.timeout.borrow_mut().take();
        if let Some(timeout) = timeout {
            timeout.remove();
        }
        let animation = self.animation.borrow_mut().take();
        if let Some(animation) = animation {
            animation.pause();
        }
    }
    fn audio_changed(self: &Rc<Self>) {
        if self.closed.get() {
            return;
        }
        let current = AudioLevel::from_state(&self.audio.state());
        let previous = self.last_audio.replace(current);
        if self.active.get() == Some(Kind::Volume)
            && !current
                .zip(previous)
                .is_some_and(|(current, previous)| current.same_device(previous))
        {
            self.hide();
        }
        if let Some(current) = current.filter(|current| current.changed_from(previous)) {
            self.show(
                Kind::Volume,
                if current.muted { 0.0 } else { current.volume },
                Some(volume_icon(current.volume, current.muted)),
            );
        }
    }
    fn availability_changed(&self) {
        if self.closed.get() {
            return;
        }
        let unavailable = match self.active.get() {
            Some(Kind::Backlight) => !self.brightness.is_available(ControlKind::Backlight),
            Some(Kind::Keyboard) => !self.brightness.is_available(ControlKind::Keyboard),
            _ => false,
        };
        if unavailable {
            self.hide();
        }
        if self.closed.get() {
            return;
        }
        let maximum = self
            .brightness
            .snapshot(ControlKind::Keyboard)
            .map_or(1, |device| device.maximum.max(1));
        self.rows[Kind::Keyboard.index()]
            .scale
            .set_range(0.0, f64::from(maximum));
    }
    fn show(self: &Rc<Self>, kind: Kind, value: f64, icon: Option<&str>) {
        if self.closed.get() || !value.is_finite() {
            return;
        }
        // GTK property observers may hide, close, or show a newer value during
        // any mutation below. Never resume an obsolete presentation afterward.
        let observed = self.revision.get();
        if (self.suppressed)() || !self.current(observed) {
            return;
        }
        let revision = self.next_revision();
        let phase = self.phase.get();
        self.cancel_pending();
        self.active.set(Some(kind));
        let selected = &self.rows[kind.index()];
        if let Some(icon) = icon {
            selected.icon.set_icon_name(Some(icon));
        }
        if !self.current(revision) {
            return;
        }
        selected.scale.set_value(value);
        if !self.current(revision) {
            return;
        }
        for (index, row) in self.rows.iter().enumerate() {
            row.widget.set_visible(index == kind.index());
            if !self.current(revision) {
                return;
            }
        }
        if phase == Phase::Hidden {
            self.window().set_margin(Edge::Bottom, 0);
        }
        if !self.current(revision) {
            return;
        }
        if !self.surface.present() {
            self.close();
            return;
        }
        if !self.current(revision) {
            return;
        }
        if phase == Phase::Visible {
            self.arm_timeout(revision);
        } else {
            self.animate(true, revision);
        }
    }
    fn animate(self: &Rc<Self>, showing: bool, revision: u64) {
        self.phase.set(if showing {
            Phase::Showing
        } else {
            Phase::Hiding
        });
        let weak = Rc::downgrade(self);
        let target = adw::CallbackAnimationTarget::new(move |value| {
            if let Some(this) = weak.upgrade().filter(|this| this.current(revision)) {
                this.window().set_margin(Edge::Bottom, value as i32);
            }
        });
        let animation = adw::TimedAnimation::new(
            self.window(),
            f64::from(self.window().margin(Edge::Bottom)),
            if showing { 120.0 } else { 0.0 },
            SLIDE_MS,
            target,
        );
        let weak = Rc::downgrade(self);
        animation.connect_done(move |_| {
            if let Some(this) = weak.upgrade().filter(|this| this.current(revision)) {
                this.animation.borrow_mut().take();
                if showing {
                    this.phase.set(Phase::Visible);
                    this.arm_timeout(revision);
                } else {
                    this.hide();
                }
            }
        });
        if self.current(revision) {
            self.animation.replace(Some(animation.clone()));
            animation.play();
        }
    }
    fn arm_timeout(self: &Rc<Self>, revision: u64) {
        let weak = Rc::downgrade(self);
        let timeout = glib::timeout_add_local_once(DISMISS_AFTER, move || {
            if let Some(this) = weak.upgrade().filter(|this| this.current(revision)) {
                this.timeout.borrow_mut().take();
                this.animate(false, revision);
            }
        });
        self.timeout.replace(Some(timeout));
    }
}
impl Drop for LevelOsd {
    fn drop(&mut self) {
        self.closed.set(true);
        self.cancel_pending();
        for (object, handler) in self.subscriptions.get_mut().drain(..) {
            object.disconnect(handler);
        }
        self.surface.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_icons_preserve_thresholds_and_zero_shows_muted() {
        for (volume, muted, expected) in [
            (0.0, false, "audio-volume-muted-symbolic"),
            (0.249, false, "audio-volume-low-symbolic"),
            (0.25, false, "audio-volume-medium-symbolic"),
            (0.5, false, "audio-volume-high-symbolic"),
            (1.5, false, "audio-volume-high-symbolic"),
            (0.9, true, "audio-volume-muted-symbolic"),
        ] {
            assert_eq!(volume_icon(volume, muted), expected);
        }
    }
    #[test]
    fn initial_defaults_and_recycled_node_ids_stay_quiet() {
        let previous = AudioLevel {
            id: 4,
            serial: 10,
            volume: 0.5,
            muted: false,
        };
        assert!(!previous.changed_from(None));
        assert!(!previous.changed_from(Some(previous)));
        assert!(!AudioLevel { id: 5, ..previous }.changed_from(Some(previous)));
        assert!(
            !AudioLevel {
                serial: 11,
                ..previous
            }
            .changed_from(Some(previous))
        );
        assert!(
            AudioLevel {
                volume: 0.6,
                ..previous
            }
            .changed_from(Some(previous))
        );
        assert!(
            AudioLevel {
                muted: true,
                ..previous
            }
            .changed_from(Some(previous))
        );
    }
}
