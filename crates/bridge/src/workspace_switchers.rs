//! Temporary C startup/command hooks for Rust workspace, output and rename views.
use std::{
    cell::RefCell,
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
};
use way_shell::ui::{
    output_switcher::OutputSwitcher,
    rename_switcher::RenameSwitcher,
    workspace_switcher::{WorkspaceMode, WorkspaceSwitcher},
};

macro_rules! adapter {
    ($module:ident, $type:ty, $activate:ident, $get:ident) => {
        mod $module {
            use super::*;
            thread_local! {
                static GLOBAL: RefCell<Option<Rc<$type>>> = const { RefCell::new(None) };
            }
            pub(super) fn shutdown() {
                let controller = GLOBAL.with(|global| global.borrow_mut().take());
                if let Some(controller) = controller {
                    controller.switcher().close();
                }
            }
            pub(super) fn command(handle: *const c_void, action: impl FnOnce(Rc<$type>)) {
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    let controller = GLOBAL.with(|global| {
                        global
                            .borrow()
                            .as_ref()
                            .filter(|controller| Rc::as_ptr(controller).cast::<c_void>() == handle)
                            .cloned()
                    });
                    if let Some(controller) = controller {
                        action(controller);
                    }
                }));
            }
            #[unsafe(no_mangle)]
            pub extern "C" fn $activate(_app: *mut c_void, _data: *mut c_void) {
                let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
                    shutdown();
                    let manager =
                        crate::wm::service().ok_or("Compositor service has not started")?;
                    let controller = <$type>::new(manager)?;
                    GLOBAL.with(|global| global.replace(Some(controller)));
                    Ok(())
                }));
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => glib::g_warning!(
                        "way-shell",
                        "Could not start {}: {error}",
                        stringify!($module)
                    ),
                    Err(_) => {
                        glib::g_warning!("way-shell", "Could not start {}", stringify!($module))
                    }
                }
            }
            #[unsafe(no_mangle)]
            pub extern "C" fn $get() -> *const c_void {
                GLOBAL.with(|global| {
                    global
                        .borrow()
                        .as_ref()
                        .map_or(std::ptr::null(), |controller| Rc::as_ptr(controller).cast())
                })
            }
        }
    };
}
adapter!(
    workspace,
    WorkspaceSwitcher,
    workspace_switcher_activate,
    workspace_switcher_get_global
);
adapter!(
    output,
    OutputSwitcher,
    output_switcher_activate,
    output_switcher_get_global
);
adapter!(
    rename,
    RenameSwitcher,
    rename_switcher_activate,
    rename_switcher_get_global
);

pub fn shutdown() {
    workspace::shutdown();
    output::shutdown();
    rename::shutdown();
}

macro_rules! command {
    ($name:ident, $module:ident, $action:expr) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(handle: *const c_void) {
            $module::command(handle, $action);
        }
    };
}
command!(workspace_switcher_show, workspace, |view| view
    .show(WorkspaceMode::FocusWorkspace));
command!(workspace_switcher_hide, workspace, |view| view.hide());
command!(workspace_switcher_toggle, workspace, |view| view
    .toggle(WorkspaceMode::FocusWorkspace));
command!(workspace_switcher_show_app_mode, workspace, |view| view
    .show(WorkspaceMode::MoveWindow));
command!(workspace_switcher_toggle_app_mode, workspace, |view| view
    .toggle(WorkspaceMode::MoveWindow));
command!(output_switcher_show, output, |view| view.show());
command!(output_switcher_hide, output, |view| view.hide());
command!(output_switcher_toggle, output, |view| view.toggle());
command!(rename_switcher_show, rename, |view| view.show());
command!(rename_switcher_hide, rename, |view| view.hide());
command!(rename_switcher_toggle, rename, |view| view.toggle());
