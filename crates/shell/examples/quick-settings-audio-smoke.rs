//! Actual GTK audio controls against isolated PipeWire/WirePlumber/Pulse daemons.
use gtk::prelude::*;
use std::{cell::Cell, rc::Rc, time::Duration};
use way_shell::{services::audio::AudioService, ui::quick_settings::audio::AudioControls};
use wireplumber::{Core, core::ObjectFeatures, local::ImplMetadata, prelude::*, pw::Properties};

#[path = "../tests/common/audio.rs"]
mod fixture;
#[path = "../tests/common/pulse.rs"]
#[allow(dead_code)]
// Reuse the complete service fixture; this probe exercises its UI-facing subset.
mod pulse_fixture;
use fixture::{Daemon, node, wait};

fn descendants(widget: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let widget = widget.as_ref();
    let mut widgets = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(current) = child {
        widgets.extend(descendants(&current));
        child = current.next_sibling();
    }
    widgets
}
fn child<T: IsA<gtk::Widget> + glib::object::ObjectType>(widget: &impl IsA<gtk::Widget>) -> T {
    descendants(widget)
        .into_iter()
        .find_map(|widget| widget.downcast::<T>().ok())
        .unwrap()
}
fn named(widget: &impl IsA<gtk::Widget>, name: &str) -> gtk::Widget {
    descendants(widget)
        .into_iter()
        .find(|widget| widget.widget_name() == name)
        .unwrap()
}
fn row(audio: &AudioControls, text: &str) -> Option<gtk::Widget> {
    let mut current = audio.menu().options().first_child();
    while let Some(widget) = current {
        if widget.has_css_class("quick-settings-menu-option-mixer")
            && descendants(&widget).iter().any(|widget| {
                widget
                    .downcast_ref::<gtk::Label>()
                    .is_some_and(|label| label.text() == text)
            })
        {
            return Some(widget);
        }
        current = widget.next_sibling();
    }
    None
}
fn select(dropdown: &gtk::DropDown, label: &str) {
    let model = dropdown
        .model()
        .unwrap()
        .downcast::<gtk::StringList>()
        .unwrap();
    let index = (0..model.n_items())
        .find(|index| model.string(*index).as_deref() == Some(label))
        .unwrap();
    dropdown.set_selected(index);
}
fn connect_core(context: &glib::MainContext, daemon: &Daemon) -> Core {
    let props = Properties::new();
    props.insert("remote.name", daemon.remote());
    let core = Core::new(Some(context), None, Some(props));
    context.block_on(core.connect_future()).unwrap();
    core
}
fn defaults(context: &glib::MainContext, core: &Core, sink: &str, source: &str) -> ImplMetadata {
    let metadata = ImplMetadata::with_properties(core, Some("default"), None);
    context
        .block_on(metadata.activate_future(ObjectFeatures::ALL))
        .unwrap();
    for (key, name) in [
        ("default.audio.sink", sink),
        ("default.audio.source", source),
    ] {
        metadata.set(
            0,
            Some(key),
            Some("Spa:String:JSON"),
            Some(&serde_json::json!({"name": name}).to_string()),
        );
    }
    metadata
}
fn host(audio: &AudioControls) -> gtk::Window {
    let contents = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let scales = gtk::Box::new(gtk::Orientation::Vertical, 0);
    scales.set_widget_name("quick-settings-scales");
    scales.append(audio.scales());
    contents.append(audio.mixer_button());
    contents.append(&scales);
    contents.append(audio.menu().widget());
    let window = gtk::Window::builder()
        .default_width(440)
        .default_height(600)
        .child(&contents)
        .build();
    window.present();
    window
}
fn volume(context: &glib::MainContext) {
    let mut daemon = Daemon::new();
    let service = AudioService::with_remote(&daemon.remote());
    let audio = AudioControls::new(service.clone());
    let window = host(&audio);
    let main_sink = named(audio.scales(), "default-sink-container");
    let main_source = named(audio.scales(), "default-source-container");
    let sink_scale: gtk::Scale = child(&main_sink);
    let mute: gtk::Button = child(&main_sink);
    assert!(!main_sink.is_sensitive());
    assert!(!main_source.is_visible());

    let core = connect_core(context, &daemon);
    let sink = node(context, &core, "ui-speakers", "Audio/Sink");
    let source = node(context, &core, "ui-microphone", "Audio/Source");
    let metadata = defaults(context, &core, "ui-speakers", "ui-microphone");
    wait(context, || {
        service.state().default_sink == Some(sink.bound_id())
            && service.state().default_source == Some(source.bound_id())
    });
    let link = fixture::connect_nodes(context, &core, &source, &sink);
    wait(context, || {
        main_source.is_visible() && main_sink.is_sensitive()
    });
    let device = row(&audio, "*ui-speakers").unwrap();
    let device_button: gtk::Button = child(&device);
    let device_scale: gtk::Scale = child(&device);
    let revealer: gtk::Revealer = child(&device);
    device_button.emit_clicked();
    assert!(revealer.reveals_child());
    device_scale.set_value(0.37);
    wait(context, || {
        (service
            .state()
            .node(sink.bound_id())
            .unwrap()
            .volume
            .as_ref()
            .unwrap()
            .volume
            - 0.37)
            .abs()
            < 0.0001
    });
    wait(context, || (sink_scale.value() - 0.37).abs() < 0.0001);
    assert_eq!(row(&audio, "*ui-speakers").unwrap(), device);
    assert!(
        revealer.reveals_child(),
        "native updates must retain the open device row"
    );
    mute.emit_clicked();
    wait(context, || {
        service
            .state()
            .node(sink.bound_id())
            .unwrap()
            .volume
            .as_ref()
            .unwrap()
            .mute
    });
    assert_eq!(sink_scale.value(), 0.0);
    assert!((device_scale.value() - 0.37).abs() < 0.0001);
    mute.emit_clicked();
    wait(context, || (sink_scale.value() - 0.37).abs() < 0.0001);
    service.set_volume(sink.bound_id(), 0.81).unwrap();
    wait(context, || {
        (device_scale.value() - 0.81).abs() < 0.0001 && (sink_scale.value() - 0.81).abs() < 0.0001
    });
    audio.set_mixer_revealed(true);
    assert!(!audio.scales().reveals_child());
    service.set_volume(sink.bound_id(), 0.65).unwrap();
    wait(context, || (device_scale.value() - 0.65).abs() < 0.0001);
    audio.set_mixer_revealed(false);
    assert!(audio.scales().reveals_child());
    assert!((sink_scale.value() - 0.65).abs() < 0.0001);

    let old_source = row(&audio, "*ui-microphone").unwrap();
    let old_source_button: gtk::Button = child(&old_source);
    let old_source_revealer: gtk::Revealer = child(&old_source);
    let removed_id = source.bound_id();
    drop((link, source));
    wait(context, || service.state().node(removed_id).is_none());
    assert!(!main_source.is_visible());
    assert!(old_source.parent().is_none());
    old_source_button.emit_clicked();
    assert!(!old_source_revealer.reveals_child());

    // Restart the actual audio daemon and restore defaults on the same controller.
    drop((metadata, sink));
    core.disconnect();
    drop(core);
    daemon.stop();
    wait(context, || !service.state().available);
    assert!(!main_sink.is_sensitive());
    assert_eq!(sink_scale.value(), 0.0);
    daemon.start();
    let core = connect_core(context, &daemon);
    let sink = node(context, &core, "ui-restored", "Audio/Sink");
    let source = node(context, &core, "ui-restored-source", "Audio/Source");
    let link = fixture::connect_nodes(context, &core, &source, &sink);
    let metadata = defaults(context, &core, "ui-restored", "missing");
    wait(context, || {
        service.state().default_sink == Some(sink.bound_id()) && main_sink.is_sensitive()
    });
    assert!(row(&audio, "*ui-restored").is_some());

    // A synchronous GTK callback can stop the service during a snapshot update.
    let once = Rc::new(Cell::new(false));
    let triggered = once.clone();
    let captured = service.clone();
    let handler = sink_scale.connect_value_changed(move |scale| {
        if (scale.value() - 0.22).abs() < 0.0001 && !triggered.replace(true) {
            captured.stop();
        }
    });
    service.set_volume(sink.bound_id(), 0.22).unwrap();
    wait(context, || once.get());
    assert!(!main_sink.is_sensitive());
    assert_eq!(sink_scale.value(), 0.0);
    assert!(row(&audio, "*ui-restored").is_none());
    sink_scale.disconnect(handler);

    let weak = Rc::downgrade(&audio);
    let weak_service = service.downgrade();
    drop(audio);
    assert!(weak.upgrade().is_none());
    mute.emit_clicked();
    sink_scale.set_value(0.9);
    assert!(!main_sink.is_sensitive());
    drop(service);
    assert!(weak_service.upgrade().is_none());
    window.close();
    drop((metadata, link, source, sink));
    core.disconnect();
}

fn routing(context: &glib::MainContext) {
    let mut daemon = Daemon::new();
    daemon.start_pulse();
    let service = AudioService::with_remotes(&daemon.remote(), &daemon.pulse_server());
    let audio = AudioControls::new(service.clone());
    let window = host(&audio);
    let core = connect_core(context, &daemon);
    let sink_a = node(context, &core, "ui-output-a", "Audio/Sink");
    let sink_b = node(context, &core, "ui-output-b", "Audio/Sink");
    let source_a = node(context, &core, "ui-input-a", "Audio/Source");
    let source_b = node(context, &core, "ui-input-b", "Audio/Source");
    let policy = pulse_fixture::Policy::new(&daemon.remote());
    let client = pulse_fixture::Client::new(context, &daemon.pulse_server());
    let playback = client.playback("UI playback", Some("ui-output-a"));
    let capture = client.capture("UI capture", Some("ui-input-a"));
    wait(context, || {
        row(&audio, "UI playback (Output)").is_some() && row(&audio, "UI capture (Input)").is_some()
    });
    let playback_row = row(&audio, "UI playback (Output)").unwrap();
    let capture_row = row(&audio, "UI capture (Input)").unwrap();
    let playback_button: gtk::Button = child(&playback_row);
    let playback_revealer: gtk::Revealer = child(&playback_row);
    let playback_dropdown: gtk::DropDown = child(&playback_row);
    let capture_dropdown: gtk::DropDown = child(&capture_row);
    playback_button.emit_clicked();
    assert!(playback_revealer.reveals_child());
    select(&playback_dropdown, "ui-output-b");
    select(&capture_dropdown, "ui-input-b");
    wait(context, || {
        playback.device_name().as_deref() == Some("ui-output-b")
            && capture.device_name().as_deref() == Some("ui-input-b")
    });
    assert_eq!(row(&audio, "UI playback (Output)").unwrap(), playback_row);
    assert!(playback_revealer.reveals_child());

    let removed_id = sink_a.bound_id();
    drop(sink_a);
    wait(context, || service.state().node(removed_id).is_none());
    assert_eq!(playback.device_name().as_deref(), Some("ui-output-b"));
    assert!(
        descendants(&playback_row)
            .iter()
            .any(|widget| widget.has_css_class("mixer-link"))
    );

    // Routing errors restore the observed choice without closing the stream row.
    let no_router = AudioService::with_remote(&daemon.remote());
    let unavailable = AudioControls::new(no_router.clone());
    wait(context, || {
        row(&unavailable, "UI capture (Input)").is_some()
    });
    let failed_row = row(&unavailable, "UI capture (Input)").unwrap();
    let failed_dropdown: gtk::DropDown = child(&failed_row);
    select(&failed_dropdown, "ui-input-a");
    wait(context, || failed_dropdown.tooltip_text().is_some());
    assert!(
        failed_dropdown
            .tooltip_text()
            .unwrap()
            .contains("unavailable")
    );
    assert_eq!(capture.device_name().as_deref(), Some("ui-input-b"));
    drop(unavailable);
    no_router.stop();
    drop(no_router);

    // Retaining widgets must not retain the controller or an in-flight move.
    select(&capture_dropdown, "ui-input-a");
    let weak = Rc::downgrade(&audio);
    drop(audio);
    assert!(weak.upgrade().is_none());
    playback_button.emit_clicked();
    assert!(playback_revealer.reveals_child());
    context.block_on(glib::timeout_future(Duration::from_millis(40)));
    assert_eq!(capture.device_name().as_deref(), Some("ui-input-b"));
    window.close();
    service.stop();
    drop((
        playback, capture, client, policy, sink_b, source_a, source_b,
    ));
    core.disconnect();
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    adw::init()?;
    let context = glib::MainContext::default();
    let _guard = context.acquire()?;
    volume(&context);
    routing(&context);
    println!(
        "quick-settings mixer/master mute/microphone/routing/hotplug/restart/ownership passed"
    );
    Ok(())
}
