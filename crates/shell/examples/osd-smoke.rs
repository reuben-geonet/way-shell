//! Passive OSD lifetime/timing checks against private audio and sysfs fixtures.
use adw::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::{
    cell::Cell,
    fs,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};
use way_shell::{
    services::{
        audio::AudioService,
        brightness::{Apply, BrightnessService, ControlKind},
    },
    ui::osd::LevelOsd,
};
use wireplumber::{Core, core::ObjectFeatures, local::ImplMetadata, prelude::*, pw::Properties};

#[path = "../tests/common/audio.rs"]
mod fixture;

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn settle(context: &glib::MainContext, milliseconds: u64) {
    context.block_on(glib::timeout_future(Duration::from_millis(milliseconds)));
}
#[track_caller]
fn until(context: &glib::MainContext, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "OSD condition timed out");
        settle(context, 5);
    }
}
fn rows(window: &adw::Window) -> Vec<gtk::Box> {
    let overlay = window
        .content()
        .unwrap()
        .downcast::<gtk::Overlay>()
        .unwrap();
    let mut child = overlay.first_child();
    let mut rows = Vec::new();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget.widget_name() == "osd-container" {
            rows.push(widget.downcast().unwrap());
        }
    }
    rows
}
fn scale(row: &gtk::Box) -> gtk::Scale {
    row.last_child().unwrap().downcast().unwrap()
}
fn icon(row: &gtk::Box) -> gtk::Image {
    row.first_child().unwrap().downcast().unwrap()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    adw::init()?;
    let context = glib::MainContext::default();
    context.with_thread_default(|| {
        let directory = Directory(
            std::env::temp_dir().join(format!("way-shell-level-osd-{}", std::process::id())),
        );
        for (kind, name, current, max) in [
            ("backlight", "panel", "2", "5"),
            ("leds", "keyboard", "3", "3"),
        ] {
            let path = directory.0.join(kind).join(name);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("brightness"), current).unwrap();
            fs::write(path.join("max_brightness"), max).unwrap();
        }
        let source =
            gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)
                .unwrap();
        let settings = gio::Settings::new_full(
            &source
                .lookup("org.ldelossa.way-shell.system", false)
                .unwrap(),
            Some(&gio::memory_settings_backend_new()),
            None,
        );
        settings.set_string("backlight-directory", "panel").unwrap();
        settings
            .set_string("keyboard-backlight-directory", "keyboard")
            .unwrap();
        let apply: Apply = Rc::new(|_, _, _, _| panic!("OSD must never write brightness"));
        let brightness = BrightnessService::with_settings_and_root(&settings, &directory.0, apply);
        let daemon = fixture::Daemon::new();
        let audio = AudioService::with_remote(&daemon.remote());
        let suppressed = Rc::new(Cell::new(false));
        let visible = suppressed.clone();
        let osd = LevelOsd::new(audio.clone(), brightness.clone(), move || visible.get()).unwrap();
        let window = osd.window().clone();
        let rows = rows(&window);
        assert_eq!(rows.len(), 3);
        assert_eq!(window.widget_name(), "osd");
        assert_eq!(window.namespace().as_deref(), Some("way-shell-osd"));
        assert_eq!(window.layer(), Layer::Overlay);
        assert_eq!(window.keyboard_mode(), KeyboardMode::None);
        assert!(window.is_anchor(Edge::Bottom));
        assert!(!window.is_anchor(Edge::Top));
        assert_eq!((window.width_request(), window.height_request()), (340, 64));
        assert!(!window.is_visible());
        brightness.emit_by_name::<()>("brightness-changed", &[&f32::NAN]);
        assert!(!window.is_visible());
        for row in &rows {
            assert!(!scale(row).is_sensitive());
            assert_eq!(icon(row).pixel_size(), 32);
        }

        // Existing defaults and unrelated inventory updates must stay quiet.
        let properties = Properties::new();
        properties.insert("remote.name", daemon.remote());
        let core = Core::new(Some(&context), None, Some(properties));
        context.block_on(core.connect_future()).unwrap();
        let sink = fixture::node(&context, &core, "osd-speakers", "Audio/Sink");
        let capture = fixture::node(&context, &core, "osd-capture", "Audio/Source");
        let metadata = ImplMetadata::with_properties(&core, Some("default"), None);
        context
            .block_on(metadata.activate_future(ObjectFeatures::ALL))
            .unwrap();
        metadata.set(
            0,
            Some("default.audio.sink"),
            Some("Spa:String:JSON"),
            Some("{\"name\":\"osd-speakers\"}"),
        );
        until(&context, || {
            audio.state().default_sink == Some(sink.bound_id())
        });
        let link = fixture::connect_nodes(&context, &core, &capture, &sink);
        settle(&context, 50);
        assert!(!window.is_visible());
        audio.set_volume(sink.bound_id(), 0.37).unwrap();
        until(&context, || {
            window.is_mapped() && (scale(&rows[0]).value() - 0.37).abs() < 0.0001
        });
        assert!(rows[0].get_visible());
        assert!(!rows[1].get_visible() && !rows[2].get_visible());
        assert_eq!(
            icon(&rows[0]).icon_name().as_deref(),
            Some("audio-volume-medium-symbolic")
        );
        audio.set_muted(sink.bound_id(), true).unwrap();
        until(&context, || scale(&rows[0]).value() == 0.0);
        assert_eq!(
            icon(&rows[0]).icon_name().as_deref(),
            Some("audio-volume-muted-symbolic")
        );
        osd.hide();
        suppressed.set(true);
        audio.set_muted(sink.bound_id(), false).unwrap();
        until(&context, || {
            !audio
                .state()
                .node(sink.bound_id())
                .unwrap()
                .volume
                .as_ref()
                .unwrap()
                .mute
        });
        brightness.emit_by_name::<()>("brightness-changed", &[&0.8_f32]);
        settle(&context, 400);
        assert!(!window.is_visible());
        suppressed.set(false);

        // Real sysfs observation selects the matching row and uses raw keyboard steps.
        fs::write(directory.0.join("backlight/panel/brightness"), "4").unwrap();
        brightness.refresh();
        until(&context, || {
            rows[1].get_visible() && (scale(&rows[1]).value() - 0.8).abs() < 0.001
        });
        assert_eq!(
            icon(&rows[1]).icon_name().as_deref(),
            Some("display-brightness-symbolic")
        );
        fs::write(directory.0.join("leds/keyboard/brightness"), "1").unwrap();
        brightness.refresh();
        until(&context, || {
            rows[2].get_visible() && scale(&rows[2]).value() == 1.0
        });
        assert_eq!(scale(&rows[2]).adjustment().upper(), 3.0);
        assert_eq!(
            icon(&rows[2]).icon_name().as_deref(),
            Some("keyboard-brightness-symbolic")
        );
        brightness.set_backend_available(false);
        assert!(!window.is_visible(), "device loss must hide an empty OSD");
        brightness.set_backend_available(true);
        settings
            .set_string("keyboard-backlight-directory", "")
            .unwrap();
        assert!(!brightness.is_available(ControlKind::Keyboard));
        assert_eq!(scale(&rows[2]).adjustment().upper(), 1.0);
        assert!(!window.is_visible());
        settings
            .set_string("keyboard-backlight-directory", "keyboard")
            .unwrap();
        until(&context, || brightness.is_available(ControlKind::Keyboard));
        fs::write(directory.0.join("leds/keyboard/max_brightness"), "5").unwrap();
        brightness.refresh();
        until(&context, || scale(&rows[2]).adjustment().upper() == 5.0);
        osd.hide();

        // A native value notification can synchronously hide the OSD. The older
        // display update must not present it again after the callback returns.
        let weak = Rc::downgrade(&osd);
        let handler =
            scale(&rows[1]).connect_value_changed(move |_| weak.upgrade().unwrap().hide());
        brightness.emit_by_name::<()>("brightness-changed", &[&0.6_f32]);
        assert!(!window.is_visible());
        scale(&rows[1]).disconnect(handler);

        // Hiding cancels in-progress animation and its eventual dismiss timer.
        brightness.emit_by_name::<()>("brightness-changed", &[&0.4_f32]);
        osd.hide();
        settle(&context, 450);
        assert!(!window.is_visible());

        // Exercise the production 350ms / 8s timing, including a fresh update
        // arriving during the reverse slide. It must cancel that older hide.
        brightness.emit_by_name::<()>("brightness-changed", &[&0.6_f32]);
        until(&context, || window.margin(Edge::Bottom) == 120);
        let shown = Instant::now();
        settle(&context, 4100);
        brightness.emit_by_name::<()>("brightness-changed", &[&0.4_f32]);
        settle(&context, 4200);
        assert!(
            window.is_visible(),
            "updated OSD must receive a fresh 8s deadline"
        );
        until(&context, || window.margin(Edge::Bottom) < 120);
        assert!(shown.elapsed() >= Duration::from_secs(11));
        brightness.emit_by_name::<()>("keyboard-brightness-changed", &[&2_u32]);
        until(&context, || window.margin(Edge::Bottom) == 120);
        settle(&context, 450);
        assert!(window.is_visible() && rows[2].get_visible());
        suppressed.set(true);
        brightness.emit_by_name::<()>("brightness-changed", &[&0.8_f32]);
        assert!(rows[2].get_visible());
        until(&context, || !window.is_visible());
        suppressed.set(false);

        // Audio loss clears the currently displayed volume. Retained GTK
        // references and shared services cannot revive a final dropped owner.
        audio.set_volume(sink.bound_id(), 0.7).unwrap();
        until(&context, || window.is_visible() && rows[0].get_visible());
        audio.stop();
        until(&context, || !window.is_visible());
        brightness.emit_by_name::<()>("brightness-changed", &[&0.2_f32]);
        assert!(window.is_visible());
        let weak = Rc::downgrade(&osd);
        drop(osd);
        assert!(weak.upgrade().is_none());
        assert!(!window.is_visible());
        brightness.emit_by_name::<()>("brightness-changed", &[&0.4_f32]);
        settle(&context, 450);
        assert!(!window.is_visible());
        let closed = LevelOsd::new(audio.clone(), brightness.clone(), || false).unwrap();
        brightness.emit_by_name::<()>("brightness-changed", &[&0.6_f32]);
        until(&context, || closed.window().is_mapped());
        closed.window().close();
        brightness.emit_by_name::<()>("brightness-changed", &[&0.6_f32]);
        assert!(!closed.window().is_visible());
        drop(closed);
        let stopped = LevelOsd::new(audio.clone(), brightness.clone(), || false).unwrap();
        stopped.close();
        brightness.emit_by_name::<()>("brightness-changed", &[&0.6_f32]);
        assert!(!stopped.window().is_visible());
        drop(stopped);
        drop((metadata, link, capture, sink));
        core.disconnect();
        assert!(brightness.is_available(ControlKind::Backlight));
        println!("OSD audio, brightness, timing, suppression, reentrancy and cleanup passed");
    })?;
    Ok(())
}
