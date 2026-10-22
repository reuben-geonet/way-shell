use crate::services::clipboard::ClipboardService;
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};
pub struct ClipboardIndicator {
    button: gtk::Button,
    service: Rc<ClipboardService>,
    subscription: u64,
    animation: Rc<RefCell<Option<glib::SourceId>>>,
}
impl ClipboardIndicator {
    pub fn new(service: Rc<ClipboardService>, action: impl Fn(bool) + 'static) -> Self {
        let button = gtk::Button::new();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let icon = gtk::Image::from_icon_name("way-shell-clipboard-symbolic");
        let text = gtk::Label::new(None);
        text.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.set_max_width_chars(28);
        content.append(&icon);
        content.append(&text);
        button.set_child(Some(&content));
        button.add_css_class("clipboard-indicator");
        button.set_tooltip_text(Some("Clipboard history"));
        button.update_property(&[gtk::accessible::Property::Label("Clipboard history")]);
        let action = Rc::new(action);
        let clicked = action.clone();
        button.connect_clicked(move |_| clicked(false));
        let gesture = gtk::GestureClick::new();
        gesture.set_button(2);
        gesture.connect_released(move |_, _, _, _| action(true));
        button.add_controller(gesture);
        let weak = button.downgrade();
        let source = service.clone();
        let icon_update = icon.clone();
        let text_update = text.clone();
        let refresh = Rc::new(move || {
            let Some(button) = weak.upgrade() else {
                return;
            };
            let mode = source.settings.string("indicator-display");
            let paused = source.settings.boolean("incognito");
            button.set_visible(mode != "hidden");
            icon_update.set_visible(mode != "clipboard-content-only" || paused);
            icon_update.set_icon_name(Some(if paused {
                "way-shell-incognito-symbolic"
            } else {
                "way-shell-clipboard-symbolic"
            }));
            text_update.set_visible(matches!(
                mode.as_str(),
                "clipboard-content-only" | "icon-and-clipboard-content"
            ));
            text_update.set_text(
                &source
                    .copied_text()
                    .unwrap_or_else(|| {
                        source
                            .entries()
                            .first()
                            .map(|entry| {
                                if entry.content.text.is_empty() {
                                    entry.content.kind.name().to_owned()
                                } else {
                                    entry.content.text.replace('\n', " ")
                                }
                            })
                            .unwrap_or_default()
                    })
                    .replace('\n', " "),
            );
            button.set_tooltip_text(Some(if paused {
                "Clipboard history paused (incognito)"
            } else {
                "Clipboard history"
            }));
        });
        refresh();
        let callback = refresh.clone();
        let animation = Rc::new(RefCell::new(None::<glib::SourceId>));
        let timer = animation.clone();
        let copied = Cell::new(service.copied());
        let source = service.clone();
        let weak_icon = icon.downgrade();
        let subscription = service.on_changed(move || {
            callback();
            if copied.replace(source.copied()) == source.copied() {
                return;
            }
            let Some(icon) = weak_icon.upgrade() else {
                return;
            };
            if let Some(timer) = timer.borrow_mut().take() {
                timer.remove();
            }
            icon.remove_css_class("clipboard-wiggle-left");
            icon.remove_css_class("clipboard-wiggle-right");
            if !source.settings.boolean("wiggle-indicator") || !icon.is_mapped() {
                return;
            }
            let weak_icon = icon.downgrade();
            let step = Cell::new(0);
            let running = timer.clone();
            *timer.borrow_mut() = Some(glib::timeout_add_local(
                Duration::from_millis(65),
                move || {
                    let Some(icon) = weak_icon.upgrade().filter(|icon| icon.is_mapped()) else {
                        running.borrow_mut().take();
                        return glib::ControlFlow::Break;
                    };
                    icon.remove_css_class("clipboard-wiggle-left");
                    icon.remove_css_class("clipboard-wiggle-right");
                    let next = step.get() + 1;
                    step.set(next);
                    if next >= 6 {
                        running.borrow_mut().take();
                        return glib::ControlFlow::Break;
                    }
                    icon.add_css_class(if next % 2 == 0 {
                        "clipboard-wiggle-left"
                    } else {
                        "clipboard-wiggle-right"
                    });
                    glib::ControlFlow::Continue
                },
            ));
        });
        let timer = animation.clone();
        let weak_icon = icon.downgrade();
        button.connect_unmap(move |_| {
            if let Some(timer) = timer.borrow_mut().take() {
                timer.remove();
            }
            if let Some(icon) = weak_icon.upgrade() {
                icon.remove_css_class("clipboard-wiggle-left");
                icon.remove_css_class("clipboard-wiggle-right");
            }
        });
        // Secondary-click actions are explicit and share the same service state.
        let menu = gtk::Popover::new();
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let pause = gtk::CheckButton::with_label("Incognito");
        pause.set_active(service.settings.boolean("incognito"));
        let source = service.clone();
        pause.connect_toggled(move |button| {
            let _ = source.incognito(button.is_active());
        });
        rows.append(&pause);
        for (title, all) in [("Clear History", false), ("Clear All History", true)] {
            let clear = gtk::Button::with_label(title);
            let source = service.clone();
            clear.connect_clicked(move |_| source.clear(all));
            rows.append(&clear);
        }
        menu.set_child(Some(&rows));
        menu.set_parent(&button);
        let gesture = gtk::GestureClick::new();
        gesture.set_button(3);
        let source = service.clone();
        let popover = menu.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            pause.set_active(source.settings.boolean("incognito"));
            popover.popup();
        });
        button.add_controller(gesture);
        button.connect_destroy(move |_| menu.unparent());
        Self {
            button,
            service,
            subscription,
            animation,
        }
    }
    pub fn widget(&self) -> &gtk::Button {
        &self.button
    }
}
impl Drop for ClipboardIndicator {
    fn drop(&mut self) {
        self.service.disconnect(self.subscription);
        if let Some(timer) = self.animation.borrow_mut().take() {
            timer.remove();
        }
    }
}
