//! Every panel observes the same recorder and elapsed clock.
use crate::services::screencast::{Phase, ScreencastService};
use gtk::prelude::*;

pub struct RecordingIndicator {
    service: ScreencastService,
    button: gtk::Button,
    changed: Option<glib::SignalHandlerId>,
    clicked: Option<glib::SignalHandlerId>,
}
impl RecordingIndicator {
    pub fn new(service: ScreencastService) -> Self {
        let button = gtk::Button::new();
        button.add_css_class("panel-recording");
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let label = gtk::Label::new(None);
        label.set_width_chars(5);
        content.append(&label);
        content.append(&gtk::Image::from_icon_name("media-playback-stop-symbolic"));
        button.set_child(Some(&content));
        let weak_button = button.downgrade();
        let weak_label = label.downgrade();
        let changed = service.connect_local("changed", false, move |values| {
            if let (Some(button), Some(label)) = (weak_button.upgrade(), weak_label.upgrade()) {
                refresh(
                    &values[0].get::<ScreencastService>().unwrap(),
                    &button,
                    &label,
                );
            }
            None
        });
        let weak = service.downgrade();
        let clicked = button.connect_clicked(move |_| {
            if let Some(service) = weak.upgrade() {
                service.stop();
            }
        });
        refresh(&service, &button, &label);
        Self {
            service,
            button,
            changed: Some(changed),
            clicked: Some(clicked),
        }
    }
    pub fn widget(&self) -> &gtk::Button {
        &self.button
    }
}
impl Drop for RecordingIndicator {
    fn drop(&mut self) {
        if let Some(handler) = self.changed.take() {
            self.service.disconnect(handler);
        }
        if let Some(handler) = self.clicked.take() {
            self.button.disconnect(handler);
        }
    }
}
fn refresh(service: &ScreencastService, button: &gtk::Button, label: &gtk::Label) {
    let phase = service.phase();
    button.set_visible(phase != Phase::Idle);
    button.set_sensitive(matches!(phase, Phase::Starting | Phase::Recording));
    let text = service.elapsed();
    label.set_text(&text);
    let action = match phase {
        Phase::Starting => "Cancel recording startup",
        Phase::Stopping => "Finishing recording",
        _ => "Stop recording",
    };
    button.set_tooltip_text(Some(action));
    button.update_property(&[gtk::accessible::Property::Label(&format!(
        "{action}: {text}"
    ))]);
}
