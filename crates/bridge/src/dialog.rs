//! Temporary startup hook; confirmations are owned entirely by the Rust dialog.
use std::{
    cell::RefCell,
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
};
use way_shell::ui::dialog::Dialog;

thread_local! { static GLOBAL: RefCell<Option<Rc<Dialog>>> = const { RefCell::new(None) }; }

pub(crate) fn present(heading: &str, body: &str, response: Box<dyn FnOnce(bool)>) {
    let dialog = GLOBAL.with(|global| global.borrow().clone());
    if let Some(dialog) = dialog {
        dialog.present(heading, body, response);
    } else {
        response(false);
        glib::g_message!("way-shell", "Confirmation dialog is unavailable");
    }
}

pub fn shutdown() {
    let dialog = GLOBAL.with(|global| global.borrow_mut().take());
    if let Some(dialog) = dialog {
        dialog.close();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn dialog_overlay_activate(_app: *mut c_void, _data: *mut c_void) {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
        shutdown();
        let dialog = Dialog::new()?;
        GLOBAL.with(|global| global.replace(Some(dialog)));
        Ok(())
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            glib::g_warning!("way-shell", "Could not start confirmation dialog: {error}")
        }
        Err(_) => glib::g_warning!("way-shell", "Could not start confirmation dialog"),
    }
}
