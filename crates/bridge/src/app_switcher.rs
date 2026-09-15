//! Temporary C startup/command entry points for the Rust application switcher.
use std::{
    cell::RefCell,
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
};
use way_shell::ui::app_switcher::AppSwitcher;

thread_local! {
    static GLOBAL: RefCell<Option<Rc<AppSwitcher>>> = const { RefCell::new(None) };
}

pub fn shutdown() {
    let switcher = GLOBAL.with(|global| global.borrow_mut().take());
    if let Some(switcher) = switcher {
        switcher.close();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn app_switcher_activate(_app: *mut c_void, _data: *mut c_void) {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
        shutdown();
        let service = crate::wayland::service().ok_or("Wayland service has not started")?;
        let switcher = AppSwitcher::new(service)?;
        GLOBAL.with(|global| global.replace(Some(switcher)));
        Ok(())
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => glib::g_warning!("way-shell", "Could not start app switcher: {error}"),
        Err(_) => glib::g_warning!("way-shell", "Could not start app switcher"),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn app_switcher_get_global() -> *const c_void {
    GLOBAL.with(|global| {
        global
            .borrow()
            .as_ref()
            .map_or(std::ptr::null(), |switcher| Rc::as_ptr(switcher).cast())
    })
}

fn current(handle: *const c_void) -> Option<Rc<AppSwitcher>> {
    GLOBAL.with(|global| {
        global
            .borrow()
            .as_ref()
            .filter(|switcher| Rc::as_ptr(switcher).cast::<c_void>() == handle)
            .cloned()
    })
}

macro_rules! command {
    ($name:ident, $method:ident) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(handle: *const c_void) {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                if let Some(switcher) = current(handle) {
                    switcher.$method();
                }
            }));
        }
    };
}
command!(app_switcher_show, show);
command!(app_switcher_hide, hide);
command!(app_switcher_toggle, toggle);
