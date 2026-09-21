//! Hermetic desktop-entry discovery/launch and real GTK activities lifecycle.
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
    time::{Duration, Instant},
};
use way_shell::{
    services::apps::AppCatalog,
    ui::{
        activities::{Activities, ActivitiesEvent},
        window::Visibility,
    },
};
fn wait(context: &glib::MainContext, predicate: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(8);
    while !predicate() {
        assert!(Instant::now() < end, "activities state timed out");
        context.block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}
fn descendants(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut result = vec![root.as_ref().clone()];
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        result.extend(descendants(&widget));
    }
    result
}
fn find<T>(root: &impl IsA<gtk::Widget>) -> T
where
    T: IsA<gtk::Widget> + glib::types::StaticType + Clone + glib::object::ObjectType,
{
    descendants(root)
        .into_iter()
        .find_map(|widget| widget.downcast::<T>().ok())
        .expect("expected widget")
}
fn app_button(child: &gtk::FlowBoxChild) -> gtk::Button {
    find(child)
}
fn desktop(directory: &Path, id: &str, name: &str, icon: &str, extra: &str) {
    let root = directory.parent().unwrap();
    let icon = if icon.is_empty() {
        String::new()
    } else {
        format!("Icon={icon}\n")
    };
    let text = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec=\"{}\" \"{}\"\n{icon}{extra}",
        root.join("launch-fixture").display(),
        root.join("launch-marker").display()
    );
    std::fs::write(directory.join(id), text).unwrap();
}
fn run(root: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    adw::init()?;
    let context = glib::MainContext::default();
    context.with_thread_default(|| {
        let stopped = AppCatalog::new();
        stopped.stop();
        assert!(!stopped.is_loading());
        let weak = stopped.downgrade();
        drop(stopped);
        context.block_on(glib::timeout_future(Duration::from_millis(20)));
        assert!(
            weak.upgrade().is_none(),
            "a detached scan cannot retain its catalog"
        );
        let catalog = AppCatalog::new();
        wait(&context, || !catalog.is_loading());
        let entries = catalog.entries();
        assert_eq!(entries.len(), 26);
        assert!(entries.iter().all(|entry| entry.id.starts_with("fixture-")));
        assert!(!entries.iter().any(|entry| entry.name == "Hidden Fixture"));
        let activities = Activities::new(catalog.clone()).unwrap();
        let events = Rc::new(RefCell::new(Vec::new()));
        let seen = events.clone();
        activities.on_event(move |event| seen.borrow_mut().push(event));
        let window = activities.window().clone();
        assert_eq!(window.widget_name(), "activities");
        let search: gtk::SearchEntry = find(&window);
        let flow: gtk::FlowBox = find(&window);
        let carousel: adw::Carousel = find(&window);
        let fallback = descendants(&window).into_iter().find_map(|widget| {
            let image = widget.downcast::<gtk::Image>().ok()?;
            (image.icon_name().as_deref() == Some("image-missing")).then_some(image)
        });
        assert!(
            fallback.is_some(),
            "a desktop entry without an icon has a visible fallback"
        );
        assert_eq!(search.widget_name(), "search-entry");
        assert_eq!(carousel.n_pages(), 2);
        let first = carousel.nth_page(0).downcast::<gtk::Grid>().unwrap();
        assert!(first.child_at(6, 2).is_some());
        assert!(first.child_at(2, 3).is_some());
        assert!(first.child_at(3, 3).is_none());
        assert!(activities.show());
        wait(&context, || {
            activities.visibility_controller().visibility() == Visibility::Visible
        });
        assert!(window.is_mapped());
        assert!(search.has_focus() || gtk::prelude::RootExt::focus(&window).is_some());
        assert_eq!(
            &events.borrow()[..2],
            &[ActivitiesEvent::WillShow, ActivitiesEvent::Visible]
        );
        let weak_search = search.downgrade();
        let once = Cell::new(false);
        let handler = carousel.connect_notify_local(Some("visible"), move |carousel, _| {
            if !carousel.is_visible()
                && !once.replace(true)
                && let Some(search) = weak_search.upgrade()
            {
                search.set_text("");
                search.emit_by_name::<()>("search-changed", &[]);
            }
        });
        search.set_text("fixture");
        search.emit_by_name::<()>("search-changed", &[]);
        assert_eq!(search.text(), "");
        assert!(
            carousel.is_visible(),
            "a nested clear wins over the old search state"
        );
        assert!(flow.selected_children().is_empty());
        carousel.disconnect(handler);
        search.set_text("cafe launch");
        search.emit_by_name::<()>("search-changed", &[]);
        let selected = flow.selected_children();
        assert_eq!(selected.len(), 1);
        let launch_button = app_button(&selected[0]);
        let icon: gtk::Image = find(&launch_button);
        assert_eq!(icon.pixel_size(), 78);
        let file_icon = icon.gicon().unwrap().downcast::<gio::FileIcon>().unwrap();
        assert_eq!(file_icon.file().path(), Some(root.join("icon.png")));
        search.emit_by_name::<()>("stop-search", &[]);
        assert_eq!(search.text(), "");
        assert!(carousel.is_visible());
        assert_eq!(
            activities.visibility_controller().visibility(),
            Visibility::Visible,
            "Escape clears search without dismissing activities"
        );
        search.set_text("fixture");
        search.emit_by_name::<()>("search-changed", &[]);
        let first = flow.selected_children()[0].clone();
        search.emit_by_name::<()>("next-match", &[]);
        let second = flow.selected_children()[0].clone();
        assert_ne!(first, second);
        search.emit_by_name::<()>("previous-match", &[]);
        assert_eq!(flow.selected_children()[0], first);
        search.set_text("no-such-application");
        // SearchEntry delays search-changed; Enter must still use the new text.
        search.emit_by_name::<()>("activate", &[]);
        assert_eq!(
            activities.visibility_controller().visibility(),
            Visibility::Visible,
            "an immediate Enter must not launch a result from the previous query"
        );
        search.emit_by_name::<()>("search-changed", &[]);
        assert!(flow.selected_children().is_empty());
        search.emit_by_name::<()>("activate", &[]);
        assert!(!root.join("launch-marker").exists());
        assert_eq!(
            activities.visibility_controller().visibility(),
            Visibility::Visible
        );
        search.set_text("cafe launch");
        search.emit_by_name::<()>("search-changed", &[]);
        search.emit_by_name::<()>("activate", &[]);
        wait(&context, || {
            std::fs::read_to_string(root.join("launch-marker"))
                .ok()
                .as_deref()
                == Some("launched")
        });
        wait(&context, || {
            activities.visibility_controller().visibility() == Visibility::Hidden
        });
        assert_eq!(
            std::fs::read_to_string(root.join("launch-marker")).unwrap(),
            "launched"
        );
        std::fs::remove_file(root.join("launch-marker")).unwrap();
        assert!(!window.is_visible());
        assert_eq!(search.text(), "");

        // A file added to the isolated XDG directory is noticed without relying
        // on the old hard-coded /usr/share/applications monitor.
        desktop(
            &root.join("applications"),
            "fixture-late.desktop",
            "Late Fixture",
            "dialog-information-symbolic",
            "",
        );
        wait(&context, || catalog.is_dirty());
        activities.show();
        wait(&context, || {
            catalog.entries().len() == 27 && !catalog.is_loading()
        });
        assert_eq!(carousel.n_pages(), 2);
        search.set_text("cafe launch");
        search.emit_by_name::<()>("search-changed", &[]);
        let removed_button = app_button(&flow.selected_children()[0]);
        std::fs::remove_file(root.join("applications/fixture-launch.desktop")).unwrap();
        wait(&context, || catalog.is_dirty());
        // Launch failure still dismisses activities, with a useful diagnostic.
        removed_button.emit_clicked();
        wait(&context, || {
            activities.visibility_controller().visibility() == Visibility::Hidden
        });
        activities.show();
        wait(&context, || {
            catalog.entries().len() == 26 && !catalog.is_loading()
        });
        wait(&context, || {
            activities.visibility_controller().visibility() == Visibility::Visible
        });
        launch_button.emit_clicked();
        context.block_on(glib::timeout_future(Duration::from_millis(30)));
        assert!(!root.join("launch-marker").exists());
        assert_eq!(
            activities.visibility_controller().visibility(),
            Visibility::Visible,
            "buttons from an obsolete inventory stay inert"
        );

        activities.hide();
        activities.show();
        wait(&context, || {
            activities.visibility_controller().visibility() == Visibility::Visible
        });
        context.block_on(glib::timeout_future(Duration::from_millis(400)));
        assert_eq!(
            activities.visibility_controller().visibility(),
            Visibility::Visible
        );
        activities.hide();
        wait(&context, || {
            activities.visibility_controller().visibility() == Visibility::Hidden
        });
        activities.show();
        activities.hide();
        wait(&context, || {
            activities.visibility_controller().visibility() == Visibility::Hidden
        });
        context.block_on(glib::timeout_future(Duration::from_millis(400)));
        assert!(!window.is_visible());
        activities.show();
        wait(&context, || {
            activities.visibility_controller().visibility() == Visibility::Visible
        });
        search.set_text("late");
        search.emit_by_name::<()>("search-changed", &[]);
        let retained = app_button(&flow.selected_children()[0]);
        drop(activities);
        assert!(!window.is_visible());
        retained.emit_clicked();
        search.emit_by_name::<()>("activate", &[]);
        context.block_on(glib::timeout_future(Duration::from_millis(40)));
        assert!(!root.join("launch-marker").exists());

        // Reentrant mediator hide wins over a show request before presentation.
        let activities = Activities::new(catalog.clone()).unwrap();
        let slot = Rc::new(RefCell::new(Some(activities.clone())));
        let weak = Rc::downgrade(&slot);
        let once = Cell::new(false);
        activities.on_event(move |event| {
            if event == ActivitiesEvent::WillShow
                && !once.replace(true)
                && let Some(slot) = weak.upgrade()
            {
                slot.borrow().as_ref().unwrap().hide();
            }
        });
        assert!(!activities.show());
        assert_eq!(
            activities.visibility_controller().visibility(),
            Visibility::Hidden
        );
        assert!(!activities.window().is_visible());
        slot.borrow_mut().take();
        drop(activities);
        catalog.stop();
    })?;
    println!(
        "activities discovery, search, navigation, desktop launching, file icons and lifecycle checks passed"
    );
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    const CHILD: &str = "WAY_SHELL_ACTIVITIES_FIXTURE";
    if let Some(root) = std::env::var_os(CHILD) {
        return run(root.into());
    }
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("way-shell-activities-{}", std::process::id()));
    let apps = root.join("applications");
    std::fs::create_dir_all(&apps)?;
    let result = (|| {
        let launcher = root.join("launch-fixture");
        std::fs::write(&launcher, "#!/bin/sh\nprintf launched > \"$1\"\n")?;
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700))?;
        let image =
            way_shell::ui::tray_support::rgba_pixbuf(&way_shell::services::tray::RgbaImage {
                width: 1,
                height: 1,
                pixels: vec![10, 20, 30, 255],
            })
            .unwrap();
        let icon = root.join("icon.png");
        std::fs::write(&icon, image.save_to_bufferv("png", &[])?)?;
        desktop(
            &apps,
            "fixture-launch.desktop",
            "Café Launchme",
            icon.to_str().unwrap(),
            "",
        );
        for index in 0..25 {
            desktop(
                &apps,
                &format!("fixture-{index:02}.desktop"),
                &format!("Fixture App {index:02}"),
                if index == 0 {
                    ""
                } else {
                    "dialog-information-symbolic"
                },
                "",
            );
        }
        desktop(
            &apps,
            "fixture-hidden.desktop",
            "Hidden Fixture",
            "",
            "NoDisplay=true\n",
        );
        let status = Command::new(std::env::current_exe()?)
            .env(CHILD, &root)
            .env("XDG_DATA_HOME", &root)
            .env("XDG_DATA_DIRS", root.join("empty-system"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .status()?;
        assert!(status.success(), "isolated activities fixture failed");
        Ok::<_, Box<dyn std::error::Error>>(())
    })();
    std::fs::remove_dir_all(&root)?;
    result
}
