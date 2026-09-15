//! Temporary startup and visibility hooks while the surrounding UI remains C.
use std::{
    cell::RefCell,
    panic::{AssertUnwindSafe, catch_unwind},
};
use way_shell::{
    services::settings,
    ui::panel::{PanelAction, PanelServices, Panels, status::StatusServices},
};

thread_local! { static GLOBAL: RefCell<Option<Panels>> = const { RefCell::new(None) }; }

fn services() -> Result<PanelServices, &'static str> {
    Ok(PanelServices {
        clock: crate::clock::service().ok_or("Clock service has not started")?,
        notifications: crate::notifications::service()
            .ok_or("Notification service has not started")?,
        manager: crate::wm::service().ok_or("Compositor service has not started")?,
        status: StatusServices {
            audio: crate::audio::service().ok_or("Audio service has not started")?,
            network: crate::network::service().ok_or("Network service has not started")?,
            power: crate::power::service().ok_or("Power service has not started")?,
            logind: crate::logind::service().ok_or("Session service has not started")?,
            wayland: crate::wayland::service().ok_or("Wayland service has not started")?,
        },
        tray: crate::tray::service(),
    })
}

pub fn shutdown() {
    let panels = GLOBAL.with(|global| global.borrow_mut().take());
    if let Some(panels) = panels {
        panels.stop();
    }
}

/// # Safety
/// Call on the initialized GTK main thread. Callbacks remain valid until shutdown
/// and target the live C message tray / quick settings controllers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn way_shell_panels_init(
    message_tray: Option<unsafe extern "C" fn()>,
    quick_settings: Option<unsafe extern "C" fn()>,
) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
        shutdown();
        let message_tray = message_tray.ok_or("Missing message-tray callback")?;
        let quick_settings = quick_settings.ok_or("Missing quick-settings callback")?;
        let display = gtk::gdk::Display::default().ok_or("No GTK display is available")?;
        let panels = Panels::new(
            &display,
            services()?,
            settings::open("org.ldelossa.way-shell.panel").map_err(|error| error.to_string())?,
            settings::open("org.ldelossa.way-shell.notifications")
                .map_err(|error| error.to_string())?,
            move |action| unsafe {
                match action {
                    PanelAction::ToggleMessageTray => message_tray(),
                    PanelAction::ToggleQuickSettings => quick_settings(),
                }
            },
        )?;
        GLOBAL.with(|global| global.replace(Some(panels)));
        Ok(())
    }));
    match result {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => {
            glib::g_message!("way-shell", "Could not start panels: {error}");
            -1
        }
        Err(_) => -1,
    }
}

macro_rules! visibility {
    ($name:ident, $method:ident) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(visible: i32) {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let panels = GLOBAL.with(|global| global.borrow().clone());
                if let Some(panels) = panels {
                    panels.$method(visible != 0);
                }
            }));
        }
    };
}
visibility!(panel_set_message_tray_visible, set_message_tray_visible);
visibility!(panel_set_quick_settings_visible, set_quick_settings_visible);
visibility!(panel_set_activities_visible, set_activities_visible);
