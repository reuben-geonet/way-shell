//! Owned notification cards; replacing content retains the card and its expanded state.
use crate::services::notifications::{CloseReason, Notification, NotificationsService};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

const FALLBACK: &str = "preferences-system-notifications-symbolic";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardMode {
    Tray,
    Osd,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardEvent {
    Expanded,
    Collapsed,
    Hide,
}

pub(super) struct CardLayout {
    pub root: gtk::Box,
    pub button_container: gtk::Box,
    pub button: gtk::Button,
    pub dismiss: gtk::Button,
    pub expand: gtk::Button,
    pub header_icon: gtk::Image,
    pub app_name: gtk::Label,
    pub timer_label: gtk::Label,
    pub avatar: adw::Avatar,
    pub summary: gtk::Label,
    pub body: gtk::Label,
    pub actions: gtk::Box,
    pub action_revealer: gtk::Revealer,
    pub stack: [gtk::Box; 2],
}
impl CardLayout {
    pub fn new(mode: CardMode) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("notification-widget-container");
        root.set_size_request(400, 120);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("notification-widget");
        root.append(&content);
        let header = gtk::CenterBox::new();
        header.add_css_class("notification-widget-header");
        let left = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        left.set_valign(gtk::Align::Center);
        let right = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        right.set_valign(gtk::Align::Center);
        header.set_start_widget(Some(&left));
        header.set_end_widget(Some(&right));
        let header_icon = gtk::Image::from_icon_name(FALLBACK);
        header_icon.set_pixel_size(18);
        header_icon.set_halign(gtk::Align::Start);
        header_icon.add_css_class("notification-widget-app-icon");
        header_icon.add_css_class("notification-widget-icon");
        let app_name = gtk::Label::new(None);
        app_name.add_css_class("notification-widget-app-name");
        app_name.set_valign(gtk::Align::Center);
        let timer_label = gtk::Label::new(Some("Just now"));
        timer_label.add_css_class("notification-widget-timer");
        timer_label.set_valign(gtk::Align::End);
        left.append(&header_icon);
        left.append(&app_name);
        left.append(&timer_label);
        let expand = gtk::Button::from_icon_name("go-down-symbolic");
        expand.add_css_class("circular");
        expand.add_css_class("notification-widget-expand-button");
        expand.set_visible(mode == CardMode::Tray);
        let dismiss_icon = gtk::Image::from_icon_name("window-close-symbolic");
        dismiss_icon.set_pixel_size(18);
        let dismiss = gtk::Button::new();
        dismiss.set_child(Some(&dismiss_icon));
        dismiss.add_css_class("circular");
        dismiss.add_css_class("notification-widget-dismiss-button");
        right.append(&expand);
        right.append(&dismiss);
        content.append(&header);
        let button_container = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let button_contents = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let button = gtk::Button::new();
        button.set_child(Some(&button_contents));
        button.add_css_class("notification-widget-button");
        button_container.append(&button);
        content.append(&button_container);
        let avatar = adw::Avatar::new(48, None, false);
        avatar.add_css_class("notification-widget-icon");
        avatar.set_icon_name(Some(FALLBACK));
        avatar.set_valign(gtk::Align::Start);
        button_contents.append(&avatar);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_valign(gtk::Align::Center);
        let summary = text_label("summary", if mode == CardMode::Osd { 30 } else { 40 });
        let body = text_label("body", if mode == CardMode::Osd { 30 } else { 200 });
        text.append(&summary);
        text.append(&body);
        button_contents.append(&text);
        let action_revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .build();
        let action_center = gtk::CenterBox::new();
        action_center.set_hexpand(true);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        actions.set_hexpand(true);
        action_center.set_center_widget(Some(&actions));
        action_revealer.set_child(Some(&action_center));
        content.append(&action_revealer);
        let stack = std::array::from_fn(|index| {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
            row.add_css_class(&format!("notification-group-stack-effect-box{}", index + 1));
            row.set_margin_start(5 * (index as i32 + 1));
            row.set_margin_end(5 * (index as i32 + 1));
            let label = gtk::Label::new(None);
            label.set_size_request(1, -1);
            row.append(&label);
            row.set_visible(false);
            root.append(&row);
            row
        });
        Self {
            root,
            button_container,
            button,
            dismiss,
            expand,
            header_icon,
            app_name,
            timer_label,
            avatar,
            summary,
            body,
            actions,
            action_revealer,
            stack,
        }
    }
}

pub struct NotificationCard {
    layout: CardLayout,
    notification: RefCell<Notification>,
    service: NotificationsService,
    hide_button: Option<gtk::Button>,
    timer: RefCell<Option<glib::SourceId>>,
    animation: RefCell<Option<adw::TimedAnimation>>,
    artwork: RefCell<Option<glib::JoinHandle<()>>>,
    revision: Cell<u64>,
    animation_revision: Cell<u64>,
    expanded: Cell<bool>,
    rendering: Cell<bool>,
    pending: RefCell<Option<Notification>>,
    stopped: Cell<bool>,
    event: Box<dyn Fn(CardEvent)>,
}
impl NotificationCard {
    pub fn new(
        service: NotificationsService,
        notification: Notification,
        mode: CardMode,
        event: impl Fn(CardEvent) + 'static,
    ) -> Rc<Self> {
        let layout = CardLayout::new(mode);
        let this = Rc::new(Self {
            layout,
            notification: RefCell::new(notification.clone()),
            service,
            hide_button: (mode == CardMode::Osd).then(|| action_button("Hide")),
            timer: RefCell::new(None),
            animation: RefCell::new(None),
            artwork: RefCell::new(None),
            revision: Cell::new(0),
            animation_revision: Cell::new(0),
            expanded: Cell::new(false),
            rendering: Cell::new(false),
            pending: RefCell::new(None),
            stopped: Cell::new(false),
            event: Box::new(event),
        });
        let weak = Rc::downgrade(&this);
        this.layout.button.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade()
                && !this.stopped.get()
            {
                let id = this.id();
                this.service.invoke_action(id, "default");
                this.service.close(id, CloseReason::Requested);
            }
        });
        let weak = Rc::downgrade(&this);
        this.layout.dismiss.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade()
                && !this.stopped.get()
            {
                this.service.close(this.id(), CloseReason::Dismissed);
            }
        });
        let weak = Rc::downgrade(&this);
        this.layout.expand.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.set_expanded(!this.expanded.get());
            }
        });
        if mode == CardMode::Osd {
            let weak = Rc::downgrade(&this);
            this.hide_button
                .as_ref()
                .unwrap()
                .connect_clicked(move |_| {
                    if let Some(this) = weak.upgrade()
                        && !this.stopped.get()
                    {
                        (this.event)(CardEvent::Hide);
                    }
                });
            let motion = gtk::EventControllerMotion::new();
            let weak = Rc::downgrade(&this);
            motion.connect_enter(move |_, _, _| {
                if let Some(this) = weak.upgrade() {
                    this.set_expanded(true);
                }
            });
            let weak = Rc::downgrade(&this);
            motion.connect_leave(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.set_expanded(false);
                }
            });
            this.layout.root.add_controller(motion);
        }
        this.set_notification(notification);
        let weak = Rc::downgrade(&this);
        this.timer.replace(Some(glib::timeout_add_local(
            Duration::from_secs(60),
            move || {
                if let Some(this) = weak.upgrade()
                    && !this.stopped.get()
                {
                    this.refresh_age();
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                }
            },
        )));
        this
    }
    pub fn widget(&self) -> &gtk::Box {
        &self.layout.root
    }
    pub fn body_container(&self) -> &gtk::Box {
        &self.layout.button_container
    }
    pub fn id(&self) -> u32 {
        self.notification.borrow().id
    }
    pub fn summary(&self) -> &gtk::Label {
        &self.layout.summary
    }
    pub fn body(&self) -> &gtk::Label {
        &self.layout.body
    }
    pub fn button(&self) -> &gtk::Button {
        &self.layout.button
    }
    pub fn dismiss_button(&self) -> &gtk::Button {
        &self.layout.dismiss
    }
    pub fn expand_button(&self) -> &gtk::Button {
        &self.layout.expand
    }
    pub fn avatar(&self) -> &adw::Avatar {
        &self.layout.avatar
    }
    pub fn actions(&self) -> &gtk::Box {
        &self.layout.actions
    }
    pub fn is_expanded(&self) -> bool {
        self.expanded.get()
    }
    pub fn set_stack(&self, stacked: bool) {
        for row in &self.layout.stack {
            row.set_visible(stacked && !self.stopped.get());
        }
    }
    pub fn collapse(self: &Rc<Self>) {
        self.set_expanded(false);
    }
    pub fn set_notification(self: &Rc<Self>, notification: Notification) {
        if self.stopped.get() {
            return;
        }
        self.pending.replace(Some(notification));
        if self.rendering.replace(true) {
            return;
        }
        loop {
            let notification = self.pending.borrow_mut().take();
            let Some(notification) = notification else {
                break;
            };
            if self.stopped.get() {
                break;
            }
            self.notification.replace(notification.clone());
            self.revision.set(self.revision.get().wrapping_add(1));
            self.render(&notification);
        }
        self.rendering.set(false);
    }
    pub fn stop(&self) {
        if self.stopped.replace(true) {
            return;
        }
        self.pending.borrow_mut().take();
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
        self.cancel_artwork();
        let animation = self.animation.borrow_mut().take();
        if let Some(animation) = animation {
            animation.pause();
        }
        self.layout.root.set_sensitive(false);
    }
    fn render(self: &Rc<Self>, notification: &Notification) {
        let request = &notification.request;
        self.layout.app_name.set_label(&request.app_name);
        self.layout.summary.set_text(&plain_text(&request.summary));
        let body = plain_text(&request.body);
        if gtk::pango::parse_markup(&body, '\0').is_ok() {
            self.layout.body.set_markup(&body);
        } else {
            self.layout.body.set_text(&body);
        }
        if request.hints.urgency == 2 {
            self.layout
                .button
                .add_css_class("notification-widget-button-critical");
        } else {
            self.layout
                .button
                .remove_css_class("notification-widget-button-critical");
        }
        while let Some(child) = self.layout.actions.first_child() {
            self.layout.actions.remove(&child);
        }
        let revision = self.revision.get();
        for action in request
            .actions
            .iter()
            .filter(|action| action.key != "default" && !action.label.is_empty())
        {
            let button = action_button(&action.label);
            let key = action.key.clone();
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade()
                    && !this.stopped.get()
                    && this.revision.get() == revision
                {
                    this.service.invoke_action(this.id(), &key);
                }
            });
            self.layout.actions.append(&button);
        }
        if let Some(button) = &self.hide_button {
            self.layout.actions.append(button);
        }
        let children = widget_children(&self.layout.actions);
        for (index, child) in children.iter().enumerate() {
            for class in ["only", "first", "center", "last"] {
                child.remove_css_class(class);
            }
            child.add_css_class(if children.len() == 1 {
                "only"
            } else if index == 0 {
                "first"
            } else if index == children.len() - 1 {
                "last"
            } else {
                "center"
            });
        }
        self.layout
            .action_revealer
            .set_visible(!children.is_empty());
        self.layout
            .action_revealer
            .set_reveal_child(self.expanded.get());
        self.set_icons(notification);
        self.refresh_age();
    }
    fn refresh_age(&self) {
        let created = self.notification.borrow().created_on_us;
        self.layout
            .timer_label
            .set_label(&age_label(created, glib::real_time()));
    }
    fn set_expanded(self: &Rc<Self>, expanded: bool) {
        if self.stopped.get() || self.expanded.replace(expanded) == expanded {
            return;
        }
        let revision = self.animation_revision.get().wrapping_add(1);
        self.animation_revision.set(revision);
        let old = self.animation.borrow_mut().take();
        if let Some(old) = old {
            old.pause();
        }
        self.layout.expand.set_icon_name(if expanded {
            "go-up-symbolic"
        } else {
            "go-down-symbolic"
        });
        self.layout.action_revealer.set_reveal_child(expanded);
        let weak = Rc::downgrade(self);
        let target = adw::CallbackAnimationTarget::new(move |value| {
            if let Some(this) = weak.upgrade()
                && !this.stopped.get()
                && this.animation_revision.get() == revision
            {
                this.layout.body.set_lines(value as i32);
            }
        });
        let animation = adw::TimedAnimation::new(
            &self.layout.body,
            self.layout.body.lines() as f64,
            if expanded { 10.0 } else { 1.0 },
            200,
            target,
        );
        animation.set_easing(adw::Easing::Linear);
        let weak = Rc::downgrade(self);
        animation.connect_done(move |_| {
            if let Some(this) = weak.upgrade()
                && !this.stopped.get()
                && this.animation_revision.get() == revision
            {
                this.animation.borrow_mut().take();
                (this.event)(if expanded {
                    CardEvent::Expanded
                } else {
                    CardEvent::Collapsed
                });
            }
        });
        if !self.stopped.get() && self.animation_revision.get() == revision {
            self.animation.replace(Some(animation.clone()));
            animation.play();
        }
    }
    fn cancel_artwork(&self) {
        let artwork = self.artwork.borrow_mut().take();
        if let Some(artwork) = artwork {
            artwork.abort();
        }
    }
    fn set_icons(self: &Rc<Self>, notification: &Notification) {
        if self.stopped.get() {
            return;
        }
        self.cancel_artwork();
        self.layout
            .avatar
            .set_custom_image(None::<&gtk::gdk::Paintable>);
        self.layout.avatar.set_icon_name(Some(FALLBACK));
        self.layout.header_icon.set_icon_name(Some(FALLBACK));
        let request = &notification.request;
        let app_icon = (!request.app_icon.is_empty()).then_some(request.app_icon.as_str());
        let desktop = if !request.app_name.is_empty() {
            Some(request.app_name.as_str())
        } else {
            request.hints.desktop_entry.as_deref()
        };
        if let Some(icon) = app_icon {
            if icon_is_file(icon) {
                self.layout
                    .header_icon
                    .set_from_gicon(&gio::FileIcon::new(&gio::File::for_commandline_arg(icon)));
            } else {
                self.layout.header_icon.set_icon_name(Some(icon));
            }
        } else if let Some(icon) = desktop.and_then(app_icon_for_id) {
            self.layout.header_icon.set_from_gicon(&icon);
        }
        if let Some(image) = &request.hints.image {
            let bytes = glib::Bytes::from_owned(image.data().to_vec());
            let texture = gtk::gdk::MemoryTexture::new(
                image.width() as i32,
                image.height() as i32,
                if image.has_alpha() {
                    gtk::gdk::MemoryFormat::R8g8b8a8
                } else {
                    gtk::gdk::MemoryFormat::R8g8b8
                },
                &bytes,
                image.rowstride() as usize,
            );
            self.layout.avatar.set_custom_image(Some(&texture));
            return;
        }
        if let Some(path) = request
            .hints
            .image_path
            .as_deref()
            .filter(|path| !path.is_empty())
            .or_else(|| app_icon.filter(|icon| icon_is_file(icon)))
        {
            self.load_artwork(path);
        } else if let Some(icon) = app_icon {
            self.layout.avatar.set_icon_name(Some(icon));
        } else if let Some(icon) = desktop.and_then(app_icon_for_id) {
            let theme = gtk::IconTheme::for_display(&self.layout.root.display());
            let paintable = theme.lookup_by_gicon(
                &icon,
                48,
                1,
                gtk::TextDirection::Rtl,
                gtk::IconLookupFlags::empty(),
            );
            self.layout.avatar.set_custom_image(Some(&paintable));
        }
    }
    fn load_artwork(self: &Rc<Self>, location: &str) {
        if self.stopped.get() {
            return;
        }
        let location = location.to_owned();
        let weak = Rc::downgrade(self);
        let revision = self.revision.get();
        let task = glib::MainContext::ref_thread_default().spawn_local(async move {
            let result = load_avatar(&location).await;
            if let Some(this) = weak.upgrade()
                && !this.stopped.get()
                && this.revision.get() == revision
            {
                match result {
                    Ok(image) => this.layout.avatar.set_custom_image(Some(&image)),
                    Err(error) if !error.matches(gio::IOErrorEnum::Cancelled) => glib::g_message!(
                        "way-shell",
                        "Could not load notification artwork: {error}"
                    ),
                    Err(_) => {}
                }
            }
        });
        self.artwork.replace(Some(task));
    }
}
impl Drop for NotificationCard {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(super) async fn load_avatar(location: &str) -> Result<gtk::gdk::Texture, glib::Error> {
    let file = gio::File::for_commandline_arg(location);
    let stream = file.read_future(glib::Priority::DEFAULT).await?;
    let image = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale_future(&stream, 48, 48, true).await?;
    Ok(gtk::gdk::Texture::for_pixbuf(&image))
}

fn text_label(class: &str, max_width: i32) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class(class);
    label.set_halign(gtk::Align::Start);
    label.set_max_width_chars(max_width);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_lines(1);
    label.set_wrap(true);
    label.set_size_request(380, -1);
    label.set_xalign(0.0);
    label
}
fn action_button(label: &str) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.set_hexpand(true);
    button.add_css_class("notification-widget-action-button");
    button
}
pub(super) fn widget_children(widget: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut result = Vec::new();
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        result.push(widget);
    }
    result
}
fn plain_text(text: &str) -> String {
    text.trim_matches(|character: char| character.is_ascii_whitespace())
        .replace('\n', " ")
}
fn icon_is_file(icon: &str) -> bool {
    std::path::Path::new(icon).is_absolute() || icon.contains("://")
}
pub(super) fn app_icon_for_id(id: &str) -> Option<gio::Icon> {
    if id.is_empty() {
        return None;
    }
    let needle = id.to_lowercase();
    gio::AppInfo::all()
        .into_iter()
        .find(|app| {
            app.id()
                .is_some_and(|id| id.to_lowercase().contains(&needle))
        })
        .and_then(|app| app.icon())
}
pub fn age_label(created_us: i64, now_us: i64) -> String {
    let minutes = now_us.saturating_sub(created_us).max(0) / 60_000_000;
    let (number, unit) = if minutes >= 1440 {
        (minutes / 1440, "day")
    } else if minutes >= 60 {
        (minutes / 60, "hour")
    } else {
        (minutes, "minute")
    };
    if number == 0 {
        "Just now".into()
    } else {
        format!("{number} {unit}{} ago", if number == 1 { "" } else { "s" })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn age_boundaries_and_future_dates() {
        for (minutes, expected) in [
            (0, "Just now"),
            (1, "1 minute ago"),
            (59, "59 minutes ago"),
            (60, "1 hour ago"),
            (120, "2 hours ago"),
            (1440, "1 day ago"),
            (2880, "2 days ago"),
        ] {
            assert_eq!(age_label(0, minutes * 60_000_000), expected);
        }
        assert_eq!(age_label(60_000_000, 0), "Just now");
    }
    #[test]
    fn text_keeps_existing_newline_and_ascii_trim_contract() {
        assert_eq!(plain_text(" \nsummary\nline\t "), "summary line");
        assert_eq!(plain_text("<b>body</b>"), "<b>body</b>");
    }
}
