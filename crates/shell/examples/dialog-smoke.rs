//! Confirmation callback and window ownership checks on a real compositor.
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};
use way_shell::ui::dialog::Dialog;

fn descendants(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut values = vec![root.as_ref().clone()];
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        values.extend(descendants(&widget));
    }
    values
}
fn styled<T: IsA<gtk::Widget> + glib::object::ObjectType>(
    root: &impl IsA<gtk::Widget>,
    class: &str,
) -> T {
    descendants(root)
        .into_iter()
        .find(|widget| widget.has_css_class(class))
        .unwrap()
        .downcast::<T>()
        .unwrap()
}
fn wait(context: &glib::MainContext, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "dialog window did not settle");
        context.block_on(glib::timeout_future(Duration::from_millis(5)));
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    adw::init()?;
    let context = glib::MainContext::default();
    context.with_thread_default(|| {
        let dialog = Dialog::new().unwrap();
        let window = dialog.window().clone();
        let confirm: gtk::Button = styled(&window, "dialog-overlay-confirm");
        let cancel: gtk::Button = styled(&window, "dialog-overlay-cancel");
        let heading: gtk::Label = styled(&window, "dialog-overlay-heading");
        let body: gtk::Label = styled(&window, "dialog-overlay-body");
        assert_eq!(window.widget_name(), "dialog-overlay");
        let contents = window.content().unwrap();
        assert_eq!(contents.widget_name(), "dialog-overlay-container");
        assert_eq!(contents.width_request(), 420);
        assert_eq!(contents.height_request(), 200);
        let replies = Rc::new(RefCell::new(Vec::new()));
        let output = replies.clone();
        dialog.present("Confirm one?", "First body", move |accepted| {
            output.borrow_mut().push(("one", accepted))
        });
        wait(&context, || window.is_mapped());
        assert_eq!(heading.text(), "Confirm one?");
        assert_eq!(body.text(), "First body");
        assert!(cancel.has_focus() || gtk::prelude::RootExt::focus(&window).is_some());
        confirm.emit_clicked();
        assert_eq!(*replies.borrow(), [("one", true)]);
        assert!(!window.is_visible());
        confirm.emit_clicked();
        cancel.emit_clicked();
        assert_eq!(replies.borrow().len(), 1);
        let output = replies.clone();
        dialog.present("Old", "Old body", move |accepted| {
            output.borrow_mut().push(("old", accepted))
        });
        let output = replies.clone();
        dialog.present("Replacement", "New body", move |accepted| {
            output.borrow_mut().push(("replacement", accepted))
        });
        assert_eq!(replies.borrow().last(), Some(&("old", false)));
        assert_eq!(heading.text(), "Replacement");
        cancel.emit_clicked();
        assert_eq!(replies.borrow().last(), Some(&("replacement", false)));

        // A response may immediately open another dialog; the previous response
        // cannot subsequently hide it or erase its callback.
        let weak = Rc::downgrade(&dialog);
        let output = replies.clone();
        dialog.present("First in chain", "Body", move |accepted| {
            output.borrow_mut().push(("chain-first", accepted));
            if let Some(dialog) = weak.upgrade() {
                dialog.present("Second in chain", "New body", move |accepted| {
                    output.borrow_mut().push(("chain-second", accepted))
                });
            }
        });
        confirm.emit_clicked();
        assert!(window.is_visible());
        assert_eq!(heading.text(), "Second in chain");
        confirm.emit_clicked();
        assert_eq!(replies.borrow().last(), Some(&("chain-second", true)));

        // Cancellation during replacement may present a newer request. It wins
        // over the already superseded outer present call.
        let weak = Rc::downgrade(&dialog);
        let output = replies.clone();
        dialog.present("To be replaced", "Body", move |accepted| {
            output.borrow_mut().push(("replaced", accepted));
            if let Some(dialog) = weak.upgrade() {
                dialog.present("Reentrant winner", "Winner body", move |accepted| {
                    output.borrow_mut().push(("winner", accepted))
                });
            }
        });
        let output = replies.clone();
        dialog.present("Superseded", "Wrong body", move |accepted| {
            output.borrow_mut().push(("superseded", accepted))
        });
        assert_eq!(heading.text(), "Reentrant winner");
        assert_eq!(body.text(), "Winner body");
        assert_eq!(replies.borrow().last(), Some(&("superseded", false)));
        confirm.emit_clicked();
        assert_eq!(replies.borrow().last(), Some(&("winner", true)));

        // Reopening from GTK's visible notification also survives the outer
        // hide. No pending-response borrow may be held across GTK mutations.
        let weak = Rc::downgrade(&dialog);
        let once = Cell::new(false);
        let output = replies.clone();
        let handler = window.connect_visible_notify(move |window| {
            if !window.is_visible()
                && !once.replace(true)
                && let Some(dialog) = weak.upgrade()
            {
                let output = output.clone();
                dialog.present("Native reentry", "Body", move |accepted| {
                    output.borrow_mut().push(("native-reentry", accepted))
                });
            }
        });
        let output = replies.clone();
        dialog.present("Hide then reopen", "Body", move |accepted| {
            output.borrow_mut().push(("before-native", accepted))
        });
        cancel.emit_clicked();
        assert!(window.is_visible());
        assert_eq!(heading.text(), "Native reentry");
        window.disconnect(handler);
        cancel.emit_clicked();
        assert_eq!(replies.borrow().last(), Some(&("native-reentry", false)));

        let output = replies.clone();
        dialog.present("Native close", "Body", move |accepted| {
            output.borrow_mut().push(("closed", accepted))
        });
        window.close();
        assert_eq!(replies.borrow().last(), Some(&("closed", false)));
        let length = replies.borrow().len();
        dialog.close();
        dialog.cancel();
        confirm.emit_clicked();
        assert_eq!(replies.borrow().len(), length);
        let output = replies.clone();
        dialog.present("Terminal", "Body", move |accepted| {
            output.borrow_mut().push(("terminal", accepted))
        });
        assert_eq!(replies.borrow().last(), Some(&("terminal", false)));
        assert!(!window.is_visible());

        // A GTK property callback may close the dialog while present updates its
        // labels. The outer request cannot remap it or leave its callback pending.
        let closing = Dialog::new().unwrap();
        let label: gtk::Label = styled(closing.window(), "dialog-overlay-heading");
        let weak = Rc::downgrade(&closing);
        label.connect_label_notify(move |_| {
            if let Some(dialog) = weak.upgrade() {
                dialog.close();
            }
        });
        let output = replies.clone();
        closing.present(
            "Close while setting text",
            "Must not show",
            move |accepted| {
                output
                    .borrow_mut()
                    .push(("closed-during-present", accepted))
            },
        );
        assert_eq!(
            replies.borrow().last(),
            Some(&("closed-during-present", false))
        );
        assert!(!closing.window().is_visible());
        drop(closing);

        let dropping = Dialog::new().unwrap();
        let retained = dropping.window().clone();
        let button: gtk::Button = styled(&retained, "dialog-overlay-confirm");
        let output = replies.clone();
        dropping.present("Drop", "Body", move |accepted| {
            output.borrow_mut().push(("dropped", accepted))
        });
        let weak = Rc::downgrade(&dropping);
        drop(dropping);
        assert!(weak.upgrade().is_none());
        assert!(!retained.is_visible());
        assert_eq!(replies.borrow().last(), Some(&("dropped", false)));
        let length = replies.borrow().len();
        button.emit_clicked();
        assert_eq!(replies.borrow().len(), length);
    })?;
    println!("dialog confirmation, cancellation, replacement, reentrancy and ownership passed");
    Ok(())
}
