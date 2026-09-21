use gio::prelude::*;
use std::{cell::RefCell, rc::Rc, time::Duration};
use way_shell::services::tray::menu::{MenuService, MenuState};
#[path = "common/tray_menu.rs"]
mod fixture;
#[path = "common/media.rs"]
mod media_fixture;
use fixture::*;

#[test]
fn layout_preserves_labels_visibility_sections_submenus_and_updates() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let connection = connect(&address);
            let endpoint = connect(&address);
            let menu = Menu::new(&endpoint, "/Menu");
            let service =
                MenuService::new(&connection, &endpoint.unique_name().unwrap(), "/Menu").unwrap();
            wait(&context, || service.state().available);
            let first = service.state();
            let root = first.root.as_ref().unwrap();
            assert_eq!(first.revision, 1);
            assert_eq!(root.id, 0);
            assert_eq!(
                root.children.iter().map(|node| node.id).collect::<Vec<_>>(),
                vec![1, 2, 3, 4, 6, 7]
            );
            assert_eq!(root.children[0].label, "_Open");
            assert!(!root.children[1].visible);
            assert!(root.children[2].separator);
            assert_eq!(root.children[3].children[0].label, "Child");
            assert!(root.children[4].separator);
            assert_eq!(root.children[5].label, "Quit");
            let requests = menu.requests.borrow();
            let (parent, depth, properties) =
                requests[0].1.get::<(i32, i32, Vec<String>)>().unwrap();
            assert_eq!((parent, depth), (0, -1));
            assert!(properties.iter().any(|value| value == "label"));
            drop(requests);
            menu.layout.replace(layout("Updated"));
            menu.revision.set(2);
            menu.layout_updated();
            wait(&context, || service.state().revision == 2);
            assert_eq!(service.state().root.unwrap().children[0].label, "Updated");
            assert_eq!(
                first.root.unwrap().children[0].label,
                "_Open",
                "Old snapshots remain owned and unchanged"
            );
            menu.layout.replace(layout("Property update"));
            menu.properties_updated();
            wait(&context, || {
                service.state().root.as_ref().unwrap().children[0].label == "Property update"
            });
            service.stop();
            assert!(!service.state().available);
            assert!(service.state().root.is_none());
            service.start();
            wait(&context, || service.state().available);
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
        })
        .unwrap();
}

#[test]
fn actual_about_to_show_reply_refreshes_and_actions_use_single_variant_and_timestamp() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let endpoint = connect(&address);
            let menu = Menu::new(&endpoint, "/Menu");
            let service = MenuService::new(
                &connect(&address),
                &endpoint.unique_name().unwrap(),
                "/Menu",
            )
            .unwrap();
            wait(&context, || service.state().available);
            let result = Rc::new(RefCell::new(None));
            let output = result.clone();
            service.about_to_show(0, move |value| {
                output.replace(Some(value));
            });
            wait(&context, || result.borrow().is_some());
            assert!(!result.borrow_mut().take().unwrap().unwrap());
            menu.needs_update.set(true);
            menu.revision.set(3);
            menu.layout.replace(layout("Shown"));
            let output = result.clone();
            service.about_to_show(4, move |value| {
                output.replace(Some(value));
            });
            wait(&context, || result.borrow().is_some());
            assert!(result.borrow_mut().take().unwrap().unwrap());
            wait(&context, || service.state().revision == 3);
            let result = Rc::new(RefCell::new(None));
            let output = result.clone();
            service.activate(5, 0xfedcba98, move |value| {
                output.replace(Some(value));
            });
            wait(&context, || result.borrow().is_some());
            assert!(result.borrow_mut().take().unwrap().is_ok());
            let requests = menu.requests.borrow();
            let (_, parameters) = requests
                .iter()
                .find(|(method, _)| method == "Event")
                .unwrap();
            let (id, event, data, timestamp) = parameters
                .get::<(i32, String, glib::Variant, u32)>()
                .unwrap();
            assert_eq!(id, 5);
            assert_eq!(event, "clicked");
            assert_eq!(data.get::<i32>(), Some(0));
            assert_eq!(timestamp, 0xfedcba98);
            drop(requests);
            for id in [-1, 0, 2, 3, 999] {
                let output = result.clone();
                service.activate(id, 1, move |value| {
                    output.replace(Some(value));
                });
                wait(&context, || result.borrow().is_some());
                assert!(result.borrow_mut().take().unwrap().is_err());
            }
            menu.action_mode.set(Reply::Failure);
            let output = result.clone();
            service.activate(1, 1, move |value| {
                output.replace(Some(value));
            });
            wait(&context, || result.borrow().is_some());
            assert!(result.borrow_mut().take().unwrap().is_err());
        })
        .unwrap();
}

#[test]
fn missing_endpoint_failed_layout_stale_replies_and_owner_loss_recover_safely() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let connection = connect(&address);
            let endpoint = connect(&address);
            for (owner, path) in [
                ("org.example.Alias", "/Menu"),
                (":1.2", "/"),
                (":1.2", "bad path"),
            ] {
                assert!(MenuService::new(&connection, owner, path).is_err());
            }
            let service =
                MenuService::new(&connection, &endpoint.unique_name().unwrap(), "/Menu").unwrap();
            context.block_on(glib::timeout_future(Duration::from_millis(30)));
            assert!(!service.state().available);
            let menu = Menu::new(&endpoint, "/Menu");
            wait(&context, || service.state().available);
            menu.layout_mode.set(Reply::Hold);
            menu.layout_updated();
            wait(&context, || !menu.held.borrow().is_empty());
            menu.layout_mode.set(Reply::Success);
            menu.revision.set(4);
            menu.layout.replace(layout("Latest"));
            menu.layout_updated();
            wait(&context, || service.state().revision == 4);
            menu.held
                .borrow_mut()
                .remove(0)
                .return_value(Some(&reply(2, &layout("Stale"))));
            context.block_on(glib::timeout_future(Duration::from_millis(20)));
            assert_eq!(service.state().revision, 4);
            menu.layout_mode.set(Reply::Malformed);
            menu.layout_updated();
            wait(&context, || !service.state().available);
            menu.layout_mode.set(Reply::Success);
            wait(&context, || service.state().available);
            menu.layout_mode.set(Reply::Hold);
            menu.layout_updated();
            wait(&context, || !menu.held.borrow().is_empty());
            endpoint.close_sync(gio::Cancellable::NONE).unwrap();
            wait(&context, || {
                !service.state().available && service.state().root.is_none()
            });
            drop(menu);
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
        })
        .unwrap();
}

#[test]
fn stop_and_drop_cancel_actions_and_reentrant_snapshot_delivery_is_ordered() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let endpoint = connect(&address);
            let menu = Menu::new(&endpoint, "/Menu");
            let service = MenuService::new(
                &connect(&address),
                &endpoint.unique_name().unwrap(),
                "/Menu",
            )
            .unwrap();
            wait(&context, || service.state().available);
            menu.action_mode.set(Reply::Hold);
            let result = Rc::new(RefCell::new(None));
            let output = result.clone();
            service.activate(1, 1, move |value| {
                output.replace(Some(value));
            });
            wait(&context, || !menu.held.borrow().is_empty());
            service.stop();
            wait(&context, || result.borrow().is_some());
            assert!(result.borrow_mut().take().unwrap().is_err());
            service.connect_local("snapshot", false, move |values| {
                if values[1].get::<MenuState>().unwrap().available {
                    values[0].get::<MenuService>().unwrap().stop();
                }
                None
            });
            let snapshots = Rc::new(RefCell::new(Vec::new()));
            let recorded = snapshots.clone();
            service.connect_local("snapshot", false, move |values| {
                recorded
                    .borrow_mut()
                    .push(values[1].get::<MenuState>().unwrap());
                None
            });
            service.start();
            wait(&context, || snapshots.borrow().len() == 2);
            assert!(snapshots.borrow()[0].available);
            assert!(!snapshots.borrow()[1].available);
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
        })
        .unwrap();
}

#[test]
fn replacement_endpoint_and_whole_bus_loss_cancel_old_work_without_reusing_snapshots() {
    let (bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let connection = connect(&address);
            let endpoint = connect(&address);
            let first = Menu::new(&endpoint, "/First");
            let second = Menu::new(&endpoint, "/Second");
            second.layout.replace(layout("Replacement endpoint"));
            let old =
                MenuService::new(&connection, &endpoint.unique_name().unwrap(), "/First").unwrap();
            wait(&context, || old.state().available);
            let snapshot = old.state();
            first.layout_mode.set(Reply::Hold);
            first.layout_updated();
            wait(&context, || !first.held.borrow().is_empty());
            old.stop();
            let replacement =
                MenuService::new(&connection, &endpoint.unique_name().unwrap(), "/Second").unwrap();
            wait(&context, || replacement.state().available);
            first
                .held
                .borrow_mut()
                .remove(0)
                .return_value(Some(&reply(99, &layout("Late old endpoint"))));
            context.block_on(glib::timeout_future(Duration::from_millis(20)));
            assert!(!old.state().available);
            assert!(old.state().root.is_none());
            assert_eq!(snapshot.root.unwrap().children[0].label, "_Open");
            assert_eq!(
                replacement.state().root.unwrap().children[0].label,
                "Replacement endpoint"
            );
            second.action_mode.set(Reply::Hold);
            let reply = Rc::new(RefCell::new(None));
            let output = reply.clone();
            replacement.about_to_show(0, move |result| {
                output.replace(Some(result));
            });
            wait(&context, || !second.held.borrow().is_empty());
            drop(bus);
            wait(&context, || {
                !replacement.state().available && replacement.state().root.is_none()
            });
            wait(&context, || reply.borrow().is_some());
            assert!(reply.borrow_mut().take().unwrap().is_err());
            let weak = replacement.downgrade();
            drop(replacement);
            assert!(weak.upgrade().is_none());
        })
        .unwrap();
}

#[test]
fn tray_factory_uses_the_registered_connection_and_keeps_absent_menus_optional() {
    #[path = "common/tray.rs"]
    mod item_fixture;
    use item_fixture::{Bus, connect, wait};
    use way_shell::services::tray::TrayService;
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let client = connect(&address);
            let tray = TrayService::on_connection(&connect(&address));
            wait(&context, || tray.state().available);
            let item = item_fixture::Item::new(&client, "/Item", "factory");
            item_fixture::register(&context, &client, "/Item").unwrap();
            wait(&context, || tray.state().items.len() == 1);
            let key = tray.state().items[0].key.clone();
            assert!(tray.menu(&key).unwrap().is_none());
            let _endpoint = Menu::new(&client, "/Menu");
            item.values.borrow_mut().insert(
                "Menu".into(),
                glib::variant::ObjectPath::try_from("/Menu")
                    .unwrap()
                    .to_variant(),
            );
            item.properties_changed();
            wait(&context, || tray.state().items[0].menu_path.is_some());
            let menu = tray.menu(&key).unwrap().unwrap();
            wait(&context, || menu.state().available);
            assert_eq!(menu.state().root.unwrap().children[0].label, "_Open");
            tray.stop();
            assert!(tray.menu(&key).is_err());
            menu.stop();
        })
        .unwrap();
}

#[test]
fn held_methods_reach_deadlines_recover_and_drop_cancels_in_flight_actions() {
    let (_bus, address) = Bus::start();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let endpoint = connect(&address);
            let menu = Menu::new(&endpoint, "/Menu");
            menu.layout_mode.set(Reply::Hold);
            let service = MenuService::new(
                &connect(&address),
                &endpoint.unique_name().unwrap(),
                "/Menu",
            )
            .unwrap();
            wait(&context, || !menu.held.borrow().is_empty());
            // A main-loop timer still progresses while the server withholds a reply.
            context.block_on(glib::timeout_future(Duration::from_millis(20)));
            assert!(!service.state().available);
            menu.layout_mode.set(Reply::Success);
            wait(&context, || service.state().available);
            assert_eq!(
                menu.reads.get(),
                2,
                "GetLayout times out and retries without a daemon signal"
            );
            menu.held
                .borrow_mut()
                .remove(0)
                .return_value(Some(&reply(99, &layout("Late timed-out layout"))));
            context.block_on(glib::timeout_future(Duration::from_millis(20)));
            assert_eq!(service.state().revision, 1);

            menu.action_mode.set(Reply::Hold);
            let about = Rc::new(RefCell::new(None));
            let output = about.clone();
            service.about_to_show(0, move |result| {
                output.replace(Some(result));
            });
            wait(&context, || !menu.held.borrow().is_empty());
            context.block_on(glib::timeout_future(Duration::from_millis(20)));
            assert!(about.borrow().is_none());
            wait(&context, || about.borrow().is_some());
            assert!(about.borrow_mut().take().unwrap().is_err());
            menu.held
                .borrow_mut()
                .remove(0)
                .return_value(Some(&(false,).to_variant()));

            let action = Rc::new(RefCell::new(None));
            let output = action.clone();
            service.activate(1, 123, move |result| {
                output.replace(Some(result));
            });
            wait(&context, || !menu.held.borrow().is_empty());
            wait(&context, || action.borrow().is_some());
            assert!(action.borrow_mut().take().unwrap().is_err());
            menu.held
                .borrow_mut()
                .remove(0)
                .return_value(Some(&().to_variant()));
            let output = action.clone();
            service.activate(1, 124, move |result| {
                output.replace(Some(result));
            });
            wait(&context, || !menu.held.borrow().is_empty());
            let weak = service.downgrade();
            drop(service);
            assert!(weak.upgrade().is_none());
            wait(&context, || action.borrow().is_some());
            assert!(action.borrow_mut().take().unwrap().is_err());
        })
        .unwrap();
}
