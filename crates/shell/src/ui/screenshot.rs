//! One cancellable frozen desktop session, owned by Desktop.
use crate::{
    services::{notifications::NotificationsService, screenshot as capture, wm::WindowManager},
    ui::window::{LayerWindow, WindowRole},
};
use adw::prelude::*;
use gtk::{gdk, gdk_pixbuf::Pixbuf};
use gtk4_layer_shell::{KeyboardMode, LayerShell};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};
use way_shell_core::{
    capture::{CaptureLayout, Mode, Rect, RgbaImage, Selection, compose, dimensions},
    notifications::{Hints, NotificationRequest},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Preparing,
    Preview,
    Exporting,
}
#[derive(Default)]
struct Lifecycle {
    phase: Option<Phase>,
    generation: u64,
}
impl Lifecycle {
    fn begin(&mut self) -> Option<u64> {
        if self.phase.is_some() {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        self.phase = Some(Phase::Preparing);
        Some(self.generation)
    }
    fn current(&self, id: u64) -> bool {
        self.phase.is_some() && self.generation == id
    }
    fn transition(&mut self, id: u64, from: Phase, to: Phase) -> bool {
        if self.current(id) && self.phase == Some(from) {
            self.phase = Some(to);
            true
        } else {
            false
        }
    }
    fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.phase = None;
    }
}
struct Preview {
    layout: CaptureLayout,
    images: Vec<RgbaImage>,
    selection: Selection,
    launch: usize,
    drag: Option<Drag>,
}
#[derive(Clone, Copy)]
struct Drag {
    start: (f64, f64),
    area: Rect,
    edges: (bool, bool, bool, bool),
    moving: bool,
}
struct Surface {
    layer: LayerWindow,
    canvas: gtk::DrawingArea,
}
struct Controls {
    label: gtk::Label,
    status: gtk::Label,
    capture: gtk::Button,
    modes: Vec<(Mode, gtk::ToggleButton)>,
    root: gtk::Box,
}
type ActiveCallback = Box<dyn Fn(bool)>;
pub struct Screenshot {
    manager: WindowManager,
    settings: gio::Settings,
    notifications: NotificationsService,
    display: gdk::Display,
    lifecycle: RefCell<Lifecycle>,
    preview: RefCell<Option<Preview>>,
    surfaces: RefCell<Vec<Surface>>,
    controls: RefCell<Option<Controls>>,
    task: RefCell<Option<glib::JoinHandle<()>>>,
    watcher: RefCell<Option<glib::JoinHandle<()>>>,
    monitor_handler: RefCell<Option<glib::SignalHandlerId>>,
    active: RefCell<Option<ActiveCallback>>,
    stopped: Cell<bool>,
    dismissed: Cell<bool>,
}
impl Screenshot {
    pub fn new(
        manager: WindowManager,
        settings: gio::Settings,
        notifications: NotificationsService,
        display: &gdk::Display,
    ) -> Rc<Self> {
        let this = Rc::new(Self {
            manager,
            settings,
            notifications,
            display: display.clone(),
            lifecycle: RefCell::new(Lifecycle::default()),
            preview: RefCell::new(None),
            surfaces: RefCell::new(Vec::new()),
            controls: RefCell::new(None),
            task: RefCell::new(None),
            watcher: RefCell::new(None),
            monitor_handler: RefCell::new(None),
            active: RefCell::new(None),
            stopped: Cell::new(false),
            dismissed: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        let handler = display.monitors().connect_items_changed(move |_, _, _, _| {
            if let Some(this) = weak.upgrade().filter(|t| t.is_active()) {
                this.fail("Display layout changed; reopen Screenshot to capture the new layout");
            }
        });
        this.monitor_handler.replace(Some(handler));
        this
    }
    pub fn on_active(&self, callback: impl Fn(bool) + 'static) {
        self.active.replace(Some(Box::new(callback)));
    }
    pub fn is_active(&self) -> bool {
        self.lifecycle.borrow().phase.is_some()
    }
    fn current(&self, id: u64) -> bool {
        !self.stopped.get() && self.lifecycle.borrow().current(id)
    }
    pub fn show(self: &Rc<Self>, connector: Option<String>) -> Result<(), String> {
        if self.stopped.get() {
            return Err("Screenshot controller has stopped".into());
        }
        if self.is_active() {
            return Ok(());
        }
        if !capture::available() {
            return Err("Install grim to take screenshots".into());
        }
        let Some(id) = self.lifecycle.borrow_mut().begin() else {
            return Ok(());
        };
        if let Some(callback) = self.active.borrow().as_ref() {
            callback(true);
        }
        let weak = Rc::downgrade(self);
        let manager = self.manager.clone();
        let display = self.display.clone();
        let task = glib::spawn_future_local(async move {
            let prepared = glib::future_with_timeout(Duration::from_secs(10), async {
                // Flush unmaps from Desktop before querying or acquiring any image.
                // GDK's Wayland roundtrip is a compositor barrier on GTK's connection.
                display.sync();
                let layout = manager.capture_layout().await?;
                let images = capture::acquire(&layout).await?;
                let after = manager.capture_layout().await?;
                if after.outputs != layout.outputs {
                    return Err("Display layout changed during capture; reopen Screenshot".into());
                }
                Ok::<_, String>((layout, images))
            })
            .await
            .unwrap_or_else(|_| Err("Screenshot preparation exceeded ten seconds".into()));
            if let Some(this) = weak.upgrade().filter(|t| t.current(id)) {
                this.task.borrow_mut().take();
                match prepared {
                    Ok((layout, images)) => {
                        let launch = layout.launching_output(connector.as_deref());
                        let selection = Selection::new(&layout, launch);
                        this.preview.replace(Some(Preview {
                            layout,
                            images,
                            selection,
                            launch,
                            drag: None,
                        }));
                        if let Err(error) = this.build_surfaces() {
                            this.fail(&error);
                            return;
                        }
                        this.lifecycle.borrow_mut().transition(
                            id,
                            Phase::Preparing,
                            Phase::Preview,
                        );
                        this.refresh();
                        this.watch_layout(id);
                    }
                    Err(error) => this.fail(&error),
                }
            }
        });
        self.task.replace(Some(task));
        Ok(())
    }
    pub fn toggle(self: &Rc<Self>) -> Result<(), String> {
        if self.is_active() {
            self.cancel();
            Ok(())
        } else {
            self.show(None)
        }
    }
    pub fn cancel(&self) {
        // Capture commits an export. Hiding can release the overlay immediately,
        // but a second session must wait for the accepted file write to finish.
        if self.lifecycle.borrow().phase == Some(Phase::Exporting) {
            if !self.dismissed.replace(true) {
                self.controls.take();
                self.surfaces.take();
                self.preview.take();
                if let Some(task) = self.watcher.take() {
                    task.abort();
                }
                if let Some(callback) = self.active.borrow().as_ref() {
                    callback(false);
                }
            }
            return;
        }
        self.reset();
    }
    fn reset(&self) {
        let active = self.is_active() && !self.dismissed.replace(false);
        self.lifecycle.borrow_mut().cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
        if let Some(task) = self.watcher.take() {
            task.abort();
        }
        self.controls.take();
        let surfaces = self.surfaces.take();
        drop(surfaces);
        self.preview.take();
        if active && let Some(callback) = self.active.borrow().as_ref() {
            callback(false);
        }
    }
    pub fn close(&self) {
        if self.stopped.replace(true) {
            return;
        }
        self.reset();
        if let Some(handler) = self.monitor_handler.take() {
            self.display.monitors().disconnect(handler);
        }
        self.active.take();
    }
    fn notify(&self, title: &str, body: &str, path: Option<&std::path::Path>) {
        let image_path = path.map(|path| gio::File::for_path(path).uri().to_string());
        let _ = self.notifications.send_internal(NotificationRequest {
            app_name: "Screenshot".into(),
            app_icon: "camera-photo-symbolic".into(),
            summary: title.into(),
            body: glib::markup_escape_text(body).into(),
            hints: Hints {
                image_path,
                ..Hints::default()
            },
            ..NotificationRequest::default()
        });
    }
    fn fail(&self, error: &str) {
        glib::g_message!("way-shell", "Screenshot: {error}");
        self.cancel();
        self.notify("Screenshot unavailable", error, None);
    }
    fn watch_layout(self: &Rc<Self>, id: u64) {
        let weak = Rc::downgrade(self);
        let manager = self.manager.clone();
        let outputs = self
            .preview
            .borrow()
            .as_ref()
            .unwrap()
            .layout
            .outputs
            .clone();
        let task = glib::spawn_future_local(async move {
            loop {
                glib::timeout_future(Duration::from_millis(500)).await;
                if !weak.upgrade().is_some_and(|t| t.current(id)) {
                    return;
                }
                let result =
                    glib::future_with_timeout(Duration::from_secs(2), manager.capture_layout())
                        .await;
                let valid = matches!(result,Ok(Ok(ref layout)) if layout.outputs==outputs);
                if !valid {
                    if let Some(this) = weak.upgrade().filter(|t| t.current(id)) {
                        this.watcher.borrow_mut().take();
                        this.fail(
                            "Display layout changed or became unavailable; reopen Screenshot",
                        );
                    }
                    return;
                }
            }
        });
        self.watcher.replace(Some(task));
    }
    fn build_surfaces(self: &Rc<Self>) -> Result<(), String> {
        let preview_guard = self.preview.borrow();
        let preview = preview_guard.as_ref().ok_or("Missing screenshot preview")?;
        let monitors = self.display.monitors();
        let monitors: Vec<_> = (0..monitors.n_items())
            .filter_map(|i| monitors.item(i).and_downcast::<gdk::Monitor>())
            .collect();
        let mut surfaces = Vec::new();
        for (index, output) in preview.layout.outputs.iter().enumerate() {
            let monitor = monitors
                .iter()
                .find(|m| m.connector().as_deref() == Some(output.connector.as_str()))
                .ok_or_else(|| format!("GTK cannot find display {}", output.connector))?;
            let layer = LayerWindow::new(WindowRole::Screenshot, Some(monitor))?;
            layer.window().add_css_class("screenshot-overlay");
            let overlay = gtk::Overlay::new();
            let canvas = gtk::DrawingArea::builder()
                .hexpand(true)
                .vexpand(true)
                .focusable(true)
                .build();
            canvas.update_property(&[gtk::accessible::Property::Label(
                "Screenshot selection canvas",
            )]);
            canvas.set_cursor_from_name(Some("crosshair"));
            let pixels = capture::pixbuf(&preview.images[index]);
            let weak = Rc::downgrade(self);
            canvas.set_draw_func(move |_, cr, width, height| {
                if let Some(this) = weak.upgrade() {
                    this.draw(index, &pixels, cr, width, height);
                }
            });
            overlay.set_child(Some(&canvas));
            if index == preview.launch {
                layer.window().set_keyboard_mode(KeyboardMode::Exclusive);
                let controls = self.build_controls(preview.layout.windows.is_empty());
                overlay.add_overlay(&controls.root);
                self.controls.replace(Some(controls));
                self.install_keys(layer.window());
            }
            self.install_gestures(index, &canvas);
            layer.window().set_content(Some(&overlay));
            let weak = Rc::downgrade(self);
            layer.on_closed(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.fail("Screenshot display was closed; reopen Screenshot");
                }
            });
            surfaces.push(Surface { layer, canvas });
        }
        let launch = preview.launch;
        drop(preview_guard);
        self.surfaces.replace(surfaces);
        for (i, surface) in self.surfaces.borrow().iter().enumerate() {
            if i != launch {
                surface.layer.present();
            }
        }
        let surfaces = self.surfaces.borrow();
        surfaces[launch].layer.present();
        surfaces[launch].canvas.grab_focus();
        Ok(())
    }
    fn build_controls(self: &Rc<Self>, no_windows: bool) -> Controls {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("screenshot-toolbar");
        root.set_halign(gtk::Align::Center);
        root.set_valign(gtk::Align::End);
        root.set_margin_bottom(32);
        root.set_margin_start(16);
        root.set_margin_end(16);
        let modes_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        modes_box.add_css_class("linked");
        let mut modes = Vec::new();
        for (mode, label, icon, key) in [
            (Mode::Area, "Area", "edit-select-all-symbolic", "S"),
            (Mode::Window, "Window", "focus-windows-symbolic", "W"),
            (Mode::Screen, "Screen", "video-display-symbolic", "C"),
            (Mode::All, "All screens", "view-grid-symbolic", "A"),
        ] {
            let button = gtk::ToggleButton::new();
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            content.append(&gtk::Image::from_icon_name(icon));
            content.append(&gtk::Label::new(Some(label)));
            button.set_child(Some(&content));
            button.set_tooltip_text(Some(&format!("{label} ({key})")));
            if mode == Mode::Window && no_windows {
                button.set_sensitive(false);
                button.set_tooltip_text(Some("Window geometry is unavailable; use Area or Screen"));
            }
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.set_mode(mode);
                }
            });
            modes_box.append(&button);
            modes.push((mode, button));
        }
        root.append(&modes_box);
        let label = gtk::Label::new(None);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(60);
        root.append(&label);
        let status = gtk::Label::new(None);
        status.set_wrap(true);
        status.set_max_width_chars(60);
        status.add_css_class("screenshot-status");
        root.append(&status);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_halign(gtk::Align::Center);
        let cancel = gtk::Button::with_label("Cancel");
        let capture = gtk::Button::with_label("Capture");
        capture.add_css_class("suggested-action");
        let weak = Rc::downgrade(self);
        cancel.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.cancel();
            }
        });
        let weak = Rc::downgrade(self);
        capture.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.capture();
            }
        });
        actions.append(&cancel);
        actions.append(&capture);
        root.append(&actions);
        let help = gtk::Label::new(Some("S/W/C/A: mode · Enter: capture · Esc: cancel"));
        help.add_css_class("dim-label");
        root.append(&help);
        Controls {
            root,
            label,
            status,
            capture,
            modes,
        }
    }
    fn set_mode(&self, mode: Mode) {
        if self.lifecycle.borrow().phase != Some(Phase::Preview) {
            return;
        }
        if let Some(p) = self.preview.borrow_mut().as_mut() {
            if mode == Mode::Window && p.layout.windows.is_empty() {
                return;
            }
            p.selection.mode = mode;
            p.drag = None;
        }
        self.refresh();
    }
    fn refresh(&self) {
        if let Some(preview) = self.preview.borrow().as_ref()
            && let Some(controls) = self.controls.borrow().as_ref()
        {
            let exporting = self.lifecycle.borrow().phase == Some(Phase::Exporting);
            for (mode, button) in &controls.modes {
                button.set_active(*mode == preview.selection.mode);
                button.set_sensitive(
                    !exporting && (*mode != Mode::Window || !preview.layout.windows.is_empty()),
                );
            }
            let size = dimensions(&preview.layout, &preview.selection);
            controls.capture.set_sensitive(!exporting && size.is_ok());
            if let Ok((width, height, _)) = size {
                let name = match preview.selection.mode {
                    Mode::Area => "Area",
                    Mode::Window => &preview.layout.windows[preview.selection.window].title,
                    Mode::Screen => &preview.layout.outputs[preview.selection.output].connector,
                    Mode::All => "All screens",
                };
                controls
                    .label
                    .set_text(&format!("{name} · {width} × {height}"));
            } else {
                controls
                    .label
                    .set_text("Select a region that intersects a display");
            }
        }
        for surface in self.surfaces.borrow().iter() {
            surface.canvas.queue_draw();
        }
    }
    fn draw(
        &self,
        index: usize,
        pixels: &Pixbuf,
        cr: &gtk::cairo::Context,
        width: i32,
        height: i32,
    ) {
        let preview = self.preview.borrow();
        let Some(p) = preview.as_ref() else {
            return;
        };
        let output = &p.layout.outputs[index];
        let _ = cr.save();
        cr.scale(
            width as f64 / pixels.width() as f64,
            height as f64 / pixels.height() as f64,
        );
        cr.set_source_pixbuf(pixels, 0.0, 0.0);
        let _ = cr.paint();
        let _ = cr.restore();
        let _ = cr.save();
        cr.scale(
            width as f64 / output.bounds.width,
            height as f64 / output.bounds.height,
        );
        cr.translate(-output.bounds.x, -output.bounds.y);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.42);
        cr.rectangle(
            output.bounds.x,
            output.bounds.y,
            output.bounds.width,
            output.bounds.height,
        );
        if let Some(bounds) = p.selection.bounds(&p.layout) {
            cr.rectangle(bounds.x, bounds.y, bounds.width, bounds.height);
            cr.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
        }
        let _ = cr.fill();
        if p.selection.mode == Mode::Window {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.45);
            cr.set_line_width(1.0);
            for window in &p.layout.windows {
                cr.rectangle(
                    window.bounds.x + 0.5,
                    window.bounds.y + 0.5,
                    window.bounds.width - 1.0,
                    window.bounds.height - 1.0,
                );
                let _ = cr.stroke();
            }
        }
        if let Some(bounds) = p.selection.bounds(&p.layout) {
            cr.set_source_rgb(0.35, 0.72, 1.0);
            cr.set_line_width(2.0);
            cr.rectangle(
                bounds.x + 1.0,
                bounds.y + 1.0,
                bounds.width - 2.0,
                bounds.height - 2.0,
            );
            let _ = cr.stroke();
            if p.selection.mode == Mode::Area {
                for x in [bounds.x, bounds.x + bounds.width / 2.0, bounds.right()] {
                    for y in [bounds.y, bounds.y + bounds.height / 2.0, bounds.bottom()] {
                        if x == bounds.x + bounds.width / 2.0 && y == bounds.y + bounds.height / 2.0
                        {
                            continue;
                        }
                        cr.rectangle(x - 4.0, y - 4.0, 8.0, 8.0);
                        let _ = cr.fill();
                    }
                }
            }
        }
        let _ = cr.restore();
    }
    fn install_keys(self: &Rc<Self>, window: &adw::Window) {
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(this) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            use gdk::Key;
            match key {
                Key::Escape => this.cancel(),
                Key::s | Key::S => this.set_mode(Mode::Area),
                Key::w | Key::W => this.set_mode(Mode::Window),
                Key::c | Key::C => this.set_mode(Mode::Screen),
                Key::a | Key::A => this.set_mode(Mode::All),
                Key::Return | Key::KP_Enter | Key::space => this.capture(),
                Key::Left | Key::Right | Key::Up | Key::Down => {
                    let canvas_focused =
                        this.surfaces.borrow().iter().any(|s| s.canvas.has_focus());
                    if !canvas_focused || this.lifecycle.borrow().phase != Some(Phase::Preview) {
                        return glib::Propagation::Proceed;
                    }
                    let step = if modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
                        10.0
                    } else {
                        1.0
                    };
                    let (dx, dy) = match key {
                        Key::Left => (-step, 0.0),
                        Key::Right => (step, 0.0),
                        Key::Up => (0.0, -step),
                        _ => (0.0, step),
                    };
                    if let Some(p) = this.preview.borrow_mut().as_mut() {
                        p.selection.adjust(
                            &p.layout,
                            dx,
                            dy,
                            modifiers.contains(gdk::ModifierType::SHIFT_MASK),
                        );
                    }
                    this.refresh();
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        window.add_controller(keys);
    }
    fn install_gestures(self: &Rc<Self>, index: usize, canvas: &gtk::DrawingArea) {
        let gesture = gtk::GestureDrag::new();
        gesture.set_button(1);
        let weak = Rc::downgrade(self);
        gesture.connect_drag_begin(move |gesture, x, y| {
            let Some(this) = weak.upgrade() else {
                return;
            };
            if this.lifecycle.borrow().phase != Some(Phase::Preview) {
                return;
            }
            let mut preview = this.preview.borrow_mut();
            let Some(p) = preview.as_mut() else {
                return;
            };
            let b = p.layout.outputs[index].bounds;
            let x = b.x + x;
            let y = b.y + y;
            if p.selection.mode == Mode::Area {
                let area = p.selection.area;
                let near = 8.0;
                let horizontal = y >= area.y - near && y <= area.bottom() + near;
                let vertical = x >= area.x - near && x <= area.right() + near;
                let edges = (
                    (x - area.x).abs() <= near && horizontal,
                    (x - area.right()).abs() <= near && horizontal,
                    (y - area.y).abs() <= near && vertical,
                    (y - area.bottom()).abs() <= near && vertical,
                );
                let resizing = edges.0 || edges.1 || edges.2 || edges.3;
                let moving = !resizing && area.contains(x, y);
                p.drag = Some(Drag {
                    start: (x, y),
                    area,
                    edges,
                    moving,
                });
                if !resizing && !moving {
                    p.selection.area =
                        Rect::from_points(x, y, x + 1.0, y + 1.0).clamped(p.layout.bounds());
                }
            } else {
                p.selection.click(&p.layout, x, y);
            }
            let launch = p.launch;
            drop(preview);
            if let Some(surface) = this.surfaces.borrow().get(launch) {
                surface.canvas.grab_focus();
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            this.refresh();
        });
        let weak = Rc::downgrade(self);
        gesture.connect_drag_update(move |_, dx, dy| {
            if let Some(this) = weak.upgrade() {
                this.drag(dx, dy);
            }
        });
        let weak = Rc::downgrade(self);
        gesture.connect_drag_end(move |_, dx, dy| {
            if let Some(this) = weak.upgrade() {
                this.drag(dx, dy);
                if let Some(p) = this.preview.borrow_mut().as_mut() {
                    p.drag = None;
                }
            }
        });
        canvas.add_controller(gesture);
    }
    fn drag(&self, dx: f64, dy: f64) {
        if self.lifecycle.borrow().phase != Some(Phase::Preview) {
            return;
        }
        if let Some(p) = self.preview.borrow_mut().as_mut()
            && let Some(drag) = p.drag
        {
            let area = if drag.moving {
                Rect {
                    x: drag.area.x + dx,
                    y: drag.area.y + dy,
                    ..drag.area
                }
            } else if drag.edges.0 || drag.edges.1 || drag.edges.2 || drag.edges.3 {
                Rect::from_points(
                    if drag.edges.0 {
                        drag.area.x + dx
                    } else {
                        drag.area.x
                    },
                    if drag.edges.2 {
                        drag.area.y + dy
                    } else {
                        drag.area.y
                    },
                    if drag.edges.1 {
                        drag.area.right() + dx
                    } else {
                        drag.area.right()
                    },
                    if drag.edges.3 {
                        drag.area.bottom() + dy
                    } else {
                        drag.area.bottom()
                    },
                )
            } else {
                Rect::from_points(
                    drag.start.0,
                    drag.start.1,
                    drag.start.0 + dx,
                    drag.start.1 + dy,
                )
            };
            p.selection.area = area.clamped(p.layout.bounds());
        }
        self.refresh();
    }
    fn capture(self: &Rc<Self>) {
        let id = self.lifecycle.borrow().generation;
        if !self
            .lifecycle
            .borrow_mut()
            .transition(id, Phase::Preview, Phase::Exporting)
        {
            return;
        }
        // Read both keys for every attempt; failed saves can be retried after a settings change.
        let directory = capture::destination(
            &self.settings.string("screenshot-save-directory"),
            glib::user_special_dir(glib::UserDirectory::Pictures).as_deref(),
            &glib::home_dir(),
        );
        let copy = self.settings.boolean("screenshot-copy-to-clipboard");
        let (layout, images, selection) = {
            let preview = self.preview.borrow();
            let p = preview.as_ref().unwrap();
            (p.layout.clone(), p.images.clone(), p.selection.clone())
        };
        self.refresh();
        self.status("Saving…");
        let clipboard = self.display.clipboard();
        let weak = Rc::downgrade(self);
        let stamp = glib::DateTime::now_local()
            .and_then(|time| time.format("%Y-%m-%d-%H%M%S"))
            .map(|s| s.to_string())
            .unwrap_or_else(|_| glib::real_time().to_string());
        let task = glib::spawn_future_local(async move {
            let prepared = gio::spawn_blocking(move || {
                compose(&layout, &images, &selection).and_then(|image| capture::encode_png(&image))
            })
            .await
            .map_err(|_| "Screenshot image worker stopped".to_owned())
            .and_then(|r| r);
            let Some(this) = weak.upgrade().filter(|t| t.current(id)) else {
                return;
            };
            let png = match prepared {
                Ok(png) => png,
                Err(error) => {
                    this.task.borrow_mut().take();
                    this.export_failed(id, &error);
                    return;
                }
            };
            let copied = if copy {
                clipboard
                    .set_content(Some(&gdk::ContentProvider::for_bytes(
                        "image/png",
                        &glib::Bytes::from_owned(png.clone()),
                    )))
                    .map(|()| true)
                    .map_err(|e| e.to_string())
            } else {
                Ok(false)
            };
            drop(this);
            let saved = gio::spawn_blocking(move || {
                directory.and_then(|directory| capture::save_png(&directory, &stamp, &png))
            })
            .await
            .map_err(|_| "Screenshot file worker stopped".to_owned())
            .and_then(|r| r);
            if let Some(this) = weak.upgrade().filter(|t| t.current(id)) {
                this.task.borrow_mut().take();
                match saved {
                    Ok(path) => {
                        this.reset();
                        let body = match &copied {
                            Ok(true) => {
                                format!("Saved to {} and copied to clipboard", path.display())
                            }
                            Ok(false) => format!("Saved to {}", path.display()),
                            Err(_) => format!("Saved to {}; clipboard copy failed", path.display()),
                        };
                        this.notify("Screenshot saved", &body, Some(&path));
                        if let Err(error) = copied {
                            this.notify("Screenshot clipboard copy failed", &error, None);
                        }
                    }
                    Err(error) => {
                        let status = match copied {
                            Ok(true) => format!("Copied to clipboard. {error}"),
                            Ok(false) => error,
                            Err(copy_error) => {
                                format!("Clipboard copy failed: {copy_error}. {error}")
                            }
                        };
                        this.export_failed(id, &status);
                    }
                }
            }
        });
        self.task.replace(Some(task));
    }
    fn status(&self, message: &str) {
        if let Some(c) = self.controls.borrow().as_ref() {
            c.status.set_text(message);
        }
    }
    fn export_failed(&self, id: u64, error: &str) {
        if self.dismissed.get() {
            self.reset();
            self.notify("Screenshot could not be saved", error, None);
            return;
        }
        self.lifecycle
            .borrow_mut()
            .transition(id, Phase::Exporting, Phase::Preview);
        self.status(error);
        self.refresh();
    }
}
impl Drop for Screenshot {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_repeated_requests_and_stale_completions() {
        let mut state = Lifecycle::default();
        let first = state.begin().unwrap();
        assert!(state.begin().is_none());
        state.cancel();
        assert!(!state.transition(first, Phase::Preparing, Phase::Preview));
        let second = state.begin().unwrap();
        assert_ne!(first, second);
        assert!(!state.current(first));
        assert!(state.transition(second, Phase::Preparing, Phase::Preview));
        assert!(state.transition(second, Phase::Preview, Phase::Exporting));
        assert!(!state.transition(second, Phase::Preview, Phase::Exporting));
        assert!(state.transition(second, Phase::Exporting, Phase::Preview));
        state.cancel();
        assert!(!state.current(second));
    }
}

#[cfg(test)]
#[path = "screenshot/live_tests.rs"]
mod live_tests;
