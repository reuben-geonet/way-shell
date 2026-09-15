//! Temporary startup and hide commands for the Rust level overlay.
use std::{
    cell::RefCell,
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
};
use way_shell::ui::osd::LevelOsd;
thread_local! { static GLOBAL: RefCell<Option<Rc<LevelOsd>>> = const { RefCell::new(None) }; }
pub fn shutdown() {
    let view = GLOBAL.with(|global| global.borrow_mut().take());
    if let Some(view) = view {
        view.close();
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn osd_activate(_app: *mut c_void, _data: *mut c_void) {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
        shutdown();
        let view = LevelOsd::new(
            crate::audio::service().ok_or("Audio service has not started")?,
            crate::brightness::service().ok_or("Brightness service has not started")?,
            crate::quick_settings::is_visible,
        )?;
        GLOBAL.with(|global| global.replace(Some(view)));
        Ok(())
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => glib::g_warning!("way-shell", "Could not start level overlay: {error}"),
        Err(_) => glib::g_warning!("way-shell", "Could not start level overlay"),
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn osd_get_global() -> *const c_void {
    GLOBAL.with(|global| {
        global
            .borrow()
            .as_ref()
            .map_or(std::ptr::null(), |view| Rc::as_ptr(view).cast())
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn osd_set_hidden(handle: *const c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let view = GLOBAL.with(|global| {
            global
                .borrow()
                .as_ref()
                .filter(|view| Rc::as_ptr(view).cast::<c_void>() == handle)
                .cloned()
        });
        if let Some(view) = view {
            view.hide();
        }
    }));
}
