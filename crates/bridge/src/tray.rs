//! Temporary startup hook; Rust panels consume TrayService directly.
use gio::prelude::*;
use std::{
    cell::RefCell,
    panic::{AssertUnwindSafe, catch_unwind},
};
use way_shell::services::{settings, tray::TrayService};

thread_local! { static GLOBAL: RefCell<Option<TrayService>> = const { RefCell::new(None) }; }

pub(crate) fn service() -> Option<TrayService> {
    GLOBAL.with(|global| global.borrow().clone())
}
pub fn shutdown() {
    let service = GLOBAL.with(|global| global.borrow_mut().take());
    if let Some(service) = service {
        service.stop();
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn status_notifier_service_global_init() -> i32 {
    match catch_unwind(AssertUnwindSafe(|| -> Result<(), glib::BoolError> {
        if service().is_some() {
            return Ok(());
        }
        let settings = settings::open("org.ldelossa.way-shell.panel")?;
        if settings.boolean("enable-tray-icons") {
            GLOBAL.with(|global| global.replace(Some(TrayService::new())));
        }
        Ok(())
    })) {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => {
            glib::g_message!("way-shell", "Could not initialize tray: {error}");
            -1
        }
        Err(_) => -1,
    }
}
