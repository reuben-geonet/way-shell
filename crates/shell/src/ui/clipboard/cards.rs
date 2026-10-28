use super::*;
use crate::services::clipboard::{now, preview};
use gtk4::pango;
impl ClipboardDialog {
    pub(super) fn card(self: &Rc<Self>, entry: &Entry) -> gtk4::Box {
        let settings = &self.service.settings;
        let dynamic = settings.boolean("dynamic-item-height")
            && settings.string("clipboard-orientation") == "vertical";
        let card = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        card.add_css_class("clipboard-card");
        card.set_overflow(gtk4::Overflow::Hidden);
        card.set_size_request(
            settings.int("item-width"),
            if dynamic {
                -1
            } else {
                settings.int("item-height")
            },
        );
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        header.add_css_class("clipboard-card-header");
        if let Some(tag) = &entry.tag {
            header.add_css_class(&format!("clipboard-tag-{tag}"));
        }
        let icon = gtk4::Image::from_icon_name(entry.content.kind.icon());
        icon.set_visible(settings.boolean("show-header"));
        header.append(&icon);
        let name = gtk4::Label::new(Some(
            if settings.boolean("show-item-title") && !entry.title.is_empty() {
                &entry.title
            } else {
                entry.content.kind.name()
            },
        ));
        name.add_css_class("clipboard-kind");
        name.set_ellipsize(pango::EllipsizeMode::End);
        name.set_xalign(0.0);
        name.set_visible(
            settings.boolean("show-header")
                || (settings.boolean("show-item-title") && !entry.title.is_empty()),
        );
        header.append(&name);
        let time = gtk4::Label::new(Some(&relative_time(entry.copied)));
        time.add_css_class("clipboard-time");
        time.set_hexpand(true);
        time.set_xalign(0.0);
        time.set_visible(settings.boolean("show-header") && entry.title.is_empty());
        header.append(&time);
        let menu = gtk4::MenuButton::new();
        menu.set_icon_name("view-more-symbolic");
        menu.set_tooltip_text(Some("Item actions and tag"));
        menu.set_popover(Some(&self.item_menu(entry.id)));
        header.append(&menu);
        let delete = icon_button("user-trash-symbolic", "Delete item (hold Shift to force)");
        let weak = Rc::downgrade(self);
        let id = entry.id;
        let force = Rc::new(Cell::new(false));
        let gesture = gtk4::GestureClick::new();
        gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let pressed = force.clone();
        gesture.connect_pressed(move |gesture, _, _, _| {
            pressed.set(
                gesture
                    .current_event_state()
                    .contains(gdk::ModifierType::SHIFT_MASK),
            );
        });
        delete.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.service.delete(id, force.replace(false));
            }
        });
        delete.add_controller(gesture);
        header.append(&delete);
        let pin = gtk4::ToggleButton::new();
        pin.set_icon_name("way-shell-pin-symbolic");
        pin.set_active(entry.pinned);
        pin.set_tooltip_text(Some("Pin item"));
        let weak = Rc::downgrade(self);
        let id = entry.id;
        pin.connect_toggled(move |button| {
            if let Some(this) = weak.upgrade()
                && let Some(mut entry) = this.service.entry(id)
            {
                entry.pinned = button.is_active();
                this.service.update(entry);
            }
        });
        header.append(&pin);
        let show_header = settings.boolean("show-header")
            || (settings.boolean("show-item-title") && !entry.title.is_empty());
        if !show_header
            && (entry.content.kind == Kind::Color
                || (entry.content.kind == Kind::Image
                    && !settings.child("image-item").boolean("show-image-info")))
        {
            card.add_css_class("clipboard-full-bleed");
        }
        if show_header {
            card.append(&header);
        }
        if settings.string("header-controls-visibility") != "visible" {
            let buttons = [menu.upcast::<gtk4::Widget>(), delete.upcast(), pin.upcast()];
            let hover = settings.string("header-controls-visibility") == "visible-on-hover";
            for button in &buttons {
                button.set_opacity(0.0);
                button.set_can_target(false);
            }
            if hover {
                let motion = gtk4::EventControllerMotion::new();
                let showing = buttons.clone();
                motion.connect_enter(move |_, _, _| {
                    for button in &showing {
                        button.set_opacity(1.0);
                        button.set_can_target(true);
                    }
                });
                motion.connect_leave(move |_| {
                    for button in &buttons {
                        button.set_opacity(0.0);
                        button.set_can_target(false);
                    }
                });
                card.add_controller(motion);
            }
        }
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        content.set_hexpand(true);
        content.set_vexpand(true);
        content.add_css_class("clipboard-content");
        let clip = gtk4::ScrolledWindow::new();
        clip.set_policy(gtk4::PolicyType::External, gtk4::PolicyType::External);
        clip.set_propagate_natural_height(dynamic);
        clip.set_max_content_height(
            (settings.int("item-height") - if show_header { 45 } else { 17 }).max(20),
        );
        clip.set_hexpand(true);
        clip.set_vexpand(!dynamic);
        clip.set_child(Some(&content));
        let overlay = gtk4::Overlay::new();
        overlay.set_child(Some(&clip));
        overlay.set_vexpand(true);
        if !show_header {
            header.set_halign(gtk4::Align::End);
            header.set_valign(gtk4::Align::Start);
            overlay.add_overlay(&header);
        }
        card.append(&overlay);
        match entry.content.kind {
            Kind::Image => {
                let picture = picture();
                if dynamic {
                    picture.set_size_request(-1, settings.int("item-height") - 17);
                }
                content.append(&picture);
                let info = gtk4::Label::new(Some(&format!(
                    "{} · {}",
                    entry.content.mime,
                    glib::format_size(entry.content.bytes.len() as u64)
                )));
                info.add_css_class("clipboard-time");
                info.set_visible(settings.child("image-item").boolean("show-image-info"));
                content.append(&info);
                self.image(
                    &picture,
                    entry.content.bytes.clone(),
                    settings.child("image-item").string("background-size") == "cover",
                    Some(&info),
                );
            }
            Kind::Color => {
                let text = entry.content.text.clone();
                let drawing = gtk4::DrawingArea::new();
                drawing.set_vexpand(true);
                drawing.set_content_height(
                    settings.int("item-height") - if show_header { 45 } else { 17 },
                );
                let color = way_shell_clipboard::actions::convert_color(&text, "hex")
                    .unwrap_or_else(|_| text.clone());
                let mut foreground = "#fff";
                if let Ok(color) = gdk::RGBA::parse(&color) {
                    if 0.2126 * color.red() + 0.7152 * color.green() + 0.0722 * color.blue() > 0.5 {
                        foreground = "#222";
                    }
                    drawing.set_draw_func(move |_, cr, w, h| {
                        cr.set_source_rgba(
                            f64::from(color.red()),
                            f64::from(color.green()),
                            f64::from(color.blue()),
                            f64::from(color.alpha()),
                        );
                        cr.rectangle(0.0, 0.0, f64::from(w), f64::from(h));
                        let _ = cr.fill();
                    });
                }
                let swatch = gtk4::Overlay::new();
                swatch.add_css_class("clipboard-color-preview");
                swatch.set_overflow(gtk4::Overflow::Hidden);
                swatch.set_child(Some(&drawing));
                let caption = label(&text);
                caption.add_css_class("clipboard-color-label");
                caption.set_markup(&format!(
                    "<b><span foreground='{foreground}'>{}</span></b>",
                    glib::markup_escape_text(&text)
                ));
                caption.set_halign(gtk4::Align::Center);
                caption.set_valign(gtk4::Align::Center);
                swatch.add_overlay(&caption);
                content.append(&swatch);
            }
            Kind::Character => {
                let text = label(&entry.content.text);
                text.add_css_class("clipboard-character");
                text.set_wrap(false);
                text.set_ellipsize(pango::EllipsizeMode::None);
                text.set_valign(gtk4::Align::Center);
                text.set_vexpand(true);
                text.set_xalign(0.5);
                let height = f64::from(
                    settings.int("item-height")
                        - if show_header { 45 } else { 17 }
                        - if settings.child("character-item").boolean("show-unicode") {
                            20
                        } else {
                            0
                        },
                )
                .max(1.0);
                let mut size = (200.0 * height / (height + 200.0)).clamp(20.0, 80.0);
                let mut font = text.pango_context().font_description().unwrap_or_default();
                font.set_absolute_size(size * f64::from(pango::SCALE));
                let layout = text.create_pango_layout(Some(&entry.content.text));
                layout.set_font_description(Some(&font));
                let (width, _) = layout.pixel_size();
                size *=
                    (f64::from(settings.int("item-width") - 20) / f64::from(width.max(1))).min(1.0);
                let attributes = pango::AttrList::new();
                attributes.insert(pango::AttrSize::new_size_absolute(
                    (size * f64::from(pango::SCALE)) as i32,
                ));
                text.set_attributes(Some(&attributes));
                content.append(&text);
                if settings.child("character-item").boolean("show-unicode") {
                    let unicode = label(
                        &entry
                            .content
                            .text
                            .trim()
                            .chars()
                            .map(|c| format!("U+{:04X}", c as u32))
                            .collect::<Vec<_>>()
                            .join(" "),
                    );
                    unicode.add_css_class("clipboard-time");
                    unicode.set_xalign(0.5);
                    content.append(&unicode);
                }
            }
            Kind::Code => self.code(&content, entry, false),
            Kind::Link => {
                let text = label(&entry.content.text);
                text.add_css_class("link");
                content.append(&text);
                self.link_preview(&card, &content, entry);
            }
            Kind::File | Kind::Files => self.files(&card, &content, entry),
            Kind::Text => {
                let text = label(&entry.content.text);
                text.set_vexpand(true);
                text.set_valign(gtk4::Align::Start);
                content.append(&text);
                self.text_info(
                    &content,
                    &entry.content.text,
                    &settings.child("text-item"),
                    "show-text-info",
                );
            }
        }
        if dynamic {
            let (minimum, _, _, _) = content.measure(gtk4::Orientation::Horizontal, -1);
            let (_, natural, _, _) = content.measure(
                gtk4::Orientation::Vertical,
                (settings.int("item-width") - 20).max(minimum),
            );
            let maximum = (settings.int("item-height") - if show_header { 45 } else { 17 }).max(20);
            clip.set_min_content_height(natural.clamp(16, maximum));
        }
        let click = gtk4::GestureClick::new();
        click.set_button(0);
        let weak = Rc::downgrade(self);
        let id = entry.id;
        let weak_card = card.downgrade();
        click.connect_released(move |gesture, _, x, y| {
            let Some(this) = weak.upgrade() else {
                return;
            };
            let Some(card) = weak_card.upgrade() else {
                return;
            };
            if card
                .pick(x, y, gtk4::PickFlags::DEFAULT)
                .is_some_and(|picked| {
                    picked.ancestor(gtk4::Button::static_type()).is_some()
                        || picked.ancestor(gtk4::MenuButton::static_type()).is_some()
                })
            {
                return;
            }
            let modifiers = gesture.current_event_state();
            match gesture.current_button() {
                1 => {
                    if modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
                        this.default_action(id);
                    } else {
                        this.activate(
                            id,
                            modifiers.contains(gdk::ModifierType::SHIFT_MASK)
                                ^ this.service.settings.boolean("swap-copy-shortcut"),
                        );
                    }
                }
                2 => match this.service.settings.string("middle-click-action").as_str() {
                    "pin" => {
                        if let Some(mut entry) = this.service.entry(id) {
                            entry.pinned = !entry.pinned;
                            this.service.update(entry);
                        }
                    }
                    "delete" => this
                        .service
                        .delete(id, modifiers.contains(gdk::ModifierType::SHIFT_MASK)),
                    _ => {}
                },
                3 => this.show_item_menu(id),
                _ => {}
            }
        });
        card.add_controller(click);
        card
    }
    fn code(self: &Rc<Self>, container: &gtk4::Box, entry: &Entry, file: bool) {
        let settings = self
            .service
            .settings
            .child(if file { "file-item" } else { "code-item" });
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let text = entry.content.text.replace(
            '\t',
            &" ".repeat(self.service.settings.int("tab-width") as usize),
        );
        let excerpt = text.lines().take(80).collect::<Vec<_>>().join("\n");
        if settings.boolean("show-line-numbers") {
            let numbers = label(
                &(0..excerpt.lines().count())
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            numbers.add_css_class("clipboard-time");
            numbers.add_css_class("monospace");
            numbers.set_xalign(1.0);
            numbers.set_valign(gtk4::Align::Start);
            row.append(&numbers);
        }
        let code = label(&excerpt);
        code.set_wrap(false);
        code.set_hexpand(true);
        code.set_valign(gtk4::Align::Start);
        code.add_css_class("monospace");
        row.append(&code);
        container.append(&row);
        if settings.boolean("syntax-highlighting") {
            let language = entry.content.metadata["language"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let weak = code.downgrade();
            let light = self.light_scheme();
            glib::spawn_future_local(async move {
                let result = gio::spawn_blocking(move || {
                    let spans = way_shell_clipboard::highlight(&excerpt, &language);
                    let mut markup = String::new();
                    let mut last = 0;
                    for span in spans {
                        if span.start > last {
                            markup.push_str(&glib::markup_escape_text(&excerpt[last..span.start]));
                        }
                        let color = match (span.role, light) {
                            ("comment", true) => "#77767b",
                            ("keyword", true) => "#a51d2d",
                            ("string" | "name", true) => "#117d55",
                            ("number", true) => "#3f44a0",
                            ("comment", false) => "#999999",
                            ("keyword", false) => "#ffa348",
                            ("string" | "name", false) => "#5bc8af",
                            ("number", false) => "#7d8ac7",
                            (_, true) => "#222226",
                            (_, false) => "#eeeeec",
                        };
                        markup.push_str(&format!(
                            "<span foreground='{color}'>{}</span>",
                            glib::markup_escape_text(&excerpt[span.start..span.end])
                        ));
                        last = span.end;
                    }
                    markup.push_str(&glib::markup_escape_text(&excerpt[last..]));
                    markup
                })
                .await;
                if let Ok(markup) = result
                    && let Some(code) = weak.upgrade()
                {
                    code.set_markup(&markup);
                }
            });
        }
        if !file {
            self.text_info(container, &entry.content.text, &settings, "show-code-info");
        }
    }
    fn text_info(&self, container: &gtk4::Box, text: &str, settings: &gio::Settings, key: &str) {
        if !settings.boolean(key) {
            return;
        }
        let (value, unit) = match settings.string("text-count-mode").as_str() {
            "words" => (text.split_whitespace().count(), "words"),
            "lines" => (text.lines().count(), "lines"),
            _ => (text.chars().count(), "characters"),
        };
        let info = label(&format!("{value} {unit}"));
        info.add_css_class("clipboard-time");
        container.append(&info);
    }
    fn image(
        &self,
        picture: &gtk4::Picture,
        bytes: Vec<u8>,
        cover: bool,
        info: Option<&gtk4::Label>,
    ) {
        picture.set_content_fit(if cover {
            gtk4::ContentFit::Cover
        } else {
            gtk4::ContentFit::Contain
        });
        let weak = picture.downgrade();
        let info = info.map(|info| (info.downgrade(), info.text().to_string()));
        glib::spawn_future_local(async move {
            let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes));
            let result = if info.is_some() {
                gtk4::gdk_pixbuf::Pixbuf::from_stream_future(&stream).await
            } else {
                gtk4::gdk_pixbuf::Pixbuf::from_stream_at_scale_future(&stream, 1000, 600, true)
                    .await
            };
            if let Ok(pixbuf) = result
                && let Some(picture) = weak.upgrade()
            {
                if let Some((info, text)) = info
                    && let Some(info) = info.upgrade()
                {
                    info.set_text(&format!(
                        "{} × {} · {text}",
                        pixbuf.width(),
                        pixbuf.height()
                    ));
                }
                picture.set_paintable(Some(&gdk::Texture::for_pixbuf(&pixbuf)));
            }
        });
    }
    fn link_preview(self: &Rc<Self>, card: &gtk4::Box, container: &gtk4::Box, entry: &Entry) {
        let settings = self.service.settings.child("link-item");
        if !settings.boolean("show-link-preview")
            || settings
                .strv("link-preview-exclusion-patterns")
                .iter()
                .any(|pattern| {
                    regress::Regex::new(pattern)
                        .is_ok_and(|regex| regex.find(&entry.content.text).is_some())
                })
        {
            return;
        }
        let preview_box = gtk4::Box::new(
            if settings.string("link-preview-orientation") == "horizontal" {
                gtk4::Orientation::Horizontal
            } else {
                gtk4::Orientation::Vertical
            },
            4,
        );
        preview_box.set_vexpand(true);
        container.prepend(&preview_box);
        if let Some(url) = container.last_child() {
            url.add_css_class("clipboard-link-caption");
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.preview_cancel.borrow_mut().push(cancel.clone());
        let cancelling = cancel.clone();
        card.connect_unrealize(move |_| {
            cancelling.store(true, Ordering::Release);
        });
        let weak_box = preview_box.downgrade();
        let weak = Rc::downgrade(self);
        let id = entry.id;
        let url = entry.content.text.trim().to_owned();
        let generation = self.generation.get();
        let service = self.service.clone();
        glib::spawn_future_local(async move {
            let cached = service.cached_preview(id).await.ok().flatten();
            let result = if let Some(preview) = cached {
                Ok(preview)
            } else {
                let cancel = cancel.clone();
                gio::spawn_blocking(move || preview::link(&url, cancel))
                    .await
                    .map_err(|_| "Preview worker failed".to_owned())
                    .and_then(|value| value)
            };
            let Some(this) = weak.upgrade().filter(|this| {
                this.visible.get()
                    && this.generation.get() == generation
                    && !cancel.load(Ordering::Acquire)
            }) else {
                return;
            };
            if let Ok(preview) = result
                && let Some(container) = weak_box.upgrade()
            {
                let settings = this.service.settings.child("link-item");
                if settings.boolean("show-link-preview-image")
                    && let Some(bytes) = &preview.image
                {
                    let picture = picture();
                    picture.set_size_request(-1, 60);
                    this.image(
                        &picture,
                        bytes.clone(),
                        settings.string("link-preview-image-background-size") == "cover",
                        None,
                    );
                    container.append(&picture);
                }
                let labels = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
                let title = label(&preview.title);
                title.add_css_class("heading");
                labels.append(&title);
                labels.append(&label(&preview.description));
                container.append(&labels);
                let _ = service.save_preview(id, preview).await;
            }
        });
    }
    fn files(self: &Rc<Self>, card: &gtk4::Box, container: &gtk4::Box, entry: &Entry) {
        let files = entry
            .content
            .text
            .lines()
            .map(gio::File::for_uri)
            .collect::<Vec<_>>();
        let settings = self.service.settings.child("file-item");
        if entry.content.kind == Kind::Files {
            if let Some(parent) = files.first().and_then(|file| file.parent()) {
                let path = label(
                    &parent
                        .path()
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_else(|| parent.uri().to_string()),
                );
                path.add_css_class("clipboard-file-path");
                container.append(&path);
            }
            let count = ((self.service.settings.int("item-height") - 60) / 23).max(1) as usize;
            let list = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            list.add_css_class("clipboard-files-list");
            container.append(&list);
            for file in files.iter().take(count) {
                list.append(&label(
                    &file
                        .basename()
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                ));
            }
            if files.len() > count {
                let more = label(&format!("{} more files", files.len() - count));
                more.add_css_class("clipboard-time");
                list.append(&more);
            }
            return;
        }
        let Some(file) = files.first().cloned() else {
            return;
        };
        let path = file
            .path()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.uri().to_string());
        let path_label = label(&path);
        path_label.add_css_class("clipboard-file-path");
        container.append(&path_label);
        let visibility = settings.string("file-preview-visibility");
        if visibility == "hidden" {
            return;
        }
        let excluded = settings
            .strv("file-preview-exclusion-patterns")
            .iter()
            .any(|pattern| glob::Pattern::new(pattern).is_ok_and(|pattern| pattern.matches(&path)));
        let types = settings.strv("file-preview-types");
        let show_text = types.iter().any(|value| value == "text");
        let show_image = types.iter().any(|value| value == "image");
        let thumbnail = types.iter().any(|value| value == "thumbnail");
        let show_preview = visibility != "file-info" && !excluded;
        let cover = settings.string("background-size") == "cover";
        let info = label("");
        info.add_css_class("clipboard-time");
        let preview_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        preview_box.set_vexpand(true);
        container.append(&preview_box);
        container.append(&info);
        let weak_box = preview_box.downgrade();
        let weak_info = info.downgrade();
        let weak_card = card.downgrade();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = file
                .query_info_future(
                    "standard::size,standard::content-type,thumbnail::path",
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                )
                .await;
            let Some(this) = weak.upgrade() else {
                return;
            };
            if weak_card.upgrade().is_none() {
                return;
            }
            if let Ok(metadata) = result {
                let mut description = glib::format_size(metadata.size() as u64).to_string();
                let kind = metadata.content_type().unwrap_or_default();
                let mut previewed = false;
                if show_preview && let Some(container) = weak_box.upgrade() {
                    if show_image && kind.starts_with("image/") {
                        let picture = picture();
                        picture.set_content_fit(if cover {
                            gtk4::ContentFit::Cover
                        } else {
                            gtk4::ContentFit::Contain
                        });
                        container.append(&picture);
                        let stream = file.read_future(glib::Priority::DEFAULT).await;
                        if let Ok(stream) = stream
                            && let Ok(pixbuf) =
                                gtk4::gdk_pixbuf::Pixbuf::from_stream_at_scale_future(
                                    &stream, 400, 250, true,
                                )
                                .await
                        {
                            picture.set_paintable(Some(&gdk::Texture::for_pixbuf(&pixbuf)));
                            previewed = true;
                        }
                    } else if show_text
                        && (kind.starts_with("text/")
                            || kind.contains("json")
                            || kind.contains("javascript"))
                    {
                        if let Ok(stream) = file.read_future(glib::Priority::DEFAULT).await
                            && let Ok(bytes) = stream
                                .read_bytes_future(10_000, glib::Priority::DEFAULT)
                                .await
                        {
                            container.add_css_class("clipboard-text-preview");
                            previewed = true;
                            let mut content = way_shell_clipboard::Content::text(
                                &String::from_utf8_lossy(&bytes),
                            );
                            let language = file
                                .basename()
                                .and_then(|path| {
                                    path.extension()
                                        .map(|value| value.to_string_lossy().into_owned())
                                })
                                .and_then(|ext| {
                                    way_shell_clipboard::content::syntaxes()
                                        .find_syntax_by_extension(&ext)
                                })
                                .map(|syntax| syntax.name.clone());
                            if let Some(language) = language {
                                content.kind = Kind::Code;
                                content.metadata = serde_json::json!({"language":language});
                                this.code(
                                    &container,
                                    &Entry {
                                        id: 0,
                                        content,
                                        title: String::new(),
                                        pinned: false,
                                        tag: None,
                                        copied: 0,
                                    },
                                    true,
                                );
                            } else {
                                container.append(&label(&content.text));
                            }
                        }
                    } else if thumbnail
                        && let Some(path) = metadata.attribute_byte_string("thumbnail::path")
                    {
                        let picture = gtk4::Picture::for_filename(path.as_str());
                        picture.set_can_shrink(true);
                        container.append(&picture);
                        previewed = true;
                    }
                }
                if kind.starts_with("image/")
                    && let Some(path) = file.path()
                    && let Ok(Some((_, width, height))) =
                        gtk4::gdk_pixbuf::Pixbuf::file_info_future(path).await
                {
                    description.push_str(&format!(" · {width} × {height}"));
                }
                if (kind.starts_with("audio/") || kind.starts_with("video/"))
                    && let Some(path) = file.path()
                {
                    use std::ffi::OsStr;
                    if let Ok(process) = gio::Subprocess::newv(
                        &[
                            OsStr::new("ffprobe"),
                            OsStr::new("-v"),
                            OsStr::new("quiet"),
                            OsStr::new("-show_entries"),
                            OsStr::new("format=duration"),
                            OsStr::new("-of"),
                            OsStr::new("default=noprint_wrappers=1:nokey=1"),
                            path.as_os_str(),
                        ],
                        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE,
                    ) {
                        this.processes.borrow_mut().push(process.clone());
                        match glib::future_with_timeout(
                            Duration::from_secs(2),
                            process.communicate_utf8_future(None),
                        )
                        .await
                        {
                            Ok(Ok((Some(output), _))) => {
                                if let Ok(seconds) = output.trim().parse::<f64>()
                                    && seconds.is_finite()
                                    && seconds >= 0.0
                                {
                                    let seconds = seconds as u64;
                                    description.push_str(&format!(
                                        " · {}m {}s",
                                        seconds / 60,
                                        seconds % 60
                                    ));
                                }
                            }
                            Err(_) => process.force_exit(),
                            _ => {}
                        }
                        this.processes
                            .borrow_mut()
                            .retain(|candidate| candidate != &process);
                    }
                }
                if let Some(info) = weak_info.upgrade() {
                    info.set_text(&description);
                    info.set_visible(
                        visibility == "file-info"
                            || visibility == "file-preview-and-file-info"
                            || (visibility == "file-preview-or-file-info" && !previewed),
                    );
                }
            } else if let Some(info) = weak_info.upgrade() {
                info.set_text("File is unavailable");
            }
        });
    }
    pub(super) fn edit(self: &Rc<Self>, id: i64, title_only: bool) {
        let Some(entry) = self.service.entry(id) else {
            return;
        };
        if !title_only && !matches!(entry.content.kind, Kind::Text | Kind::Code) {
            return;
        }
        let dialog = libadwaita::MessageDialog::new(
            Some(self.window()),
            Some(if title_only {
                "Edit title"
            } else {
                "Edit clipboard item"
            }),
            None,
        );
        dialog.add_css_class("clipboard-edit-dialog");
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        let editor = gtk4::TextView::new();
        editor.set_monospace(entry.content.kind == Kind::Code);
        editor.set_wrap_mode(gtk4::WrapMode::WordChar);
        editor.buffer().set_text(if title_only {
            &entry.title
        } else {
            &entry.content.text
        });
        let scroller = gtk4::ScrolledWindow::new();
        scroller.set_min_content_height(if title_only { 60 } else { 250 });
        scroller.set_min_content_width(400);
        scroller.set_child(Some(&editor));
        content.append(&scroller);
        let languages = std::iter::once("None".to_owned())
            .chain(
                way_shell_clipboard::content::syntaxes()
                    .syntaxes()
                    .iter()
                    .map(|syntax| syntax.name.clone()),
            )
            .collect::<Vec<_>>();
        let names = languages.iter().map(String::as_str).collect::<Vec<_>>();
        let language = gtk4::DropDown::from_strings(&names);
        language.set_enable_search(true);
        if let Some(current) = entry.content.metadata["language"].as_str() {
            language.set_selected(
                languages
                    .iter()
                    .position(|value| value == current)
                    .unwrap_or(0) as u32,
            );
        }
        language.set_visible(!title_only && entry.content.kind == Kind::Code);
        content.append(&language);
        dialog.set_extra_child(Some(&content));
        dialog.add_responses(&[("cancel", "Cancel"), ("save", "Save")]);
        dialog.set_default_response(Some("save"));
        dialog.set_close_response("cancel");
        let service = self.service.clone();
        dialog.connect_response(None,move |dialog,response|{if response=="save"{let mut entry=entry.clone();let buffer=editor.buffer();let text=buffer.text(&buffer.start_iter(),&buffer.end_iter(),true).to_string();if title_only{entry.title=text;}else{entry.content.set_text(text);if entry.content.kind==Kind::Code{entry.content.metadata=serde_json::json!({"language":languages.get(language.selected() as usize).filter(|value|value.as_str()!="None")});}}service.update(entry);}dialog.close();});
        self.present_dialog(&dialog);
    }
}
pub(super) fn label(text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(pango::WrapMode::WordChar);
    label.set_ellipsize(pango::EllipsizeMode::End);
    label.set_lines(8);
    label
}
fn picture() -> gtk4::Picture {
    let picture = gtk4::Picture::new();
    picture.add_css_class("clipboard-image-preview");
    picture.set_overflow(gtk4::Overflow::Hidden);
    picture.set_can_shrink(true);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture
}
fn relative_time(copied: i64) -> String {
    let seconds = (now() - copied).max(0) / 1000;
    match seconds {
        0..=59 => "just now".into(),
        60..=119 => "1 minute ago".into(),
        120..=3599 => format!("{} minutes ago", seconds / 60),
        3600..=7199 => "1 hour ago".into(),
        7200..=86399 => format!("{} hours ago", seconds / 3600),
        86400..=172799 => "1 day ago".into(),
        _ => format!("{} days ago", seconds / 86400),
    }
}
