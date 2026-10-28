//! Run explicitly inside an isolated two-output Sway or Niri session.
use super::*;
use crate::{resources, services::wm::Backend, ui::quick_settings::QuickSettingsWindow};
use std::{path::PathBuf, time::Instant};
#[path = "../../../tests/common/audio_policy.rs"]
mod recording_policy;
#[track_caller]
fn wait(context: &glib::MainContext, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(12);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "Screenshot UI did not reach the expected state"
        );
        context.block_on(glib::timeout_future(Duration::from_millis(20)));
    }
}
#[test]
#[ignore = "requires an isolated two-output Wayland compositor"]
fn two_output_capture_retry_clipboard_and_cleanup() {
    libadwaita::init().unwrap();
    resources::get();
    let context = glib::MainContext::default();
    let audio_test = std::env::var_os("WAY_SHELL_RECORDING_AUDIO_TEST").is_some();
    let _policy = audio_test
        .then(|| recording_policy::Policy::new(&std::env::var("PIPEWIRE_REMOTE").unwrap()));
    let display = gdk::Display::default().unwrap();
    let provider = gtk4::CssProvider::new();
    provider.load_from_path(
        std::env::var_os("WAY_SHELL_SCREENSHOT_TEST_CSS")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../data/theme/way-shell-dark.css")
            }),
    );
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let source =
        gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)
            .unwrap();
    let setting = |name: &str| {
        gio::Settings::new_full(
            &source.lookup(name, false).unwrap(),
            Some(&gio::memory_settings_backend_new()),
            None,
        )
    };
    let wm_settings = setting("org.ldelossa.way-shell.window-manager");
    let system = setting("org.ldelossa.way-shell.system");
    let backend = if std::env::var_os("NIRI_SOCKET").is_some() {
        Backend::Niri
    } else {
        Backend::Sway
    };
    let name = if backend == Backend::Niri {
        "niri"
    } else {
        "sway"
    };
    let directory = PathBuf::from(format!(
        "/tmp/way-shell-screenshot-ui-{name}-{}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    let manager = match backend {
        Backend::Sway => WindowManager::sway_with_settings(
            wm_settings,
            std::env::var_os("SWAYSOCK").unwrap().into(),
            directory.clone(),
        ),
        Backend::Niri => WindowManager::niri_with_settings(
            wm_settings,
            std::env::var_os("NIRI_SOCKET").unwrap().into(),
            directory.clone(),
        ),
    }
    .unwrap();
    let notifications = NotificationsService::new();
    let sample = gtk4::Window::builder()
        .title("Screenshot geometry sample")
        .default_width(460)
        .default_height(260)
        .build();
    let canvas = gtk4::DrawingArea::new();
    canvas.set_draw_func(|_, cr, w, h| {
        cr.set_source_rgb(0.15, 0.38, 0.55);
        let _ = cr.paint();
        cr.set_source_rgb(0.95, 0.65, 0.2);
        cr.rectangle(24.0, 24.0, w as f64 / 2.0, h as f64 / 2.0);
        let _ = cr.fill();
    });
    sample.set_child(Some(&canvas));
    sample.present();
    let expected_outputs = std::env::var("WAY_SHELL_SCREENSHOT_TEST_OUTPUTS")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(2);
    wait(&context, || {
        manager.is_connected() && display.monitors().n_items() == expected_outputs
    });
    context.block_on(glib::timeout_future(Duration::from_millis(500)));
    let quick = QuickSettingsWindow::new().unwrap();
    quick.show();
    let recording = ScreencastService::new(system.clone(), notifications.clone(), manager.clone());
    let screenshot = Screenshot::new(
        manager.clone(),
        system.clone(),
        notifications.clone(),
        recording.clone(),
        &display,
    );
    let quick_weak = Rc::downgrade(&quick);
    screenshot.on_active(move |active| {
        if active && let Some(quick) = quick_weak.upgrade() {
            quick.hide_now();
        }
    });
    screenshot.show(None).unwrap();
    screenshot.show(None).unwrap();
    assert_eq!(screenshot.lifecycle.borrow().generation, 1);
    screenshot.cancel();
    assert!(!screenshot.is_active());
    screenshot.show(None).unwrap();
    wait(&context, || {
        screenshot.lifecycle.borrow().phase == Some(Phase::Preview)
    });
    assert!(!quick.is_visible());
    assert_eq!(
        screenshot.surfaces.borrow().len(),
        expected_outputs as usize
    );
    {
        let p = screenshot.preview.borrow();
        let p = p.as_ref().unwrap();
        assert_eq!(p.layout.outputs.len(), expected_outputs as usize);
        if backend == Backend::Sway {
            assert!(!p.layout.windows.is_empty());
        }
        assert_eq!(
            p.selection.area.width,
            p.layout.outputs[p.launch].bounds.width / 2.0
        );
    }
    // Exercise hover updates through GTK, including a mode change without motion.
    let launch = screenshot.preview.borrow().as_ref().unwrap().launch;
    let canvas = screenshot.surfaces.borrow()[launch].canvas.clone();
    let controllers = canvas.observe_controllers();
    let motion = (0..controllers.n_items())
        .find_map(|i| {
            controllers
                .item(i)
                .and_downcast::<gtk4::EventControllerMotion>()
        })
        .unwrap();
    let (area, output) = {
        let p = screenshot.preview.borrow();
        let p = p.as_ref().unwrap();
        (p.selection.area, p.layout.outputs[launch].bounds)
    };
    for (x, y, cursor) in [
        (area.x - 25.0, area.y - 25.0, "crosshair"),
        (area.x, area.y, "nwse-resize"),
        (area.x + area.width / 2.0, area.y, "ns-resize"),
        (
            area.x + area.width / 2.0,
            area.y + area.height / 2.0,
            "grab",
        ),
    ] {
        motion.emit_by_name::<()>("motion", &[&(x - output.x), &(y - output.y)]);
        assert_eq!(canvas.cursor().unwrap().name().as_deref(), Some(cursor));
    }
    screenshot.set_mode(Mode::Screen);
    assert_eq!(canvas.cursor().unwrap().name().as_deref(), Some("pointer"));
    screenshot.set_mode(Mode::Area);
    assert_eq!(canvas.cursor().unwrap().name().as_deref(), Some("grab"));
    context.block_on(glib::timeout_future(Duration::from_millis(200)));
    let layout = screenshot.preview.borrow().as_ref().unwrap().layout.clone();
    let rendered = context.block_on(capture::acquire(&layout)).unwrap();
    std::fs::write(
        format!("/tmp/way-shell-screenshot-{name}-area-preview.png"),
        capture::encode_png(&rendered[launch]).unwrap(),
    )
    .unwrap();
    let has_windows = !screenshot
        .preview
        .borrow()
        .as_ref()
        .unwrap()
        .layout
        .windows
        .is_empty();
    screenshot.set_mode(Mode::Window);
    if has_windows {
        assert_eq!(
            screenshot.preview.borrow().as_ref().unwrap().selection.mode,
            Mode::Window
        );
    } else {
        assert_eq!(
            screenshot.preview.borrow().as_ref().unwrap().selection.mode,
            Mode::Area
        );
        let controls = screenshot.controls.borrow();
        let button = &controls
            .as_ref()
            .unwrap()
            .modes
            .iter()
            .find(|(mode, _)| *mode == Mode::Window)
            .unwrap()
            .1;
        assert!(!button.is_sensitive());
        assert!(button.tooltip_text().unwrap().contains("geometry"));
    }
    // Remember edits even when closing in another mode or cancelling preparation.
    let remembered = {
        let mut preview = screenshot.preview.borrow_mut();
        let p = preview.as_mut().unwrap();
        p.selection.mode = Mode::Area;
        p.selection.adjust(&p.layout, 17.0, 23.0, false);
        p.selection.adjust(&p.layout, -19.0, -29.0, true);
        p.selection.area
    };
    screenshot.set_mode(Mode::Screen);
    screenshot.cancel();
    screenshot.show(None).unwrap();
    screenshot.cancel();
    screenshot.show(None).unwrap();
    wait(&context, || {
        screenshot.lifecycle.borrow().phase == Some(Phase::Preview)
    });
    {
        let preview = screenshot.preview.borrow();
        let p = preview.as_ref().unwrap();
        assert_eq!(p.selection.mode, Mode::Area);
        assert_eq!(p.selection.area, remembered);
    }
    screenshot.set_mode(Mode::Screen);
    context.block_on(glib::timeout_future(Duration::from_millis(200)));
    // Retain a real compositor-rendered preview for visual inspection.
    let layout = screenshot.preview.borrow().as_ref().unwrap().layout.clone();
    let rendered = context.block_on(capture::acquire(&layout)).unwrap();
    let launch = screenshot.preview.borrow().as_ref().unwrap().launch;
    std::fs::write(
        format!("/tmp/way-shell-screenshot-{name}-preview.png"),
        capture::encode_png(&rendered[launch]).unwrap(),
    )
    .unwrap();
    system
        .set_string("screenshot-save-directory", directory.to_str().unwrap())
        .unwrap();
    system
        .set_boolean("screenshot-copy-to-clipboard", false)
        .unwrap();
    display.clipboard().set_text("clipboard untouched");
    let expected = screenshot.preview.borrow().as_ref().unwrap().images[launch].clone();
    screenshot.capture();
    assert!(
        !screenshot
            .controls
            .borrow()
            .as_ref()
            .unwrap()
            .capture
            .is_sensitive()
    );
    assert_ne!(
        screenshot.controls.borrow().as_ref().unwrap().status.text(),
        "Saving…"
    );
    screenshot.capture();
    screenshot.cancel();
    assert!(screenshot.is_active());
    screenshot.show(None).unwrap();
    wait(&context, || !screenshot.is_active());
    let files: Vec<_> = std::fs::read_dir(&directory)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|e| e == "png"))
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(
        capture::decode_png(
            &std::fs::read(files[0].path()).unwrap(),
            (expected.width, expected.height),
            (expected.width, expected.height)
        )
        .unwrap(),
        expected
    );
    assert_eq!(
        context
            .block_on(display.clipboard().read_text_future())
            .unwrap()
            .as_deref(),
        Some("clipboard untouched")
    );
    screenshot.show(None).unwrap();
    wait(&context, || {
        screenshot.lifecycle.borrow().phase == Some(Phase::Preview)
    });
    assert_eq!(
        screenshot.preview.borrow().as_ref().unwrap().selection.area,
        remembered
    );
    screenshot.set_mode(Mode::Area);
    {
        let mut preview = screenshot.preview.borrow_mut();
        let p = preview.as_mut().unwrap();
        let b = p.layout.bounds();
        p.drag = Some(Drag {
            start: (b.x + 10.0, b.y + 10.0),
            area: p.selection.area,
            edges: (false, false, false, false),
            moving: false,
        });
    }
    screenshot.drag(layout.bounds().width - 20.0, layout.bounds().height - 20.0);
    let selected = screenshot.preview.borrow().as_ref().unwrap().selection.area;
    assert!(
        layout
            .outputs
            .iter()
            .all(|o| o.bounds.intersection(selected).is_some())
    );
    let blocker = directory.join("not-a-directory");
    std::fs::write(&blocker, b"block").unwrap();
    system
        .set_string("screenshot-save-directory", blocker.to_str().unwrap())
        .unwrap();
    system
        .set_boolean("screenshot-copy-to-clipboard", true)
        .unwrap();
    screenshot.capture();
    wait(&context, || {
        screenshot.lifecycle.borrow().phase == Some(Phase::Preview)
    });
    assert!(
        screenshot
            .controls
            .borrow()
            .as_ref()
            .unwrap()
            .status
            .text()
            .contains("Copied to clipboard")
    );
    system
        .set_string("screenshot-save-directory", directory.to_str().unwrap())
        .unwrap();
    screenshot.capture();
    wait(&context, || !screenshot.is_active());
    // GTK owns the PNG provider after every layer window has been destroyed.
    assert!(screenshot.surfaces.borrow().is_empty());
    let (stream, mime) = context
        .block_on(
            display
                .clipboard()
                .read_future(&["image/png"], glib::Priority::DEFAULT),
        )
        .unwrap();
    assert_eq!(mime, "image/png");
    let mut pasted = Vec::new();
    loop {
        let bytes = context
            .block_on(stream.read_bytes_future(65536, glib::Priority::DEFAULT))
            .unwrap();
        if bytes.is_empty() {
            break;
        }
        pasted.extend_from_slice(&bytes);
    }
    assert!(pasted.starts_with(b"\x89PNG\r\n\x1a\n"));
    // Exercise the presentation independently of disk speed. Real encoding,
    // saving, clipboard ownership and retry are checked above.
    for outcome in ["success", "error", "clipboard error", "dismissal"] {
        screenshot.show(None).unwrap();
        wait(&context, || {
            screenshot.lifecycle.borrow().phase == Some(Phase::Preview)
        });
        let id = screenshot.lifecycle.borrow().generation;
        screenshot
            .lifecycle
            .borrow_mut()
            .transition(id, Phase::Preview, Phase::Exporting);
        screenshot.refresh();
        let height = screenshot
            .controls
            .borrow()
            .as_ref()
            .unwrap()
            .root
            .measure(gtk4::Orientation::Vertical, -1);
        screenshot.progress.begin();
        context.block_on(glib::timeout_future(Duration::from_millis(320)));
        assert_eq!(
            screenshot.controls.borrow().as_ref().unwrap().status.text(),
            "Saving…"
        );
        assert_eq!(
            screenshot
                .controls
                .borrow()
                .as_ref()
                .unwrap()
                .root
                .measure(gtk4::Orientation::Vertical, -1),
            height
        );
        if outcome == "error" {
            screenshot.export_failed(id, "Immediate save failure");
            assert!(!screenshot.progress.visible());
            assert_eq!(
                screenshot.controls.borrow().as_ref().unwrap().status.text(),
                "Immediate save failure"
            );
            assert!(
                screenshot
                    .controls
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .capture
                    .is_sensitive()
            );
            screenshot.cancel();
        } else {
            let before = notifications.state().notifications.len();
            let copied = if outcome == "clipboard error" {
                Err("Immediate clipboard failure".to_owned())
            } else {
                Ok(false)
            };
            screenshot.complete_export(id, &files[0].path(), copied);
            let published = notifications.state().notifications;
            assert_eq!(published[before].request.summary, "Screenshot saved");
            assert!(
                published[before]
                    .request
                    .body
                    .contains(directory.to_str().unwrap())
            );
            if outcome == "clipboard error" {
                assert_eq!(published.len(), before + 2);
                assert_eq!(
                    published[before + 1].request.summary,
                    "Screenshot clipboard copy failed"
                );
                assert!(!screenshot.is_active());
            } else {
                assert_eq!(published.len(), before + 1);
                context.block_on(glib::timeout_future(Duration::from_millis(100)));
                assert!(screenshot.is_active());
                assert!(screenshot.progress.visible());
                if outcome == "dismissal" {
                    screenshot.cancel();
                    screenshot.show(None).unwrap();
                    wait(&context, || {
                        screenshot.lifecycle.borrow().phase == Some(Phase::Preview)
                    });
                    context.block_on(glib::timeout_future(Duration::from_millis(520)));
                    assert!(screenshot.is_active());
                    screenshot.cancel();
                }
            }
            wait(&context, || !screenshot.is_active());
            assert!(screenshot.surfaces.borrow().is_empty());
            assert_eq!(notifications.state().notifications, published);
        }
        context.block_on(glib::timeout_future(Duration::from_millis(520)));
        assert!(!screenshot.is_active());
        assert!(!screenshot.progress.visible());
    }
    // Exercise real wf-recorder output, not a mock capture pipeline. The
    // surrounding compositor and audio server must belong to this test run.
    assert!(screencast::available());
    let videos = directory.join("videos");
    system
        .set_string("screencast-save-directory", videos.to_str().unwrap())
        .unwrap();
    let mut cases = vec![(Mode::Screen, 0), (Mode::Area, 0)];
    if audio_test {
        cases.extend([(Mode::Screen, 1), (Mode::Screen, 2)]);
    }
    for (index, (mode, audio)) in cases.into_iter().enumerate() {
        screenshot.show(None).unwrap();
        wait(&context, || {
            screenshot.lifecycle.borrow().phase == Some(Phase::Preview)
        });
        screenshot.set_mode(Mode::All);
        screenshot
            .controls
            .borrow()
            .as_ref()
            .unwrap()
            .video
            .emit_clicked();
        assert!(screenshot.video.get());
        assert_eq!(
            screenshot.preview.borrow().as_ref().unwrap().selection.mode,
            Mode::Area
        );
        if expected_outputs > 1 && index == 0 {
            assert!(
                !screenshot
                    .controls
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .capture
                    .is_sensitive()
            );
        }
        screenshot.set_mode(mode);
        if mode == Mode::Area {
            let mut preview = screenshot.preview.borrow_mut();
            let p = preview.as_mut().unwrap();
            let bounds = p.layout.outputs[p.launch].bounds;
            p.selection.area = Rect {
                x: bounds.x + 21.2,
                y: bounds.y + 21.2,
                width: 81.7,
                height: 51.7,
            };
        }
        screenshot.refresh();
        {
            let controls = screenshot.controls.borrow();
            let controls = controls.as_ref().unwrap();
            assert!(controls.video.is_active());
            assert!(!controls.photo.is_active());
            assert!(controls.capture.has_css_class("video"));
            assert!(controls.capture.is_sensitive());
            assert_eq!(controls.audio.selected(), 0);
            controls.audio.set_selected(audio);
            assert!(
                controls
                    .modes
                    .iter()
                    .filter(|(mode, _)| matches!(mode, Mode::Window | Mode::All))
                    .all(|(_, button)| !button.is_visible())
            );
        }
        context.block_on(glib::timeout_future(Duration::from_millis(250)));
        let layout = screenshot.preview.borrow().as_ref().unwrap().layout.clone();
        let rendered = context.block_on(capture::acquire(&layout)).unwrap();
        std::fs::write(
            format!("/tmp/way-shell-video-{name}-{index}-preview.png"),
            capture::encode_png(&rendered[launch]).unwrap(),
        )
        .unwrap();
        screenshot.capture();
        assert!(!screenshot.is_active());
        assert!(screenshot.surfaces.borrow().is_empty());
        wait(&context, || {
            recording.phase() == screencast::Phase::Recording
        });
        quick.show();
        context.block_on(glib::timeout_future(Duration::from_millis(1100)));
        assert_eq!(recording.phase(), screencast::Phase::Recording);
        assert_ne!(recording.elapsed(), "00:00");
        quick.hide_now();
        screenshot.show(None).unwrap();
        wait(&context, || {
            screenshot.lifecycle.borrow().phase == Some(Phase::Preview)
        });
        assert!(!screenshot.video.get());
        assert!(
            !screenshot
                .controls
                .borrow()
                .as_ref()
                .unwrap()
                .video
                .is_sensitive()
        );
        screenshot.set_video(true);
        assert!(!screenshot.video.get());
        screenshot.cancel();
        recording.stop();
        recording.stop();
        assert_eq!(recording.phase(), screencast::Phase::Stopping);
        let frozen = recording.elapsed();
        let first_panel = crate::ui::panel::recording::RecordingIndicator::new(recording.clone());
        let rebuilt_panel = crate::ui::panel::recording::RecordingIndicator::new(recording.clone());
        for panel in [&first_panel, &rebuilt_panel] {
            assert!(!panel.widget().is_sensitive());
            let label = panel
                .widget()
                .child()
                .unwrap()
                .first_child()
                .unwrap()
                .downcast::<gtk4::Label>()
                .unwrap();
            assert_eq!(label.text(), frozen);
            assert!(!label.text().contains("Saving"));
        }
        wait(&context, || !recording.is_active());
        let files: Vec<_> = std::fs::read_dir(&videos)
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|e| e == "mp4"))
            .collect();
        assert_eq!(files.len(), index + 1);
        let path = files.iter().map(|e| e.path()).max().unwrap();
        let state = notifications.state();
        assert!(
            state.notifications.iter().any(|notification| {
                notification.request.summary == "Recording saved"
                    && notification.request.body.contains(path.to_str().unwrap())
            }),
            "Recording did not finish successfully: {state:?}"
        );
        let output = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name,codec_type,width,height",
                "-of",
                "json",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert!(output.status.success());
        let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let video = &info["streams"][0];
        assert_eq!(video["codec_name"], "h264");
        assert_eq!(video["width"].as_u64().unwrap() % 2, 0);
        assert_eq!(video["height"].as_u64().unwrap() % 2, 0);
        if audio != 0 {
            assert!(
                info["streams"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|stream| stream["codec_type"] == "audio" && stream["codec_name"] == "aac")
            );
        }
        std::fs::copy(path, format!("/tmp/way-shell-recording-{name}-{index}.mp4")).unwrap();
    }
    // Keep the remembered screenshot-area check independent of video edits.
    screenshot.last_area.set(Some(selected));
    screenshot.show(None).unwrap();
    wait(&context, || {
        screenshot.lifecycle.borrow().phase == Some(Phase::Preview)
    });
    assert_eq!(
        screenshot.preview.borrow().as_ref().unwrap().selection.area,
        selected
    );
    if backend == Backend::Sway {
        use std::io::{Read, Write};
        let scale = screenshot
            .preview
            .borrow()
            .as_ref()
            .unwrap()
            .layout
            .outputs
            .iter()
            .find(|output| output.connector == "HEADLESS-1")
            .unwrap()
            .scale;
        let command = format!(
            "output HEADLESS-1 scale {}",
            if scale == 1.25 { 1.0 } else { 1.25 }
        );
        let mut peer = std::os::unix::net::UnixStream::connect(manager.socket_path()).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        peer.write_all(&way_shell_core::sway::encode(0, command.as_bytes()).unwrap())
            .unwrap();
        let mut header = [0; 14];
        peer.read_exact(&mut header).unwrap();
        let size = u32::from_le_bytes(header[6..10].try_into().unwrap()) as usize;
        let mut reply = vec![0; size];
        peer.read_exact(&mut reply).unwrap();
        way_shell_core::sway::acknowledged(&reply, true).unwrap();
        wait(&context, || !screenshot.is_active());
        assert!(screenshot.surfaces.borrow().is_empty());
    }
    screenshot.show(None).unwrap();
    screenshot.cancel();
    assert!(screenshot.surfaces.borrow().is_empty());
    screenshot.close();
    recording.shutdown();
    quick.close();
    sample.destroy();
    manager.stop();
    notifications.stop();
    std::fs::remove_dir_all(directory).unwrap();
}
