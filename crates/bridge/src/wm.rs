//! C snapshots and callbacks for the temporary WindowManager vtable.
use glib::{ffi, prelude::*};
use std::{
    cell::RefCell,
    ffi::{CStr, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
};
use way_shell::services::wm::WindowManager;
use way_shell_core::wm::{Action, Output, Workspace, WorkspaceTarget};

#[repr(C)]
struct CWorkspace {
    name: *mut c_char,
    output: *mut c_char,
    id: u64,
    num: i32,
    urgent: i32,
    focused: i32,
    visible: i32,
    empty: i32,
}
#[repr(C)]
struct COutput {
    name: *mut c_char,
    make: *mut c_char,
    model: *mut c_char,
    serial: *mut c_char,
    current_workspace: *mut c_char,
}

unsafe extern "C" fn free_workspace(pointer: *mut c_void) {
    // Every entry and its strings were allocated with GLib and are owned by
    // this GPtrArray; consumers retain the array, never individual entries.
    unsafe {
        let entry = &*pointer.cast::<CWorkspace>();
        ffi::g_free(entry.name.cast());
        ffi::g_free(entry.output.cast());
        ffi::g_free(pointer);
    }
}
unsafe extern "C" fn free_output(pointer: *mut c_void) {
    unsafe {
        let entry = &*pointer.cast::<COutput>();
        for value in [
            entry.name,
            entry.make,
            entry.model,
            entry.serial,
            entry.current_workspace,
        ] {
            ffi::g_free(value.cast());
        }
        ffi::g_free(pointer);
    }
}
fn string(value: Option<&str>) -> *mut c_char {
    value.map_or(std::ptr::null_mut(), |value| unsafe {
        ffi::g_strndup(value.as_ptr().cast(), value.len())
    })
}

struct Array(*mut ffi::GPtrArray);
impl Drop for Array {
    fn drop(&mut self) {
        unsafe {
            ffi::g_ptr_array_unref(self.0);
        }
    }
}
impl Array {
    fn workspaces(entries: &[Workspace]) -> Self {
        unsafe {
            let array = ffi::g_ptr_array_new_with_free_func(Some(free_workspace));
            for entry in entries {
                let pointer =
                    ffi::g_malloc0(std::mem::size_of::<CWorkspace>()).cast::<CWorkspace>();
                pointer.write(CWorkspace {
                    name: string(Some(&entry.name)),
                    output: string(entry.output.as_deref()),
                    id: entry.id,
                    num: entry.num,
                    urgent: entry.urgent.into(),
                    focused: entry.focused.into(),
                    visible: entry.visible.into(),
                    empty: entry.empty.into(),
                });
                ffi::g_ptr_array_add(array, pointer.cast());
            }
            Self(array)
        }
    }
    fn outputs(entries: &[Output]) -> Self {
        unsafe {
            let array = ffi::g_ptr_array_new_with_free_func(Some(free_output));
            for entry in entries {
                let pointer = ffi::g_malloc0(std::mem::size_of::<COutput>()).cast::<COutput>();
                pointer.write(COutput {
                    name: string(Some(&entry.name)),
                    make: string(entry.make.as_deref()),
                    model: string(entry.model.as_deref()),
                    serial: string(entry.serial.as_deref()),
                    current_workspace: string(entry.current_workspace.as_deref()),
                });
                ffi::g_ptr_array_add(array, pointer.cast());
            }
            Self(array)
        }
    }
    fn retain(&self) -> *mut ffi::GPtrArray {
        unsafe { ffi::g_ptr_array_ref(self.0) }
    }
}

type Notify = unsafe extern "C" fn(*mut c_void, *mut ffi::GPtrArray);
struct Manager {
    service: WindowManager,
    workspaces: Rc<RefCell<Array>>,
    outputs: Rc<RefCell<Array>>,
    handlers: Vec<glib::SignalHandlerId>,
}
impl Drop for Manager {
    fn drop(&mut self) {
        for handler in self.handlers.drain(..) {
            self.service.disconnect(handler);
        }
    }
}

impl Manager {
    fn new(
        service: WindowManager,
        workspaces_changed: Option<Notify>,
        outputs_changed: Option<Notify>,
        data: *mut c_void,
    ) -> Self {
        let workspaces = Rc::new(RefCell::new(Array::workspaces(&service.workspaces())));
        let outputs = Rc::new(RefCell::new(Array::outputs(&service.outputs())));
        let snapshot = workspaces.clone();
        let workspaces_handler =
            service.connect_local("workspaces-changed", false, move |values| {
                let service = values[0].get::<WindowManager>().unwrap();
                let array = Array::workspaces(&service.workspaces());
                let pointer = array.0;
                snapshot.replace(array);
                if let Some(callback) = workspaces_changed {
                    unsafe {
                        callback(data, pointer);
                    }
                }
                None
            });
        let snapshot = outputs.clone();
        let outputs_handler = service.connect_local("outputs-changed", false, move |values| {
            let service = values[0].get::<WindowManager>().unwrap();
            let array = Array::outputs(&service.outputs());
            let pointer = array.0;
            snapshot.replace(array);
            if let Some(callback) = outputs_changed {
                unsafe {
                    callback(data, pointer);
                }
            }
            None
        });
        Self {
            service,
            workspaces,
            outputs,
            handlers: vec![workspaces_handler, outputs_handler],
        }
    }

    fn target(&self, id: u64, number: i32, name: String) -> WorkspaceTarget {
        // C also creates temporary targets for typed workspace names. Only
        // associate an ID when it belongs to the current named workspace.
        let id = self
            .service
            .workspaces()
            .iter()
            .find(|entry| entry.id == id && entry.name == name)
            .map(|entry| entry.id);
        WorkspaceTarget { id, number, name }
    }
}

fn status(action: impl FnOnce() -> Result<(), String>) -> i32 {
    match catch_unwind(AssertUnwindSafe(action)) {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => {
            glib::g_warning!("way-shell", "Window manager action failed: {error}");
            -1
        }
        Err(_) => -1,
    }
}

/// # Safety
/// Callbacks and their borrowed data remain valid until way_shell_wm_free.
/// All calls and destruction occur on the application thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn way_shell_wm_new_sway(
    workspaces: Option<Notify>,
    outputs: Option<Notify>,
    data: *mut c_void,
) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| match WindowManager::sway() {
        Ok(service) => {
            Box::into_raw(Box::new(Manager::new(service, workspaces, outputs, data))).cast()
        }
        Err(error) => {
            glib::g_warning!("way-shell", "Could not initialize Sway: {error}");
            std::ptr::null_mut()
        }
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// Callbacks and their borrowed data remain valid until way_shell_wm_free.
/// All calls and destruction occur on the application thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn way_shell_wm_new_niri(
    workspaces: Option<Notify>,
    outputs: Option<Notify>,
    data: *mut c_void,
) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| match WindowManager::niri() {
        Ok(service) => {
            Box::into_raw(Box::new(Manager::new(service, workspaces, outputs, data))).cast()
        }
        Err(error) => {
            glib::g_warning!("way-shell", "Could not initialize Niri: {error}");
            std::ptr::null_mut()
        }
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// A non-null handle is live and owned, returned by either compositor constructor.
/// This consumes it; no calls or callbacks may use it afterward.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn way_shell_wm_free(handle: *mut c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !handle.is_null() {
            drop(unsafe { Box::from_raw(handle.cast::<Manager>()) });
        }
    }));
}

/// # Safety
/// A non-null handle remains live on this thread. The result is an owned
/// GPtrArray reference; C releases it with g_ptr_array_unref.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn way_shell_wm_workspaces(handle: *mut c_void) -> *mut ffi::GPtrArray {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { handle.cast::<Manager>().as_ref() }.map_or(std::ptr::null_mut(), |manager| {
            manager.workspaces.borrow().retain()
        })
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// Same ownership rules as way_shell_wm_workspaces.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn way_shell_wm_outputs(handle: *mut c_void) -> *mut ffi::GPtrArray {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { handle.cast::<Manager>().as_ref() }.map_or(std::ptr::null_mut(), |manager| {
            manager.outputs.borrow().retain()
        })
    }))
    .unwrap_or(std::ptr::null_mut())
}

unsafe fn name(pointer: *const c_char) -> Result<String, String> {
    if pointer.is_null() {
        return Err("Missing workspace or output name".into());
    }
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map(str::to_owned)
        .map_err(|_| "Workspace or output name is not UTF-8".into())
}

/// # Safety
/// handle is live and name is null or a borrowed NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn way_shell_wm_workspace_action(
    handle: *mut c_void,
    move_window: i32,
    id: u64,
    number: i32,
    pointer: *const c_char,
) -> i32 {
    status(|| {
        let manager =
            unsafe { handle.cast::<Manager>().as_ref() }.ok_or("Missing window manager")?;
        let target = manager.target(id, number, unsafe { name(pointer) }?);
        manager.service.perform(&if move_window != 0 {
            Action::MoveWindowToWorkspace(target)
        } else {
            Action::FocusWorkspace(target)
        })
    })
}

/// # Safety
/// handle is live and name is null or a borrowed NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn way_shell_wm_named_action(
    handle: *mut c_void,
    move_output: i32,
    pointer: *const c_char,
) -> i32 {
    status(|| {
        let manager =
            unsafe { handle.cast::<Manager>().as_ref() }.ok_or("Missing window manager")?;
        let name = unsafe { name(pointer) }?;
        manager.service.perform(&if move_output != 0 {
            Action::MoveWorkspaceToOutput(name)
        } else {
            Action::RenameWorkspace(name)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn c_snapshot_retains_strings_and_full_width_identifiers() {
        let array = Array::workspaces(&[Workspace {
            id: u64::MAX,
            name: "日本語".into(),
            ..Default::default()
        }]);
        let retained = Array(array.retain());
        drop(array);
        unsafe {
            assert_eq!((*retained.0).len, 1);
            let workspace = &*(*(*retained.0).pdata).cast::<CWorkspace>();
            assert_eq!(workspace.id, u64::MAX);
            assert_eq!(CStr::from_ptr(workspace.name).to_str().unwrap(), "日本語");
            assert!(workspace.output.is_null());
        }
    }
}
