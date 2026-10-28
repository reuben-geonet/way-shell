//! One cancellable frozen desktop session, owned by Desktop.
use super::progress::Progress;
use crate::{
    services::{
        notifications::NotificationsService,
        screencast::{self, Audio, ScreencastService, Target},
        screenshot as capture,
        wm::WindowManager,
    },
    ui::window::{LayerWindow, WindowRole},
};
use gtk4::{gdk, gdk_pixbuf::Pixbuf};
use gtk4_layer_shell::{KeyboardMode, LayerShell};
use libadwaita::prelude::*;
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
impl Drag {
    fn new(area: Rect, x: f64, y: f64) -> Self {
        let near = 10.0;
        let horizontal = y >= area.y - near && y <= area.bottom() + near;
        let vertical = x >= area.x - near && x <= area.right() + near;
        let edges = (
            (x - area.x).abs() <= near && horizontal,
            (x - area.right()).abs() <= near && horizontal,
            (y - area.y).abs() <= near && vertical,
            (y - area.bottom()).abs() <= near && vertical,
        );
        Self {
            start: (x, y),
            area,
            edges,
            moving: !(edges.0 || edges.1 || edges.2 || edges.3) && area.contains(x, y),
        }
    }
    fn cursor(self, dragging: bool) -> &'static str {
        match self.edges {
            (true, _, true, _) | (_, true, _, true) => "nwse-resize",
            (true, _, _, true) | (_, true, true, _) => "nesw-resize",
            (true, _, _, _) | (_, true, _, _) => "ew-resize",
            (_, _, true, _) | (_, _, _, true) => "ns-resize",
            _ if self.moving && dragging => "grabbing",
            _ if self.moving => "grab",
            _ => "crosshair",
        }
    }
}
struct Surface {
    layer: LayerWindow,
    canvas: gtk4::DrawingArea,
    pointer: Cell<Option<(f64, f64)>>,
}
struct Controls {
    label: gtk4::Label,
    status: gtk4::Label,
    capture: gtk4::Button,
    modes: Vec<(Mode, gtk4::ToggleButton)>,
    photo: gtk4::ToggleButton,
    video: gtk4::ToggleButton,
    audio: gtk4::DropDown,
    audio_row: gtk4::Box,
    root: gtk4::Overlay,
}
type ActiveCallback = Box<dyn Fn(bool)>;
pub struct Screenshot {
    manager: WindowManager,
    settings: gio::Settings,
    notifications: NotificationsService,
    recording: ScreencastService,
    recording_handler: RefCell<Option<glib::SignalHandlerId>>,
    video: Cell<bool>,
    display: gdk::Display,
    lifecycle: RefCell<Lifecycle>,
    preview: RefCell<Option<Preview>>,
    last_area: Cell<Option<Rect>>,
    surfaces: RefCell<Vec<Surface>>,
    controls: RefCell<Option<Controls>>,
    task: RefCell<Option<glib::JoinHandle<()>>>,
    watcher: RefCell<Option<glib::JoinHandle<()>>>,
    monitor_handler: RefCell<Option<glib::SignalHandlerId>>,
    active: RefCell<Option<ActiveCallback>>,
    stopped: Cell<bool>,
    dismissed: Cell<bool>,
    progress: Progress,
    saved: Cell<bool>,
}
impl Screenshot {
    pub fn new(
        manager: WindowManager,
        settings: gio::Settings,
        notifications: NotificationsService,
        recording: ScreencastService,
        display: &gdk::Display,
    ) -> Rc<Self> {
        let this = Rc::new_cyclic(|weak: &std::rc::Weak<Self>| Self {
            manager,
            settings,
            notifications,
            recording,
            recording_handler: RefCell::new(None),
            video: Cell::new(false),
            display: display.clone(),
            lifecycle: RefCell::new(Lifecycle::default()),
            preview: RefCell::new(None),
            last_area: Cell::new(None),
            surfaces: RefCell::new(Vec::new()),
            controls: RefCell::new(None),
            task: RefCell::new(None),
            watcher: RefCell::new(None),
            monitor_handler: RefCell::new(None),
            active: RefCell::new(None),
            stopped: Cell::new(false),
            dismissed: Cell::new(false),
            progress: Progress::new({
                let weak = weak.clone();
                move |visible| {
                    if let Some(this) = weak.upgrade()
                        && this.lifecycle.borrow().phase == Some(Phase::Exporting)
                        && !this.dismissed.get()
                    {
                        this.status(if visible { "Saving…" } else { "" });
                    }
                }
            }),
            saved: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        let handler = display.monitors().connect_items_changed(move |_, _, _, _| {
            if let Some(this) = weak.upgrade().filter(|t| t.is_active()) {
                this.fail("Display layout changed; reopen Screenshot to capture the new layout");
            }
        });
        this.monitor_handler.replace(Some(handler));
        let weak = Rc::downgrade(&this);
        let handler = this.recording.connect_local("changed", false, move |_| {
            if let Some(this) = weak.upgrade() {
                this.refresh();
            }
            None
        });
        this.recording_handler.replace(Some(handler));
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
        self.video.set(false);
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
                        let selection = Selection::new(&layout, launch, this.last_area.get());
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
            if self.saved.get() {
                self.reset();
                return;
            }
            self.progress.clear();
            if !self.dismissed.replace(true) {
                self.controls.take();
                self.surfaces.take();
                self.clear_preview();
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
        self.progress.clear();
        self.saved.set(false);
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
        self.clear_preview();
        if active && let Some(callback) = self.active.borrow().as_ref() {
            callback(false);
        }
    }
    fn clear_preview(&self) {
        // Keep only the logical rectangle between sessions, never desktop images.
        if let Some(preview) = self.preview.take() {
            self.last_area.set(Some(preview.selection.area));
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
        if let Some(handler) = self.recording_handler.take() {
            self.recording.disconnect(handler);
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
            let overlay = gtk4::Overlay::new();
            let canvas = gtk4::DrawingArea::builder()
                .hexpand(true)
                .vexpand(true)
                .focusable(true)
                .build();
            canvas.update_property(&[gtk4::accessible::Property::Label(
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
            surfaces.push(Surface {
                layer,
                canvas,
                pointer: Cell::new(None),
            });
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
        let root = gtk4::Overlay::new();
        root.set_halign(gtk4::Align::Center);
        root.set_valign(gtk4::Align::End);
        root.set_margin_bottom(32);
        root.set_margin_start(16);
        root.set_margin_end(16);
        let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
        panel.add_css_class("screenshot-toolbar");
        panel.set_margin_top(12);
        panel.set_margin_end(12);
        root.set_child(Some(&panel));
        let modes_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        modes_box.set_homogeneous(true);
        let mut modes = Vec::new();
        for (mode, label, icon, key) in [
            (Mode::Area, "Selection", "edit-select-all-symbolic", "S"),
            (Mode::Screen, "Screen", "video-display-symbolic", "C"),
            (Mode::Window, "Window", "focus-windows-symbolic", "W"),
            (
                Mode::All,
                "All screens",
                "video-joined-displays-symbolic",
                "A",
            ),
        ] {
            let button = gtk4::ToggleButton::new();
            button.add_css_class("screenshot-mode");
            let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            content.set_halign(gtk4::Align::Center);
            let image = gtk4::Image::from_icon_name(icon);
            image.set_pixel_size(28);
            content.append(&image);
            content.append(&gtk4::Label::new(Some(label)));
            button.set_child(Some(&content));
            button.update_property(&[gtk4::accessible::Property::Label(label)]);
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
        panel.append(&modes_box);
        let label = gtk4::Label::new(None);
        label.add_css_class("screenshot-details");
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        label.set_max_width_chars(38);
        panel.append(&label);
        let status = gtk4::Label::new(None);
        status.set_wrap(true);
        status.set_max_width_chars(38);
        status.add_css_class("screenshot-status");
        status.set_text(" ");
        status.set_opacity(0.0);
        let cancel = gtk4::Button::from_icon_name("window-close-symbolic");
        cancel.add_css_class("screenshot-cancel");
        cancel.set_halign(gtk4::Align::End);
        cancel.set_valign(gtk4::Align::Start);
        cancel.set_tooltip_text(Some("Cancel (Esc)"));
        cancel.update_property(&[gtk4::accessible::Property::Label("Cancel screenshot")]);
        let capture = gtk4::Button::new();
        capture.add_css_class("screenshot-capture");
        capture.set_halign(gtk4::Align::Center);
        capture.set_tooltip_text(Some("Capture (Enter)"));
        capture.update_property(&[gtk4::accessible::Property::Label("Capture screenshot")]);
        let shutter = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        shutter.add_css_class("screenshot-shutter");
        capture.set_child(Some(&shutter));
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
        let media = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
        media.add_css_class("screenshot-media");
        media.set_valign(gtk4::Align::Center);
        let photo = gtk4::ToggleButton::new();
        photo.set_icon_name("camera-photo-symbolic");
        photo.set_active(true);
        photo.set_tooltip_text(Some("Take Screenshot (V)"));
        photo.update_property(&[gtk4::accessible::Property::Label("Take Screenshot")]);
        let video = gtk4::ToggleButton::new();
        video.set_icon_name("camera-web-symbolic");
        video.set_group(Some(&photo));
        video.update_property(&[gtk4::accessible::Property::Label("Record Screen")]);
        for (button, recording) in [(&photo, false), (&video, true)] {
            button.add_css_class("screenshot-media-button");
            media.append(button);
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.set_video(recording);
                }
            });
        }
        let audio_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        audio_row.set_halign(gtk4::Align::Center);
        audio_row.append(&gtk4::Label::new(Some("Audio")));
        let audio = gtk4::DropDown::from_strings(&["Off", "Microphone", "Desktop"]);
        audio.update_property(&[gtk4::accessible::Property::Label("Recording audio")]);
        audio_row.append(&audio);
        panel.append(&audio_row);
        let bottom = gtk4::CenterBox::new();
        bottom.set_size_request(264, -1);
        bottom.set_start_widget(Some(&media));
        bottom.set_center_widget(Some(&capture));
        panel.append(&bottom);
        panel.append(&status);
        root.add_overlay(&cancel);
        Controls {
            root,
            label,
            status,
            capture,
            modes,
            photo,
            video,
            audio,
            audio_row,
        }
    }
    fn set_video(&self, video: bool) {
        if self.lifecycle.borrow().phase != Some(Phase::Preview)
            || (video && (!screencast::available() || self.recording.is_active()))
        {
            self.refresh();
            return;
        }
        self.video.set(video);
        if video
            && let Some(preview) = self.preview.borrow_mut().as_mut()
            && matches!(preview.selection.mode, Mode::Window | Mode::All)
        {
            preview.selection.mode = Mode::Area;
            preview.drag = None;
        }
        self.status("");
        self.refresh();
    }
    fn set_mode(&self, mode: Mode) {
        if self.lifecycle.borrow().phase != Some(Phase::Preview) {
            return;
        }
        if let Some(p) = self.preview.borrow_mut().as_mut() {
            if self.video.get() && matches!(mode, Mode::Window | Mode::All) {
                return;
            }
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
            let video = self.video.get();
            controls.photo.set_active(!video);
            controls.video.set_active(video);
            controls.photo.set_sensitive(!exporting);
            let available = screencast::available();
            controls
                .video
                .set_sensitive(!exporting && available && !self.recording.is_active());
            controls.video.set_tooltip_text(Some(if !available {
                "Install wf-recorder and FFmpeg tools to record video"
            } else if self.recording.is_active() {
                "A recording is already running"
            } else {
                "Record Screen (V)"
            }));
            controls.audio_row.set_visible(video);
            controls.audio.set_sensitive(!exporting);
            if video {
                controls.capture.add_css_class("video");
            } else {
                controls.capture.remove_css_class("video");
            }
            controls.capture.set_tooltip_text(Some(if video {
                "Record (Enter)"
            } else {
                "Capture (Enter)"
            }));
            controls
                .capture
                .update_property(&[gtk4::accessible::Property::Label(if video {
                    "Start recording"
                } else {
                    "Capture screenshot"
                })]);
            for (mode, button) in &controls.modes {
                button.set_visible(!video || matches!(mode, Mode::Area | Mode::Screen));
                button.set_active(*mode == preview.selection.mode);
                button.set_sensitive(
                    !exporting && (*mode != Mode::Window || !preview.layout.windows.is_empty()),
                );
            }
            let size = dimensions(&preview.layout, &preview.selection);
            let target = Target::from_selection(&preview.layout, &preview.selection);
            controls.capture.set_sensitive(
                !exporting
                    && if video {
                        target.is_ok() && !self.recording.is_active()
                    } else {
                        size.is_ok()
                    },
            );
            if video {
                let error = if self.recording.is_active() {
                    "A recording is already running"
                } else {
                    target.as_ref().err().map_or("", String::as_str)
                };
                controls
                    .status
                    .set_text(if error.is_empty() { " " } else { error });
                controls
                    .status
                    .set_opacity(if error.is_empty() { 0.0 } else { 1.0 });
            }
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
        self.update_cursors();
    }
    fn update_cursors(&self) {
        let preview = self.preview.borrow();
        let Some(p) = preview.as_ref() else {
            return;
        };
        for (index, surface) in self.surfaces.borrow().iter().enumerate() {
            let cursor = if self.lifecycle.borrow().phase == Some(Phase::Exporting) {
                "wait"
            } else {
                match p.selection.mode {
                    Mode::Area => {
                        let hit = p.drag.or_else(|| {
                            surface.pointer.get().map(|(x, y)| {
                                let b = p.layout.outputs[index].bounds;
                                Drag::new(p.selection.area, b.x + x, b.y + y)
                            })
                        });
                        hit.map_or("crosshair", |hit| hit.cursor(p.drag.is_some()))
                    }
                    Mode::Window | Mode::Screen => "pointer",
                    Mode::All => "default",
                }
            };
            surface.canvas.set_cursor_from_name(Some(cursor));
        }
    }
    fn draw(
        &self,
        index: usize,
        pixels: &Pixbuf,
        cr: &gtk4::cairo::Context,
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
            cr.set_fill_rule(gtk4::cairo::FillRule::EvenOdd);
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
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
            cr.set_line_width(4.0);
            cr.rectangle(bounds.x, bounds.y, bounds.width, bounds.height);
            let _ = cr.stroke_preserve();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(2.0);
            let _ = cr.stroke();
            if p.selection.mode == Mode::Area {
                for x in [bounds.x, bounds.x + bounds.width / 2.0, bounds.right()] {
                    for y in [bounds.y, bounds.y + bounds.height / 2.0, bounds.bottom()] {
                        if x == bounds.x + bounds.width / 2.0 && y == bounds.y + bounds.height / 2.0
                        {
                            continue;
                        }
                        let corner = (x == bounds.x || x == bounds.right())
                            && (y == bounds.y || y == bounds.bottom());
                        cr.arc(
                            x,
                            y,
                            if corner { 8.0 } else { 4.0 },
                            0.0,
                            std::f64::consts::TAU,
                        );
                        cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
                        let _ = cr.stroke_preserve();
                        cr.set_source_rgb(1.0, 1.0, 1.0);
                        let _ = cr.fill();
                    }
                }
            }
        }
        let _ = cr.restore();
    }
    fn install_keys(self: &Rc<Self>, window: &libadwaita::Window) {
        let keys = gtk4::EventControllerKey::new();
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
                Key::v | Key::V => this.set_video(!this.video.get()),
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
    fn install_gestures(self: &Rc<Self>, index: usize, canvas: &gtk4::DrawingArea) {
        let motion = gtk4::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        let track_pointer = move |_: &gtk4::EventControllerMotion, x, y| {
            if let Some(this) = weak.upgrade() {
                if let Some(surface) = this.surfaces.borrow().get(index) {
                    surface.pointer.set(Some((x, y)));
                }
                this.update_cursors();
            }
        };
        motion.connect_enter(track_pointer.clone());
        motion.connect_motion(track_pointer);
        let weak = Rc::downgrade(self);
        motion.connect_leave(move |_| {
            if let Some(this) = weak.upgrade() {
                if let Some(surface) = this.surfaces.borrow().get(index) {
                    surface.pointer.set(None);
                }
                this.update_cursors();
            }
        });
        canvas.add_controller(motion);
        let gesture = gtk4::GestureDrag::new();
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
                let drag = Drag::new(p.selection.area, x, y);
                let (left, right, top, bottom) = drag.edges;
                p.drag = Some(drag);
                if !(left || right || top || bottom || drag.moving) {
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
            gesture.set_state(gtk4::EventSequenceState::Claimed);
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
                this.refresh();
            }
        });
        let weak = Rc::downgrade(self);
        gesture.connect_cancel(move |_, _| {
            if let Some(this) = weak.upgrade() {
                if let Some(p) = this.preview.borrow_mut().as_mut() {
                    p.drag = None;
                }
                this.refresh();
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
        if self.video.get() {
            if self.lifecycle.borrow().phase != Some(Phase::Preview) {
                return;
            }
            let prepared = {
                let preview = self.preview.borrow();
                let p = preview.as_ref().unwrap();
                Target::from_selection(&p.layout, &p.selection)
            };
            let target = match prepared {
                Ok(target) => target,
                Err(error) => {
                    self.status(&error);
                    return;
                }
            };
            let audio = match self.controls.borrow().as_ref().unwrap().audio.selected() {
                1 => Audio::Microphone,
                2 => Audio::Desktop,
                _ => Audio::Off,
            };
            if self.recording.is_active() {
                self.status("A recording is already running");
                return;
            }
            self.reset();
            self.display.sync();
            if let Err(error) = self.recording.start(target, audio) {
                self.notify("Recording unavailable", &error, None);
            }
            return;
        }
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
        self.status("");
        self.progress.begin();
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
                        this.complete_export(id, &path, copied);
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
            c.status
                .set_text(if message.is_empty() { " " } else { message });
            c.status
                .set_opacity(if message.is_empty() { 0.0 } else { 1.0 });
        }
    }
    fn complete_export(
        self: &Rc<Self>,
        id: u64,
        path: &std::path::Path,
        copied: Result<bool, String>,
    ) {
        if !self.current(id) || self.saved.replace(true) {
            return;
        }
        let immediate = self.dismissed.get() || copied.is_err();
        let body = match &copied {
            Ok(true) => format!("Saved to {} and copied to clipboard", path.display()),
            Ok(false) => format!("Saved to {}", path.display()),
            Err(_) => format!("Saved to {}; clipboard copy failed", path.display()),
        };
        self.notify("Screenshot saved", &body, Some(path));
        if let Err(error) = copied {
            self.notify("Screenshot clipboard copy failed", &error, None);
        }
        // Notifications follow the file worker; only the overlay may wait.
        if !self.current(id) {
            return;
        }
        if immediate {
            self.reset();
        } else {
            let weak = Rc::downgrade(self);
            self.progress.finish(move || {
                if let Some(this) = weak.upgrade().filter(|t| t.current(id)) {
                    this.reset();
                }
            });
        }
    }
    fn export_failed(&self, id: u64, error: &str) {
        self.progress.clear();
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
    fn area_cursor_matches_draw_move_and_resize_targets() {
        let area = Rect {
            x: -100.0,
            y: 50.0,
            width: 300.0,
            height: 200.0,
        };
        for (x, y, cursor) in [
            (-120.0, 150.0, "crosshair"),
            (50.0, 150.0, "grab"),
            (-108.0, 150.0, "ew-resize"),
            (208.0, 150.0, "ew-resize"),
            (50.0, 42.0, "ns-resize"),
            (50.0, 258.0, "ns-resize"),
            (-100.0, 50.0, "nwse-resize"),
            (200.0, 250.0, "nwse-resize"),
            (-100.0, 250.0, "nesw-resize"),
            (200.0, 50.0, "nesw-resize"),
        ] {
            assert_eq!(Drag::new(area, x, y).cursor(false), cursor);
        }
        assert_eq!(Drag::new(area, 50.0, 150.0).cursor(true), "grabbing");
    }
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
