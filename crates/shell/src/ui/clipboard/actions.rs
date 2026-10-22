use super::*;
use std::{
    ffi::OsStr,
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
};
use way_shell_clipboard::actions::Action;
impl ClipboardDialog {
    pub(super) fn item_menu(self: &Rc<Self>, id: i64) -> gtk::Popover {
        let popover = gtk::Popover::new();
        popover.add_css_class("clipboard-popover");
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let tag_names: Vec<_> = std::iter::once("No tag").chain(TAGS).collect();
        let tag = gtk::SingleSelection::new(Some(gtk::StringList::new(&tag_names)));
        tag.set_selected(
            self.service
                .entry(id)
                .and_then(|entry| {
                    TAGS.iter()
                        .position(|tag| entry.tag.as_deref() == Some(tag))
                })
                .map_or(0, |index| index as u32 + 1),
        );
        menu.append(&tag_picker(&tag));
        let weak = Rc::downgrade(self);
        let weak_popover = popover.downgrade();
        tag.connect_selected_notify(move |selection| {
            if let Some(this) = weak.upgrade()
                && let Some(mut entry) = this.service.entry(id)
            {
                entry.tag = selection
                    .selected()
                    .checked_sub(1)
                    .and_then(|index| TAGS.get(index as usize))
                    .map(|tag| (*tag).to_owned());
                this.service.update(entry);
            }
            if let Some(popover) = weak_popover.upgrade() {
                popover.popdown();
            }
        });
        menu.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        for (title, title_only) in [("Edit", false), ("Edit Title", true)] {
            if !title_only
                && self
                    .service
                    .entry(id)
                    .is_none_or(|entry| !matches!(entry.content.kind, Kind::Text | Kind::Code))
            {
                continue;
            }
            let button = gtk::Button::with_label(title);
            button.add_css_class("clipboard-menu-action");
            if let Some(label) = button.child().and_downcast::<gtk::Label>() {
                label.set_xalign(0.0);
            }
            let weak = Rc::downgrade(self);
            let weak_popover = popover.downgrade();
            button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.edit(id, title_only);
                }
                if let Some(popover) = weak_popover.upgrade() {
                    popover.popdown();
                }
            });
            menu.append(&button);
        }
        if let Some(entry) = self.service.entry(id) {
            self.action_buttons(&menu, id, &entry, &self.service.actions().actions, &popover);
        }
        popover.set_child(Some(&menu));
        popover
    }
    fn action_buttons(
        self: &Rc<Self>,
        menu: &gtk::Box,
        id: i64,
        entry: &Entry,
        actions: &[Action],
        popover: &gtk::Popover,
    ) {
        for action in actions {
            if action.is_group() {
                let expander = gtk::Expander::new(Some(&action.name));
                let children = gtk::Box::new(gtk::Orientation::Vertical, 3);
                self.action_buttons(&children, id, entry, &action.actions, popover);
                expander.set_child(Some(&children));
                menu.append(&expander);
                continue;
            }
            if action.captures(entry).is_none() {
                continue;
            }
            let button = gtk::Button::with_label(&action.name);
            button.add_css_class("clipboard-menu-action");
            if let Some(label) = button.child().and_downcast::<gtk::Label>() {
                label.set_xalign(0.0);
            }
            let action = action.clone();
            let weak = Rc::downgrade(self);
            let weak_popover = popover.downgrade();
            button.connect_clicked(move |_| {
                if let Some(popover) = weak_popover.upgrade() {
                    popover.popdown();
                }
                if let Some(this) = weak.upgrade() {
                    this.run_action(id, action.clone());
                }
            });
            menu.append(&button);
        }
    }
    pub(super) fn show_item_menu(self: &Rc<Self>, id: i64) {
        let popover = self.item_menu(id);
        popover.set_parent(&self.list);
        popover.connect_closed(|popover| popover.unparent());
        popover.popup();
    }
    pub(super) fn default_action(self: &Rc<Self>, id: i64) {
        let Some(entry) = self.service.entry(id) else {
            return;
        };
        let config = self.service.actions();
        if let Some(action) = config
            .defaults
            .get(entry.content.kind.name())
            .and_then(|id| config.find(id))
        {
            self.run_action(id, action.clone());
        }
    }
    pub(super) fn run_action(self: &Rc<Self>, id: i64, action: Action) {
        let Some(entry) = self.service.entry(id) else {
            return;
        };
        let Some(captures) = action.captures(&entry) else {
            return;
        };
        if action.kind == "qrcode" {
            self.qrcode(&entry.content.text);
            return;
        }
        let target = self.target.borrow().clone();
        match action.kind.as_str() {
            "command" => {
                let mut args = vec![
                    "sh".to_owned(),
                    "-c".into(),
                    action.command.clone(),
                    "_".into(),
                ];
                args.extend(captures);
                let refs = args.iter().map(OsStr::new).collect::<Vec<_>>();
                let process = match gio::Subprocess::newv(
                    &refs,
                    gio::SubprocessFlags::STDIN_PIPE
                        | gio::SubprocessFlags::STDOUT_PIPE
                        | gio::SubprocessFlags::STDERR_PIPE,
                ) {
                    Ok(process) => process,
                    Err(error) => {
                        self.service.report(error.to_string());
                        return;
                    }
                };
                self.processes.borrow_mut().push(process.clone());
                self.hide();
                let generation = self.generation.get();
                let weak = Rc::downgrade(self);
                glib::spawn_future_local(async move {
                    let result = glib::future_with_timeout(
                        Duration::from_secs(30),
                        process.communicate_utf8_future(Some(entry.content.text)),
                    )
                    .await;
                    if result.is_err() {
                        process.force_exit();
                    }
                    let Some(this) = weak.upgrade().filter(|this| !this.stopped.get()) else {
                        return;
                    };
                    this.processes
                        .borrow_mut()
                        .retain(|candidate| candidate != &process);
                    if this.generation.get() != generation {
                        return;
                    }
                    match result {
                        Ok(Ok((stdout, stderr))) if process.is_successful() => {
                            let output = stdout.unwrap_or_default();
                            this.action_output(output.trim(), &action.output, target);
                            let _ = stderr;
                        }
                        Ok(Ok((_, stderr))) => this.service.report(
                            stderr
                                .map(|value| value.to_string())
                                .filter(|value| !value.is_empty())
                                .unwrap_or_else(|| "Clipboard action failed".into()),
                        ),
                        Ok(Err(error)) => this.service.report(error.to_string()),
                        Err(_) => this
                            .service
                            .report("Clipboard action exceeded its 30-second deadline".into()),
                    }
                });
            }
            "color" => match way_shell_clipboard::actions::convert_color(
                &entry.content.text,
                &action.space,
            ) {
                Ok(output) => {
                    self.hide();
                    self.action_output(&output, &action.output, target);
                }
                Err(error) => self.service.report(error),
            },
            "open" | "files" | "path" => {
                let uris = if entry.content.kind == Kind::Image {
                    match materialize(&entry) {
                        Ok(path) => {
                            self.temporary_images.borrow_mut().push(path.clone());
                            vec![gio::File::for_path(path).uri().to_string()]
                        }
                        Err(error) => {
                            self.service.report(error);
                            return;
                        }
                    }
                } else {
                    entry
                        .content
                        .text
                        .lines()
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                };
                if action.kind == "path" {
                    let paths = uris
                        .iter()
                        .map(|uri| {
                            gio::File::for_uri(uri)
                                .path()
                                .map(|path| path.to_string_lossy().into_owned())
                                .unwrap_or_else(|| uri.clone())
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.hide();
                    self.action_output(&paths, &action.output, target);
                } else if action.kind == "files" {
                    self.hide();
                    let service = self.service.clone();
                    glib::spawn_future_local(async move {
                        match gio::bus_get_future(gio::BusType::Session).await {
                            Ok(bus) => {
                                if let Err(error) = bus
                                    .call_future(
                                        Some("org.freedesktop.FileManager1"),
                                        "/org/freedesktop/FileManager1",
                                        "org.freedesktop.FileManager1",
                                        "ShowItems",
                                        Some(&(uris, "").to_variant()),
                                        None,
                                        gio::DBusCallFlags::NONE,
                                        2000,
                                    )
                                    .await
                                {
                                    service.report(error.to_string());
                                }
                            }
                            Err(error) => service.report(error.to_string()),
                        }
                    });
                } else {
                    self.hide();
                    for uri in uris {
                        let service = self.service.clone();
                        glib::spawn_future_local(async move {
                            if let Err(error) = gio::AppInfo::launch_default_for_uri_future(
                                &uri,
                                None::<&gio::AppLaunchContext>,
                            )
                            .await
                            {
                                service.report(error.to_string());
                            }
                        });
                    }
                }
            }
            _ => self.service.report("Unknown clipboard action".into()),
        }
    }
    fn action_output(self: &Rc<Self>, output: &str, mode: &str, target: Option<Toplevel>) {
        if mode == "ignore" || output.is_empty() {
            return;
        }
        match self.service.restore_text(output) {
            Ok(()) if mode == "paste" => self.schedule_paste(target),
            Err(error) => self.service.report(error),
            _ => {}
        }
    }
    fn qrcode(self: &Rc<Self>, text: &str) {
        let code = match qrcode::QrCode::new(text) {
            Ok(code) => code,
            Err(error) => {
                self.service.report(error.to_string());
                return;
            }
        };
        let modules = code.width();
        let size = (modules + 8) * 6;
        let mut pixels = vec![255u8; size * size * 4];
        for y in 0..modules {
            for x in 0..modules {
                if code[(x, y)] == qrcode::Color::Dark {
                    for dy in 0..6 {
                        for dx in 0..6 {
                            let at = (((y + 4) * 6 + dy) * size + (x + 4) * 6 + dx) * 4;
                            pixels[at..at + 3].fill(0);
                        }
                    }
                }
            }
        }
        let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_bytes(
            &glib::Bytes::from_owned(pixels),
            gtk::gdk_pixbuf::Colorspace::Rgb,
            true,
            8,
            size as i32,
            size as i32,
            size as i32 * 4,
        );
        let picture = gtk::Picture::for_paintable(&gdk::Texture::for_pixbuf(&pixbuf));
        picture.set_size_request(300, 300);
        picture.set_can_shrink(true);
        let dialog = adw::MessageDialog::new(Some(self.window()), Some("QR Code"), None);
        dialog.add_css_class("clipboard-qr-dialog");
        let preview = gtk::Box::new(gtk::Orientation::Vertical, 0);
        preview.add_css_class("clipboard-qr-preview");
        preview.append(&picture);
        dialog.set_extra_child(Some(&preview));
        dialog.add_responses(&[("close", "Close"), ("copy", "Copy Image")]);
        dialog.set_close_response("close");
        let service = self.service.clone();
        dialog.connect_response(None, move |dialog, response| {
            if response == "copy" {
                match pixbuf.save_to_bufferv("png", &[]) {
                    Ok(bytes) => {
                        if let Err(error) = service
                            .wayland
                            .set_clipboard(vec![("image/png".into(), bytes)], false)
                        {
                            service.report(error);
                        }
                    }
                    Err(error) => service.report(error.to_string()),
                }
            }
            dialog.close();
        });
        self.present_dialog(&dialog);
    }
}
fn materialize(entry: &Entry) -> Result<PathBuf, String> {
    let runtime = glib::user_runtime_dir();
    let cpath = std::ffi::CString::new(runtime.as_os_str().as_encoded_bytes())
        .map_err(|e| e.to_string())?;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::statfs(cpath.as_ptr(), stat.as_mut_ptr()) } != 0
        || unsafe { stat.assume_init() }.f_type != libc::TMPFS_MAGIC
    {
        return Err("Opening a stored image requires a memory-backed runtime directory".into());
    }
    let directory = runtime.join(format!("way-shell-clipboard-{}", std::process::id()));
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)
        .map_err(|e| e.to_string())?;
    let extension = entry
        .content
        .mime
        .strip_prefix("image/")
        .filter(|value| value.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("img");
    let path = directory.join(format!(
        "{}-{}.{}",
        entry.id,
        glib::monotonic_time(),
        extension
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.write_all(&entry.content.bytes)
        .map_err(|e| e.to_string())?;
    Ok(path)
}
