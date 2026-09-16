//! Real shared/app switcher widgets on the existing Sway/Niri component runner.
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};
use way_shell::{
    services::wayland::{ToplevelAction, WaylandService},
    ui::{
        app_switcher::AppSwitcher,
        switcher::{Switcher, SwitcherAction, SwitcherEntry},
    },
};

fn until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "switcher smoke test timed out");
        glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}

fn keys(window: &adw::Window) -> gtk::EventControllerKey {
    let controllers = window.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|i| {
            controllers
                .item(i)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .find(|controller| controller.name().as_deref() == Some("way-shell-switcher-keys"))
        .unwrap()
}

fn selected_button(widget: &gtk::Widget) -> Option<gtk::Button> {
    if widget.has_css_class("selected")
        && let Ok(button) = widget.clone().downcast::<gtk::Button>()
    {
        return Some(button);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(button) = selected_button(&widget) {
            return Some(button);
        }
        child = widget.next_sibling();
    }
    None
}

fn shared() -> Result<(), String> {
    let actions = Rc::new(RefCell::new(Vec::new()));
    let captured = actions.clone();
    let view = Switcher::new(true, true, move |action| captured.borrow_mut().push(action))?;
    view.set_entries(vec![
        SwitcherEntry {
            id: "opaque-7".into(),
            label: "Work Café".into(),
        },
        SwitcherEntry {
            id: "opaque-9".into(),
            label: "Browser".into(),
        },
        SwitcherEntry {
            id: "opaque-11".into(),
            label: "Work Chat".into(),
        },
    ]);
    view.show();
    until(|| view.window().is_mapped());
    view.search().set_text("work");
    assert_eq!(view.selected_id().as_deref(), Some("opaque-7"));
    let keys = keys(view.window());
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Down,
            &0u32,
            &gtk::gdk::ModifierType::empty()
        ]
    ));
    assert_eq!(view.selected_id().as_deref(), Some("opaque-11"));
    view.search().emit_by_name::<()>("next-match", &[]);
    assert_eq!(view.selected_id().as_deref(), Some("opaque-7"));
    view.search().emit_by_name::<()>("previous-match", &[]);
    assert_eq!(view.selected_id().as_deref(), Some("opaque-11"));
    view.search().emit_by_name::<()>("activate", &[]);
    assert_eq!(
        actions.borrow().last(),
        Some(&SwitcherAction::Activate {
            selected: Some("opaque-11".into()),
            text: "work".into(),
            raw: false
        })
    );
    view.search().set_text(" Café new ");
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Return,
            &0u32,
            &gtk::gdk::ModifierType::CONTROL_MASK
        ]
    ));
    assert_eq!(
        actions.borrow().last(),
        Some(&SwitcherAction::Activate {
            selected: None,
            text: " Café new ".into(),
            raw: true
        })
    );
    view.cancel();
    assert!(view.is_visible());
    assert!(view.search().text().is_empty());
    view.cancel();
    assert!(!view.is_visible());
    assert_eq!(actions.borrow().last(), Some(&SwitcherAction::Hidden));
    view.show();
    view.search().set_text("not-matching");
    view.navigate(true);
    assert_eq!(view.selected_id(), None);
    view.set_entries(vec![]);
    view.navigate(false);
    let window = view.window().clone();
    let search = view.search().clone();
    drop(view);
    assert!(!window.is_visible());
    let count = actions.borrow().len();
    search.emit_by_name::<()>("activate", &[]);
    assert_eq!(actions.borrow().len(), count);

    let rename = Switcher::new(false, false, |_| {})?;
    rename.show();
    rename.close();
    rename.show();
    assert!(!rename.is_visible());
    Ok(())
}

fn entry(id: &str, label: &str) -> SwitcherEntry {
    SwitcherEntry {
        id: id.into(),
        label: label.into(),
    }
}

fn shared_reentrant() -> Result<(), String> {
    let hidden = Rc::new(Cell::new(0));
    let captured = hidden.clone();
    let view = Switcher::new(true, true, move |action| {
        if action == SwitcherAction::Hidden {
            captured.set(captured.get() + 1);
        }
    })?;
    view.set_entries(vec![entry("old", "Old")]);
    view.show();
    let weak = Rc::downgrade(&view);
    let triggered = Cell::new(false);
    let handler = view.list().connect_row_selected(move |_, row| {
        if row.is_none()
            && !triggered.replace(true)
            && let Some(view) = weak.upgrade()
        {
            view.set_entries(vec![entry("newest", "Newest")]);
        }
    });
    view.set_entries(vec![entry("superseded", "Superseded")]);
    view.list().disconnect(handler);
    assert_eq!(
        view.selected_id().as_deref(),
        Some("newest"),
        "nested row replacement must win"
    );

    view.set_entries(vec![entry("old", "Old"), entry("match", "Match")]);
    let row = view.list().row_at_index(0).unwrap();
    let weak = Rc::downgrade(&view);
    let triggered = Cell::new(false);
    let handler = row.connect_visible_notify(move |row| {
        if !row.is_visible()
            && !triggered.replace(true)
            && let Some(view) = weak.upgrade()
        {
            view.set_entries(vec![entry("replacement", "Match replacement")]);
        }
    });
    view.search().set_text("Match");
    row.disconnect(handler);
    assert_eq!(view.selected_id().as_deref(), Some("replacement"));

    let weak = Rc::downgrade(&view);
    let reopened = Cell::new(false);
    let handler = view.window().connect_visible_notify(move |window| {
        if !window.is_visible()
            && !reopened.replace(true)
            && let Some(view) = weak.upgrade()
        {
            view.show();
            view.search().set_text("new query");
        }
    });
    view.hide();
    view.window().disconnect(handler);
    assert!(
        view.is_visible() && view.window().is_visible(),
        "a reentrant show wins over the interrupted hide"
    );
    assert_eq!(
        view.search().text(),
        "new query",
        "an obsolete hide cannot clear a reopened search"
    );
    assert_eq!(
        hidden.get(),
        0,
        "an interrupted hide must not emit a stale Hidden action"
    );
    Ok(())
}

fn first_label(widget: &gtk::Widget) -> Option<String> {
    if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
        return Some(label.text().into());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(text) = first_label(&widget) {
            return Some(text);
        }
        child = widget.next_sibling();
    }
    None
}

fn apps() -> Result<(), Box<dyn std::error::Error>> {
    let source =
        gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)?;
    let schema = source
        .lookup("org.ldelossa.way-shell.window-manager", false)
        .unwrap();
    let settings =
        gio::Settings::new_full(&schema, Some(&gio::memory_settings_backend_new()), None);
    let service = WaylandService::with_settings(settings)?;
    until(|| service.is_ready() || service.error().is_some());
    assert!(service.error().is_none());
    let first_app = gtk::Application::builder()
        .application_id("org.example.WayShellEditorFixture")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    first_app.register(None::<&gio::Cancellable>)?;
    let second_app = gtk::Application::builder()
        .application_id("org.example.WayShellBrowserFixture")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    second_app.register(None::<&gio::Cancellable>)?;
    let editor_a = gtk::ApplicationWindow::builder()
        .application(&first_app)
        .title("Editor α")
        .default_width(180)
        .default_height(100)
        .build();
    let editor_b = gtk::ApplicationWindow::builder()
        .application(&first_app)
        .title("Editor β")
        .default_width(180)
        .default_height(100)
        .build();
    let browser = gtk::ApplicationWindow::builder()
        .application(&second_app)
        .title("Browser")
        .default_width(180)
        .default_height(100)
        .build();
    editor_a.present();
    editor_b.present();
    browser.present();
    until(|| service.toplevels().len() == 3);
    let id = |title: &str| {
        service
            .toplevels()
            .into_iter()
            .find(|top| top.title.as_deref() == Some(title))
            .unwrap()
            .id
    };
    let a = id("Editor α");
    let b = id("Editor β");
    let browser_id = id("Browser");
    let view = AppSwitcher::new(service.clone())?;
    service.action(a, ToplevelAction::Activate)?;
    until(|| {
        service
            .toplevels()
            .iter()
            .any(|top| top.id == a && top.active)
    });
    service.action(browser_id, ToplevelAction::Activate)?;
    until(|| {
        service
            .toplevels()
            .iter()
            .any(|top| top.id == browser_id && top.active)
    });
    view.show();
    until(|| view.window().is_mapped());
    assert_eq!(view.selected_id(), Some(a));
    until(|| view.instances_window().is_mapped());
    {
        let selected = selected_button(view.window().upcast_ref()).unwrap();
        let contents = selected.parent().unwrap();
        let weak = Rc::downgrade(&view);
        let triggered = Cell::new(false);
        let handler = contents.connect_parent_notify(move |widget| {
            if widget.parent().is_none()
                && !triggered.replace(true)
                && let Some(view) = weak.upgrade()
            {
                view.next_app(false);
            }
        });
        view.next_app(false);
        contents.disconnect(handler);
        let selected = view.selected_id().unwrap();
        let expected = service
            .toplevels()
            .into_iter()
            .find(|top| top.id == selected)
            .unwrap()
            .app_id
            .unwrap();
        assert_eq!(
            first_label(
                selected_button(view.window().upcast_ref())
                    .unwrap()
                    .upcast_ref()
            )
            .as_deref(),
            Some(expected.as_str()),
            "nested selection must repaint the newest application"
        );
        let weak = Rc::downgrade(&view);
        let reopened = Cell::new(false);
        let handler = view
            .instances_window()
            .connect_visible_notify(move |window| {
                if !window.is_visible()
                    && !reopened.replace(true)
                    && let Some(view) = weak.upgrade()
                {
                    view.show();
                }
            });
        view.hide();
        view.instances_window().disconnect(handler);
        assert!(
            view.is_visible(),
            "new show from instance-window hide must win"
        );
        view.hide();
        service.action(a, ToplevelAction::Activate)?;
        until(|| {
            service
                .toplevels()
                .iter()
                .any(|top| top.id == a && top.active)
        });
        service.action(browser_id, ToplevelAction::Activate)?;
        until(|| {
            service
                .toplevels()
                .iter()
                .any(|top| top.id == browser_id && top.active)
        });
        view.show();
        assert_eq!(view.selected_id(), Some(a));
    }
    view.next_instance(false);
    assert_eq!(view.selected_id(), Some(b));
    // The exclusive layer has keyboard focus during preview, so compositors
    // may report every normal toplevel inactive until the switcher closes.
    assert!(selected_button(view.instances_window().upcast_ref()).is_some());
    // The application header activates the same highlighted instance, never a
    // null protocol handle or a different index in its group.
    selected_button(view.window().upcast_ref())
        .unwrap()
        .emit_clicked();
    assert!(!view.is_visible());
    until(|| {
        service
            .toplevels()
            .iter()
            .any(|top| top.id == b && top.active)
    });
    view.show();
    // A release of either Super key activates the highlighted typed ID.
    keys(view.window()).emit_by_name::<()>(
        "key-released",
        &[
            &gtk::gdk::Key::Super_R,
            &0u32,
            &gtk::gdk::ModifierType::empty(),
        ],
    );
    assert!(!view.is_visible());
    view.show();
    editor_a.set_title(Some("Editor renamed"));
    until(|| {
        service
            .toplevels()
            .iter()
            .any(|top| top.id == a && top.title.as_deref() == Some("Editor renamed"))
    });
    let selected = view.selected_id().unwrap();
    let closed = if selected == a {
        &editor_a
    } else if selected == b {
        &editor_b
    } else {
        &browser
    };
    closed.destroy();
    until(|| service.toplevels().iter().all(|top| top.id != selected));
    assert_ne!(view.selected_id(), Some(selected));
    let window = view.window().clone();
    let instances = view.instances_window().clone();
    drop(view);
    assert!(!window.is_visible() && !instances.is_visible());
    if service.has_shortcut_inhibition() {
        let owner = [&editor_a, &editor_b, &browser]
            .into_iter()
            .find(|window| window.is_mapped())
            .unwrap();
        service.inhibit_shortcuts(owner)?;
        let competing = AppSwitcher::new(service.clone())?;
        competing.show();
        competing.hide();
        drop(competing);
        assert!(
            service.restore_shortcuts(),
            "a popup must not release another window's inhibitor"
        );
    }
    editor_a.destroy();
    editor_b.destroy();
    browser.destroy();
    until(|| service.toplevels().is_empty());
    let empty = AppSwitcher::new(service)?;
    empty.show();
    assert!(!empty.is_visible());
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    adw::init()?;
    if std::env::var_os("WAY_SHELL_REENTRANT_SHARED").is_some() {
        shared_reentrant()?;
        return Ok(());
    }
    shared()?;
    shared_reentrant()?;
    apps()?;
    println!(
        "shared search/navigation/cancellation and application grouping/preview/activation/removal/reentrant updates/cleanup passed"
    );
    Ok(())
}
