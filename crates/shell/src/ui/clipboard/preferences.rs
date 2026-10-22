use super::*;
use way_shell_clipboard::actions::{Action, ActionConfig};
impl ClipboardDialog {
    pub(super) fn open_preferences(self: &Rc<Self>) {
        self.hide();
        if let Some(window) = self.preferences.borrow().as_ref() {
            window.present();
            return;
        }
        let window = adw::PreferencesWindow::new();
        // This window is independent of the history popup and panel controls.
        window.set_modal(false);
        window.set_title(Some("Clipboard Preferences"));
        window.set_default_size(700, 810);
        window.set_search_enabled(true);
        let settings = &self.service.settings;
        let general = page(&window, "General", "emblem-system-symbolic");
        group(
            &general,
            "History",
            settings,
            &[
                "database-backend",
                "database-location",
                "clipboard-history",
                "history-length",
                "history-time",
            ],
        );
        group(
            &general,
            "Feedback",
            settings,
            &[
                "indicator-display",
                "wiggle-indicator",
                "send-notification",
                "sound",
                "volume",
            ],
        );
        group(
            &general,
            "Behavior",
            settings,
            &[
                "remember-search",
                "exclude-pinned",
                "exclude-tagged",
                "protect-pinned",
                "protect-tagged",
                "sync-primary",
                "update-date-on-copy",
                "incognito",
            ],
        );
        group(
            &general,
            "Application Exclusions",
            settings,
            &["wmclass-exclusions"],
        );
        group(&general, "Automatic Paste", settings, &["terminal-apps"]);
        let customization = page(&window, "Customization", "applications-graphics-symbolic");
        let profiles = adw::PreferencesGroup::new();
        profiles.set_title("Profiles");
        let row = adw::ActionRow::new();
        row.set_title("Layout profile");
        for (name, compact) in [("Default", false), ("Compact", true)] {
            let button = gtk::Button::with_label(name);
            button.set_valign(gtk::Align::Center);
            let settings = settings.clone();
            button.connect_clicked(move |_| apply_profile(&settings, compact));
            row.add_suffix(&button);
        }
        profiles.add(&row);
        customization.add(&profiles);
        group(
            &customization,
            "Dialog",
            settings,
            &[
                "show-at-pointer",
                "clipboard-orientation",
                "clipboard-position-vertical",
                "clipboard-position-horizontal",
                "clipboard-size",
                "clipboard-margin-top",
                "clipboard-margin-right",
                "clipboard-margin-bottom",
                "clipboard-margin-left",
                "auto-hide-search",
                "show-scrollbar",
            ],
        );
        group(
            &customization,
            "Item",
            settings,
            &[
                "item-width",
                "item-height",
                "dynamic-item-height",
                "tab-width",
            ],
        );
        group(
            &customization,
            "Header",
            settings,
            &[
                "show-header",
                "header-controls-visibility",
                "show-item-title",
            ],
        );
        for (child, title, keys) in [
            (
                "text-item",
                "Text",
                vec!["show-text-info", "text-count-mode"],
            ),
            (
                "code-item",
                "Code",
                vec![
                    "syntax-highlighting",
                    "show-line-numbers",
                    "show-code-info",
                    "text-count-mode",
                ],
            ),
            (
                "image-item",
                "Image",
                vec!["show-image-info", "background-size"],
            ),
            (
                "file-item",
                "File",
                vec![
                    "file-preview-visibility",
                    "file-preview-types",
                    "file-preview-exclusion-patterns",
                    "background-size",
                    "syntax-highlighting",
                    "show-line-numbers",
                ],
            ),
            (
                "link-item",
                "Link",
                vec![
                    "show-link-preview",
                    "show-link-preview-image",
                    "link-preview-image-background-size",
                    "link-preview-orientation",
                    "link-preview-exclusion-patterns",
                ],
            ),
            (
                "character-item",
                "Character",
                vec!["max-characters", "show-unicode"],
            ),
        ] {
            group(&customization, title, &settings.child(child), &keys);
        }
        group(
            &customization,
            "Theme",
            &settings.child("theme"),
            &[
                "theme",
                "color-scheme",
                "custom-color-scheme",
                "custom-bg-color",
                "custom-fg-color",
                "custom-card-bg-color",
                "custom-search-bg-color",
            ],
        );
        group(
            &customization,
            "Custom Stylesheet",
            settings,
            &["custom-css"],
        );
        let shortcuts = page(&window, "Shortcuts", "input-keyboard-symbolic");
        let global = adw::PreferencesGroup::new();
        global.set_title("Global Shortcuts");
        global.set_description(Some("Global bindings are configured in Sway or Niri. Use way-sh clipboard open and way-sh clipboard incognito-toggle."));
        let detected = gtk::Label::new(None);
        detected.set_wrap(true);
        detected.set_xalign(0.0);
        global.add(&detected);
        shortcuts.add(&global);
        let weak_label = detected.downgrade();
        let source = self.shortcuts.clone();
        let id = self.shortcuts.on_changed(move || {
            if let Some(label) = weak_label.upgrade() {
                let text = source
                    .data()
                    .rows
                    .iter()
                    .filter(|row| row.description.to_lowercase().contains("clipboard"))
                    .map(|row| format!("{}  —  {}", row.keys.join(" / "), row.description))
                    .collect::<Vec<_>>()
                    .join("\n");
                label.set_text(if text.is_empty() {
                    "No clipboard bindings detected"
                } else {
                    &text
                });
            }
        });
        self.shortcuts.set_visible(true);
        let source = self.shortcuts.clone();
        window.connect_destroy(move |_| {
            source.disconnect(id);
            source.set_visible(false);
        });
        group(
            &shortcuts,
            "Dialog",
            settings,
            &["open-clipboard-dialog-behavior"],
        );
        group(
            &shortcuts,
            "Item",
            settings,
            &[
                "pin-item-shortcut",
                "delete-item-shortcut",
                "edit-item-shortcut",
                "edit-title-shortcut",
                "open-menu-shortcut",
                "middle-click-action",
                "swap-copy-shortcut",
            ],
        );
        group(
            &shortcuts,
            "Search",
            settings,
            &["toggle-pinned-search-shortcut", "swap-scroll-shortcut"],
        );
        let navigation = adw::PreferencesGroup::new();
        navigation.set_title("Navigation");
        for (title, keys) in [
            (
                "Navigate items",
                "Tab / Shift+Tab / Arrow keys / Home / End",
            ),
            ("Paste item", "Enter / Space"),
            ("Copy item", "Shift+Enter / Shift+Space"),
            ("Run default action", "Ctrl+Enter / Ctrl+Space"),
            ("Jump to item", "Ctrl+1 … Ctrl+0"),
            ("Cycle item type", "Ctrl+Tab / Ctrl+Shift+Tab"),
            ("Cycle item tag", "Ctrl+` / Ctrl+Shift+`"),
            ("Choose tag in item menu", "0 … 9"),
        ] {
            let row = adw::ActionRow::new();
            row.set_title(title);
            row.set_subtitle(keys);
            navigation.add(&row);
        }
        shortcuts.add(&navigation);
        let actions = page(&window, "Actions", "system-run-symbolic");
        let group = adw::PreferencesGroup::new();
        group.set_title("Clipboard Actions");
        actions.add(&group);
        let config = Rc::new(RefCell::new(self.service.actions()));
        render_actions(&group, self.service.clone(), &window, config);
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |window| {
            if let Some(this) = weak.upgrade() {
                this.preferences.borrow_mut().take();
            }
            window.destroy();
            glib::Propagation::Stop
        });
        self.preferences.replace(Some(window.clone()));
        window.present();
    }
}
fn page(window: &adw::PreferencesWindow, title: &str, icon: &str) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title(title);
    page.set_icon_name(Some(icon));
    window.add(&page);
    page
}
fn group(page: &adw::PreferencesPage, title: &str, settings: &gio::Settings, keys: &[&str]) {
    let group = adw::PreferencesGroup::new();
    group.set_title(title);
    for key in keys {
        setting_row(&group, settings, key);
    }
    page.add(&group);
}
fn setting_row(group: &adw::PreferencesGroup, settings: &gio::Settings, key: &str) {
    let schema = settings.settings_schema().unwrap();
    let definition = schema.key(key);
    let title = definition
        .summary()
        .map(|value| value.to_string())
        .unwrap_or_else(|| key.replace('-', " "));
    let range = definition.range();
    let range_type = range.child_value(0).str().unwrap_or_default().to_owned();
    let payload = range.child_value(1).as_variant().unwrap();
    if range_type == "flags"
        && let Some(values) = payload.get::<Vec<String>>()
    {
        for flag in values {
            let row = adw::SwitchRow::new();
            row.set_title(&format!("{flag} previews"));
            let matching = flag.clone();
            let source = settings.clone();
            let setting_key = key.to_owned();
            settings
                .bind(key, &row, "active")
                .mapping(move |value, _| {
                    Some(value.get::<Vec<String>>()?.contains(&matching).to_value())
                })
                .set_mapping(move |value, _| {
                    let enabled = value.get::<bool>().ok()?;
                    let mut flags = source
                        .strv(&setting_key)
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>();
                    flags.retain(|value| value != &flag);
                    if enabled {
                        flags.push(flag.clone());
                    }
                    Some(flags.to_variant())
                })
                .build();
            group.add(&row);
        }
        return;
    }
    if range_type == "enum"
        && let Some(values) = payload.get::<Vec<String>>()
    {
        let row = adw::ComboRow::new();
        row.set_title(&title);
        let strings = values.iter().map(String::as_str).collect::<Vec<_>>();
        row.set_model(Some(&gtk::StringList::new(&strings)));
        let reverse = values.clone();
        settings
            .bind(key, &row, "selected")
            .mapping(move |value, _| {
                Some(
                    (values
                        .iter()
                        .position(|item| Some(item.as_str()) == value.str())
                        .unwrap_or(0) as u32)
                        .to_value(),
                )
            })
            .set_mapping(move |value, _| {
                reverse
                    .get(value.get::<u32>().ok()? as usize)
                    .map(ToVariant::to_variant)
            })
            .build();
        add_reset(&row, settings, key);
        group.add(&row);
        return;
    }
    match settings.value(key).type_().as_str() {
        "b" => {
            let row = adw::SwitchRow::new();
            row.set_title(&title);
            settings.bind(key, &row, "active").build();
            group.add(&row);
        }
        "i" | "d" => {
            let integer = settings.value(key).type_().as_str() == "i";
            let bounds = if integer {
                payload
                    .get::<(i32, i32)>()
                    .map(|(min, max)| (f64::from(min), f64::from(max)))
            } else {
                payload.get::<(f64, f64)>()
            }
            .unwrap_or((0.0, 10000.0));
            let adjustment = gtk::Adjustment::new(0.0, bounds.0, bounds.1, 1.0, 10.0, 0.0);
            let spin = gtk::SpinButton::new(Some(&adjustment), 1.0, if integer { 0 } else { 1 });
            spin.set_valign(gtk::Align::Center);
            let row = adw::ActionRow::new();
            row.set_title(&title);
            row.add_suffix(&spin);
            settings
                .bind(key, &spin, "value")
                .mapping(move |value, _| {
                    Some(
                        if integer {
                            f64::from(value.get::<i32>()?)
                        } else {
                            value.get::<f64>()?
                        }
                        .to_value(),
                    )
                })
                .set_mapping(move |value, _| {
                    let value = value.get::<f64>().ok()?;
                    Some(if integer {
                        (value as i32).to_variant()
                    } else {
                        value.to_variant()
                    })
                })
                .build();
            add_reset(&row, settings, key);
            group.add(&row);
        }
        "s" | "as" => {
            let array = settings.value(key).type_().as_str() == "as";
            let row = adw::EntryRow::new();
            row.set_title(&title);
            settings
                .bind(key, &row, "text")
                .mapping(move |value, _| {
                    Some(
                        if array {
                            value.get::<Vec<String>>()?.join("; ")
                        } else {
                            value.str()?.to_owned()
                        }
                        .to_value(),
                    )
                })
                .set_mapping(move |value, _| {
                    let text = value.get::<String>().ok()?;
                    Some(if array {
                        text.split(';')
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .collect::<Vec<_>>()
                            .to_variant()
                    } else {
                        text.to_variant()
                    })
                })
                .build();
            let button = icon_button("edit-undo-symbolic", "Reset to default");
            let settings = settings.clone();
            let name = key.to_owned();
            button.connect_clicked(move |_| settings.reset(&name));
            row.add_suffix(&button);
            group.add(&row);
        }
        _ => {}
    }
}
fn add_reset(row: &impl IsA<adw::ActionRow>, settings: &gio::Settings, key: &str) {
    let button = icon_button("edit-undo-symbolic", "Reset to default");
    button.set_valign(gtk::Align::Center);
    let settings = settings.clone();
    let key = key.to_owned();
    button.connect_clicked(move |_| settings.reset(&key));
    row.add_suffix(&button);
}
pub(super) fn apply_profile(settings: &gio::Settings, compact: bool) {
    for (key, value) in [
        ("show-at-pointer", compact),
        ("auto-hide-search", compact),
        ("dynamic-item-height", compact),
        ("show-header", !compact),
    ] {
        let _ = settings.set_boolean(key, value);
    }
    for (key, value) in [
        ("clipboard-size", 500),
        ("item-width", if compact { 300 } else { 250 }),
        ("item-height", if compact { 100 } else { 170 }),
    ] {
        let _ = settings.set_int(key, value);
    }
    for (key, value) in [
        (
            "clipboard-orientation",
            if compact { "vertical" } else { "horizontal" },
        ),
        (
            "clipboard-position-vertical",
            if compact { "fill" } else { "top" },
        ),
        (
            "clipboard-position-horizontal",
            if compact { "top" } else { "fill" },
        ),
        (
            "header-controls-visibility",
            if compact {
                "visible-on-hover"
            } else {
                "visible"
            },
        ),
    ] {
        let _ = settings.set_string(key, value);
    }
    let _ = settings.child("file-item").set_string(
        "file-preview-visibility",
        if compact {
            "file-info"
        } else {
            "file-preview-or-file-info"
        },
    );
    let _ = settings.child("link-item").set_string(
        "link-preview-orientation",
        if compact { "horizontal" } else { "vertical" },
    );
}

fn save(service: &ClipboardService, config: &ActionConfig) -> Result<(), String> {
    config.validate()?;
    service
        .settings
        .set_string(
            "action-config",
            &serde_json::to_string(config).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
}
fn at_mut<'a>(actions: &'a mut [Action], path: &[usize]) -> Option<&'a mut Action> {
    let (index, rest) = path.split_first()?;
    let action = actions.get_mut(*index)?;
    if rest.is_empty() {
        Some(action)
    } else {
        at_mut(&mut action.actions, rest)
    }
}
fn render_actions(
    group: &adw::PreferencesGroup,
    service: Rc<ClipboardService>,
    window: &adw::PreferencesWindow,
    config: Rc<RefCell<ActionConfig>>,
) {
    // Replace only this group's list, retaining the window's search/navigation state.
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    group.add(&list);
    action_rows(
        &list,
        &service,
        window,
        &config,
        &config.borrow().actions,
        &[],
    );
    let add = gtk::Button::with_label("Add Action");
    let service_add = service.clone();
    let window_add = window.clone();
    let config_add = config.clone();
    let list_weak = list.downgrade();
    add.connect_clicked(move |_| {
        edit_action(
            service_add.clone(),
            &window_add,
            config_add.clone(),
            None,
            list_weak.clone(),
            None,
        )
    });
    group.add(&add);
    let defaults = gtk::Box::new(gtk::Orientation::Vertical, 4);
    for kind in Kind::ALL {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(Some(&format!("{} default action", kind.name())));
        label.set_hexpand(true);
        label.set_xalign(0.0);
        row.append(&label);
        let actions = config
            .borrow()
            .actions
            .iter()
            .flat_map(flatten_actions)
            .filter(|action| action.types.is_empty() || action.types.contains(&kind))
            .cloned()
            .collect::<Vec<_>>();
        let names = std::iter::once("None")
            .chain(actions.iter().map(|action| action.name.as_str()))
            .collect::<Vec<_>>();
        let dropdown = gtk::DropDown::from_strings(&names);
        let ids = actions
            .iter()
            .map(|action| action.id.clone())
            .collect::<Vec<_>>();
        dropdown.set_selected(
            config
                .borrow()
                .defaults
                .get(kind.name())
                .and_then(|id| ids.iter().position(|candidate| candidate == id))
                .map_or(0, |index| index as u32 + 1),
        );
        let ids = Rc::new(RefCell::new(ids));
        let updating = Rc::new(Cell::new(false));
        let weak = dropdown.downgrade();
        let weak_service = Rc::downgrade(&service);
        let refresh_ids = ids.clone();
        let guard = updating.clone();
        let refresh_config = config.clone();
        let handler = service
            .settings
            .connect_changed(Some("action-config"), move |_, _| {
                let weak = weak.clone();
                let source = weak_service.clone();
                let ids = refresh_ids.clone();
                let updating = guard.clone();
                let config = refresh_config.clone();
                glib::idle_add_local_once(move || {
                    let (Some(dropdown), Some(service)) = (weak.upgrade(), source.upgrade()) else {
                        return;
                    };
                    let current = service.actions();
                    let actions = current
                        .actions
                        .iter()
                        .flat_map(flatten_actions)
                        .filter(|action| action.types.is_empty() || action.types.contains(&kind))
                        .collect::<Vec<_>>();
                    let names = std::iter::once("None")
                        .chain(actions.iter().map(|action| action.name.as_str()))
                        .collect::<Vec<_>>();
                    updating.set(true);
                    ids.replace(actions.iter().map(|action| action.id.clone()).collect());
                    dropdown.set_model(Some(&gtk::StringList::new(&names)));
                    dropdown.set_selected(
                        current
                            .defaults
                            .get(kind.name())
                            .and_then(|id| ids.borrow().iter().position(|value| value == id))
                            .map_or(0, |index| index as u32 + 1),
                    );
                    config.replace(current);
                    updating.set(false);
                });
            });
        let source = service.settings.clone();
        let handler = RefCell::new(Some(handler));
        window.connect_destroy(move |_| {
            if let Some(handler) = handler.borrow_mut().take() {
                source.disconnect(handler);
            }
        });
        let config = config.clone();
        let service = service.clone();
        dropdown.connect_selected_notify(move |dropdown| {
            if updating.get() {
                return;
            }
            let mut config = config.borrow_mut();
            let ids = ids.borrow();
            if let Some(id) = dropdown
                .selected()
                .checked_sub(1)
                .and_then(|index| ids.get(index as usize))
            {
                config.defaults.insert(kind.name().into(), id.clone());
            } else {
                config.defaults.remove(kind.name());
            }
            if let Err(error) = save(&service, &config) {
                service.report(error);
            }
        });
        row.append(&dropdown);
        defaults.append(&row);
    }
    group.add(&defaults);
}
fn action_rows(
    list: &gtk::ListBox,
    service: &Rc<ClipboardService>,
    window: &adw::PreferencesWindow,
    config: &Rc<RefCell<ActionConfig>>,
    actions: &[Action],
    prefix: &[usize],
) {
    for (index, action) in actions.iter().enumerate() {
        let mut path = prefix.to_vec();
        path.push(index);
        let row = adw::ActionRow::new();
        row.set_title(&action.name);
        row.set_subtitle(if !action.is_group() {
            &action.kind
        } else {
            "Submenu"
        });
        let edit = icon_button("document-edit-symbolic", "Edit action");
        edit.set_valign(gtk::Align::Center);
        let edit_service = service.clone();
        let edit_window = window.clone();
        let edit_config = config.clone();
        let weak = list.downgrade();
        let edit_path = path.clone();
        edit.connect_clicked(move |_| {
            edit_action(
                edit_service.clone(),
                &edit_window,
                edit_config.clone(),
                Some(edit_path.clone()),
                weak.clone(),
                None,
            )
        });
        row.add_suffix(&edit);
        for (icon, title, delta) in [
            ("go-up-symbolic", "Move up", -1isize),
            ("go-down-symbolic", "Move down", 1),
        ] {
            let button = icon_button(icon, title);
            button.set_valign(gtk::Align::Center);
            button.set_sensitive(
                (index as isize + delta) >= 0 && (index as isize + delta) < actions.len() as isize,
            );
            let service = service.clone();
            let config = config.clone();
            let path = path.clone();
            let window = window.clone();
            let list = list.downgrade();
            button.connect_clicked(move |_| {
                let mut changed = config.borrow().clone();
                let siblings = if path.len() == 1 {
                    Some(&mut changed.actions)
                } else {
                    at_mut(&mut changed.actions, &path[..path.len() - 1])
                        .map(|parent| &mut parent.actions)
                };
                if let Some(siblings) = siblings {
                    siblings.swap(index, (index as isize + delta) as usize);
                }
                if let Err(error) = save(&service, &changed) {
                    service.report(error);
                    return;
                }
                config.replace(changed);
                if let Some(list) = list.upgrade() {
                    refresh_actions(&list, &service, &window, &config);
                }
            });
            row.add_suffix(&button);
        }
        if action.is_group() {
            let add = icon_button("list-add-symbolic", "Add action to submenu");
            add.set_valign(gtk::Align::Center);
            let service = service.clone();
            let window = window.clone();
            let config = config.clone();
            let parent = path.clone();
            let list = list.downgrade();
            add.connect_clicked(move |_| {
                edit_action(
                    service.clone(),
                    &window,
                    config.clone(),
                    None,
                    list.clone(),
                    Some(parent.clone()),
                )
            });
            row.add_suffix(&add);
        }
        list.append(&row);
        if action.is_group() {
            action_rows(list, service, window, config, &action.actions, &path);
        }
    }
}
fn refresh_actions(
    list: &gtk::ListBox,
    service: &Rc<ClipboardService>,
    window: &adw::PreferencesWindow,
    config: &Rc<RefCell<ActionConfig>>,
) {
    while let Some(row) = list.first_child() {
        list.remove(&row);
    }
    action_rows(list, service, window, config, &config.borrow().actions, &[]);
}
fn edit_action(
    service: Rc<ClipboardService>,
    window: &adw::PreferencesWindow,
    config: Rc<RefCell<ActionConfig>>,
    path: Option<Vec<usize>>,
    list: glib::WeakRef<gtk::ListBox>,
    parent: Option<Vec<usize>>,
) {
    let existing = path
        .as_ref()
        .and_then(|path| at_mut(&mut config.borrow_mut().actions, path).cloned());
    let action=existing.unwrap_or_else(||serde_json::from_value(serde_json::json!({"kind":"command","id":format!("action-{}",glib::monotonic_time()),"name":"New Action","output":"ignore"})).unwrap());
    let dialog = adw::MessageDialog::new(Some(window), Some("Clipboard Action"), None);
    let fields = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let entries = [
        ("Name", action.name.clone()),
        ("Command", action.command.clone()),
        ("Regex pattern", action.pattern.clone().unwrap_or_default()),
        (
            "Shortcuts (separated by semicolons)",
            action.shortcut.join("; "),
        ),
    ]
    .into_iter()
    .map(|(title, value)| {
        let row = adw::EntryRow::new();
        row.set_title(title);
        row.set_text(&value);
        fields.append(&row);
        row
    })
    .collect::<Vec<_>>();
    let kinds = [
        "command", "color", "qrcode", "open", "files", "path", "submenu",
    ];
    let kind = gtk::DropDown::from_strings(&[
        "Command",
        "Color conversion",
        "QR Code",
        "Open with Default",
        "Open with Files",
        "Paste as Path",
        "Submenu",
    ]);
    kind.set_selected(
        kinds
            .iter()
            .position(|value| *value == action.kind || (*value == "submenu" && action.is_group()))
            .unwrap_or(0) as u32,
    );
    kind.set_sensitive(!action.is_group());
    fields.append(&kind);
    let outputs = ["ignore", "copy", "paste"];
    let output = gtk::DropDown::from_strings(&outputs);
    output.set_selected(
        outputs
            .iter()
            .position(|value| *value == action.output)
            .unwrap_or(0) as u32,
    );
    fields.append(&output);
    let spaces = ["hex", "rgb", "hsl", "hwb", "lab", "lch", "oklab", "oklch"];
    let space = gtk::DropDown::from_strings(&spaces);
    space.set_selected(
        spaces
            .iter()
            .position(|value| *value == action.space)
            .unwrap_or(0) as u32,
    );
    fields.append(&space);
    let checks = Kind::ALL
        .iter()
        .map(|kind| {
            let check = gtk::CheckButton::with_label(kind.name());
            check.set_active(action.types.contains(kind));
            fields.append(&check);
            (*kind, check)
        })
        .collect::<Vec<_>>();
    let error = gtk::Label::new(None);
    error.set_wrap(true);
    error.add_css_class("error");
    fields.append(&error);
    dialog.set_extra_child(Some(&fields));
    dialog.add_responses(&[("cancel", "Cancel"), ("save", "Save")]);
    if path.is_some() {
        dialog.add_response("delete", "Delete");
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    }
    dialog.set_close_response("cancel");
    let weak_dialog = dialog.downgrade();
    let weak_error = error.downgrade();
    let weak_fields = entries.iter().map(ObjectExt::downgrade).collect::<Vec<_>>();
    let weak_kind = kind.downgrade();
    let weak_output = output.downgrade();
    let weak_space = space.downgrade();
    let weak_checks = checks
        .iter()
        .map(|(_, check)| check.downgrade())
        .collect::<Vec<_>>();
    let validate = Rc::new(move || {
        let (Some(dialog), Some(error), Some(kind)) = (
            weak_dialog.upgrade(),
            weak_error.upgrade(),
            weak_kind.upgrade(),
        ) else {
            return;
        };
        let fields = weak_fields
            .iter()
            .filter_map(glib::WeakRef::upgrade)
            .collect::<Vec<_>>();
        if fields.len() != 4 {
            return;
        }
        let selected = kinds[kind.selected() as usize];
        fields[1].set_visible(selected == "command");
        fields[2].set_visible(selected != "submenu");
        fields[3].set_visible(selected != "submenu");
        if let Some(output) = weak_output.upgrade() {
            output.set_visible(matches!(selected, "command" | "color" | "path"));
        }
        if let Some(space) = weak_space.upgrade() {
            space.set_visible(selected == "color");
        }
        for check in &weak_checks {
            if let Some(check) = check.upgrade() {
                check.set_visible(selected != "submenu");
            }
        }
        let pattern = fields[2].text();
        let message = if fields[0].text().trim().is_empty() {
            Some("Enter an action name".to_owned())
        } else if !pattern.is_empty() {
            regress::Regex::new(&pattern)
                .err()
                .map(|error| error.to_string())
        } else {
            None
        };
        dialog.set_response_enabled("save", message.is_none());
        error.set_text(message.as_deref().unwrap_or_default());
    });
    for entry in &entries {
        let validate = validate.clone();
        entry.connect_changed(move |_| validate());
    }
    let changed = validate.clone();
    kind.connect_selected_notify(move |_| changed());
    validate();
    let window = window.clone();
    dialog.connect_response(None, move |dialog, response| {
        if response == "save" || response == "delete" {
            let mut changed = config.borrow().clone();
            if response == "delete" {
                if let Some(path) = &path {
                    if path.len() == 1 {
                        let removed = changed.actions.remove(path[0]);
                        changed.defaults.retain(|_, id| id != &removed.id);
                    } else if let Some(parent) =
                        at_mut(&mut changed.actions, &path[..path.len() - 1])
                    {
                        parent.actions.remove(path[path.len() - 1]);
                    }
                }
            } else {
                let mut action = action.clone();
                action.name = entries[0].text().to_string();
                action.command = entries[1].text().to_string();
                action.pattern =
                    Some(entries[2].text().to_string()).filter(|value| !value.is_empty());
                action.shortcut = entries[3]
                    .text()
                    .split(';')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect();
                action.kind = kinds[kind.selected() as usize].into();
                action.output = outputs[output.selected() as usize].into();
                action.space = spaces[space.selected() as usize].into();
                action.types = checks
                    .iter()
                    .filter(|(_, check)| check.is_active())
                    .map(|(kind, _)| *kind)
                    .collect();
                if let Some(path) = &path {
                    if let Some(existing) = at_mut(&mut changed.actions, path) {
                        *existing = action;
                    }
                } else if let Some(parent) = &parent {
                    if let Some(parent) = at_mut(&mut changed.actions, parent) {
                        parent.actions.push(action);
                    }
                } else {
                    changed.actions.push(action);
                }
            }
            // Removing a submenu also removes defaults referring to its descendants.
            let valid = changed
                .actions
                .iter()
                .flat_map(flatten_actions)
                .map(|action| action.id.clone())
                .collect::<std::collections::BTreeSet<_>>();
            changed.defaults.retain(|_, id| valid.contains(id));
            if let Err(message) = save(&service, &changed) {
                error.set_text(&message);
                return;
            }
            config.replace(changed);
            if let Some(list) = list.upgrade() {
                refresh_actions(&list, &service, &window, &config);
            }
        }
        dialog.close();
    });
    dialog.present();
}
