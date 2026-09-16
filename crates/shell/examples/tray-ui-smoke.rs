//! Standalone single-thread GTK contract check, run under isolated Sway or Niri.
#[path = "../tests/common/tray.rs"]
mod item_fixture;
#[path = "../tests/common/media.rs"]
mod media_fixture;
#[path = "../tests/common/tray_menu.rs"]
mod menu_fixture;
use adw::prelude::*;
use std::time::Duration;
use way_shell::{
    services::tray::TrayService,
    ui::{
        panel::tray::TrayBar,
        window::{LayerWindow, WindowRole},
    },
};

fn children(widget: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut children = Vec::new();
    let mut next = widget.first_child();
    while let Some(child) = next {
        next = child.next_sibling();
        children.push(child);
    }
    children
}
fn button(row: &gtk::Widget) -> gtk::Button {
    row.first_child()
        .unwrap()
        .first_child()
        .unwrap()
        .downcast()
        .unwrap()
}
fn image(button: &gtk::Button) -> gtk::Image {
    button.child().unwrap().downcast().unwrap()
}
fn popup(button: &gtk::Button) -> Option<gtk::PopoverMenu> {
    children(button)
        .into_iter()
        .find_map(|child| child.downcast().ok())
}
fn set(item: &item_fixture::Item, name: &str, value: glib::Variant) {
    item.values.borrow_mut().insert(name.into(), value);
    item.properties_changed();
}
fn set_menu(item: &item_fixture::Item, path: &str) {
    set(
        item,
        "Menu",
        glib::variant::ObjectPath::try_from(path)
            .unwrap()
            .to_variant(),
    );
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    adw::init()?;
    if !gtk4_layer_shell::is_supported() {
        return Err("Tray UI smoke test needs layer-shell".into());
    }
    let (_bus, address) = menu_fixture::Bus::start();
    let context = glib::MainContext::default();
    context.with_thread_default(|| {
        let connection = menu_fixture::connect(&address);
        let service = TrayService::on_connection(&item_fixture::connect(&address));
        menu_fixture::wait(&context, || service.state().available);
        let first = item_fixture::Item::new(&connection, "/First", "first");
        let second = item_fixture::Item::new(&connection, "/Second", "second");
        set(
            &first,
            "IconPixmap",
            vec![(1_i32, 1_i32, vec![128_u8, 11, 22, 33])].to_variant(),
        );
        item_fixture::register(&context, &connection, "/First").unwrap();
        item_fixture::wait(&context, || service.state().items.len() == 1);
        let bar = TrayBar::new(service.clone());
        let root = bar.widget().clone();
        assert_eq!(root.widget_name(), "panel-indicator-bar");
        let list = root.first_child().unwrap();
        assert!(list.has_css_class("panel-indicator-bar-list"));
        let rows = children(&list);
        assert_eq!(rows.len(), 1);
        let row = rows[0].clone();
        assert_eq!(row.widget_name(), "panel-indicator-bar-widget");
        assert!(
            row.first_child()
                .unwrap()
                .has_css_class("panel-indicator-bar-widget-box")
        );
        let first_button = button(&row);
        assert!(image(&first_button).paintable().is_some());
        let window = LayerWindow::new(WindowRole::Panel, None).unwrap();
        // This fixture intentionally removes the panel's only content while the
        // surface stays mapped; retain a nonzero native minimum during cleanup.
        window.window().set_size_request(320, 30);
        window.window().set_content(Some(&root));
        window.present();
        item_fixture::wait(&context, || window.window().is_mapped());
        first_button.emit_clicked();
        item_fixture::wait(&context, || first.actions.borrow().len() == 1);
        assert_eq!(first.actions.borrow()[0].0, "Activate");
        assert_eq!(
            first.actions.borrow()[0].1.get::<(i32, i32)>(),
            Some((0, 0))
        );
        item_fixture::register(&context, &connection, "/Second").unwrap();
        item_fixture::wait(&context, || children(&list).len() == 2);
        assert_eq!(
            children(&list)[0],
            row,
            "adding another path preserves existing widget identity"
        );
        set(&first, "Title", "Updated title".to_variant());
        set(
            &first,
            "IconName",
            "dialog-information-symbolic".to_variant(),
        );
        item_fixture::wait(&context, || {
            image(&first_button).icon_name().as_deref() == Some("dialog-information-symbolic")
        });
        assert_eq!(children(&list)[0], row);

        let menu = menu_fixture::Menu::new(&connection, "/Menu");
        set_menu(&first, "/Menu");
        item_fixture::wait(&context, || popup(&first_button).is_some());
        let retained_popup = popup(&first_button).unwrap();
        let key = format!("{}/First", connection.unique_name().unwrap());
        let model = retained_popup.menu_model().unwrap();
        assert_eq!(
            model
                .item_attribute_value(0, "target", None)
                .unwrap()
                .get::<(String, i32)>(),
            Some((key.clone(), 1))
        );
        let reads = menu.reads.get();
        menu.needs_update.set(true);
        menu.layout.replace(menu_fixture::layout("Changed menu"));
        first_button.emit_clicked();
        item_fixture::wait(&context, || {
            menu.reads.get() > reads
                && retained_popup
                    .menu_model()
                    .unwrap()
                    .item_attribute_value(0, "label", None)
                    .unwrap()
                    .get::<String>()
                    .as_deref()
                    == Some("Changed menu")
        });
        assert!(retained_popup.is_visible());
        assert_eq!(
            first.actions.borrow().len(),
            1,
            "menu clicks send AboutToShow instead of Activate"
        );
        assert!(
            menu.requests
                .borrow()
                .iter()
                .any(|(name, args)| name == "AboutToShow" && args.get::<(i32,)>() == Some((0,)))
        );
        retained_popup
            .activate_action(
                "sni.item-clicked",
                Some(&(key.as_str(), 1_i32).to_variant()),
            )
            .unwrap();
        item_fixture::wait(&context, || {
            menu.requests
                .borrow()
                .iter()
                .any(|(name, _)| name == "Event")
        });
        let event = menu
            .requests
            .borrow()
            .iter()
            .find(|(name, _)| name == "Event")
            .unwrap()
            .1
            .clone();
        assert_eq!(event.type_().as_str(), "(isvu)");
        let (id, action, data, timestamp) =
            event.get::<(i32, String, glib::Variant, u32)>().unwrap();
        assert_eq!(
            (id, action.as_str(), data.get::<i32>()),
            (1, "clicked", Some(0))
        );
        assert!(timestamp > 0);
        set_menu(&first, "/");
        item_fixture::wait(&context, || popup(&first_button).is_none());
        assert!(retained_popup.parent().is_none());
        assert!(
            retained_popup
                .activate_action(
                    "sni.item-clicked",
                    Some(&(key.as_str(), 1_i32).to_variant())
                )
                .is_err()
        );
        first_button.emit_clicked();
        item_fixture::wait(&context, || first.actions.borrow().len() == 2);

        let directory =
            std::env::temp_dir().join(format!("way-shell-tray-ui-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("fixture.png"),
            include_bytes!("../../../tests/fixtures/tray-icon.png"),
        )
        .unwrap();
        set(
            &first,
            "IconThemePath",
            directory.to_str().unwrap().to_variant(),
        );
        set(&first, "IconName", "fixture".to_variant());
        item_fixture::wait(&context, || image(&first_button).paintable().is_some());
        set(&first, "IconName", "fixture.png".to_variant());
        set(
            &first,
            "IconThemePath",
            "/nonexistent/way-shell-tray-test".to_variant(),
        );
        set(&first, "IconName", "network-wireless-symbolic".to_variant());
        item_fixture::wait(&context, || {
            image(&first_button).icon_name().as_deref() == Some("network-wireless-symbolic")
        });
        context.block_on(glib::timeout_future(Duration::from_millis(50)));
        assert_eq!(
            image(&first_button).icon_name().as_deref(),
            Some("network-wireless-symbolic")
        );
        std::fs::remove_dir_all(directory).unwrap();

        set_menu(&first, "/Menu");
        item_fixture::wait(&context, || popup(&first_button).is_some());
        let retained_popup = popup(&first_button).unwrap();
        let old_requests = menu.requests.borrow().len();
        let old_actions = first.actions.borrow().len();
        drop(bar);
        assert_eq!(
            children(&list).len(),
            0,
            "retaining GTK widgets must not retain controllers"
        );
        assert!(retained_popup.parent().is_none());
        first_button.emit_clicked();
        assert!(
            retained_popup
                .activate_action(
                    "sni.item-clicked",
                    Some(&(key.as_str(), 1_i32).to_variant())
                )
                .is_err()
        );
        set(&second, "Title", "After controller drop".to_variant());
        context.block_on(glib::timeout_future(Duration::from_millis(30)));
        assert_eq!(first.actions.borrow().len(), old_actions);
        assert_eq!(menu.requests.borrow().len(), old_requests);
        assert!(children(&list).is_empty());
        window.close();
        drop(window);
        service.stop();
    })?;
    // Owner loss removes rows and invalidates callbacks even when the caller
    // retains a GTK button after its controller has gone away.
    let (_second_bus, address) = item_fixture::Bus::start();
    context.with_thread_default(|| {
        let connection = item_fixture::connect(&address);
        let service = TrayService::on_connection(&menu_fixture::connect(&address));
        item_fixture::wait(&context, || service.state().available);
        let item = item_fixture::Item::new(&connection, "/Item", "removal");
        let bar = TrayBar::new(service.clone());
        let list = bar.widget().first_child().unwrap();
        item_fixture::register(&context, &connection, "/Item").unwrap();
        item_fixture::wait(&context, || children(&list).len() == 1);
        let retained = button(&children(&list)[0]);
        connection.close_sync(gio::Cancellable::NONE).unwrap();
        item_fixture::wait(&context, || children(&list).is_empty());
        retained.emit_clicked();
        assert!(item.actions.borrow().is_empty());
        drop(bar);
        service.stop();
    })?;
    println!(
        "tray GTK identity, icons, menu actions, endpoint changes and weak disposal checks passed"
    );
    Ok(())
}
