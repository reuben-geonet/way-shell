//! libcanberra playback runs separately from clipboard capture and the GTK loop.
use std::{
    ffi::{CString, c_char, c_int, c_void},
    sync::mpsc,
};

#[link(name = "canberra")]
unsafe extern "C" {
    fn ca_context_create(context: *mut *mut c_void) -> c_int;
    fn ca_context_destroy(context: *mut c_void) -> c_int;
    fn ca_context_cancel(context: *mut c_void, id: u32) -> c_int;
    fn ca_context_play(context: *mut c_void, id: u32, first: *const c_char, ...) -> c_int;
}

pub struct Sound(mpsc::Sender<(String, f64)>);
impl Sound {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel::<(String, f64)>();
        std::thread::spawn(move || {
            let mut context = std::ptr::null_mut();
            // Only this worker creates, accesses and destroys the canberra context.
            if unsafe { ca_context_create(&mut context) } < 0 {
                return;
            }
            while let Ok((name, volume)) = receiver.recv() {
                let file = glib::system_data_dirs()
                    .iter()
                    .map(|dir| dir.join(format!("sounds/gnome/default/alerts/{name}.ogg")))
                    .find(|path| path.is_file());
                let property = if file.is_some() {
                    c"media.filename"
                } else {
                    c"event.id"
                };
                let value = file
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or(name);
                let Ok(value) = CString::new(value) else {
                    continue;
                };
                let volume = CString::new(volume.to_string()).unwrap();
                unsafe {
                    ca_context_cancel(context, 1);
                    ca_context_play(
                        context,
                        1,
                        property.as_ptr(),
                        value.as_ptr(),
                        c"canberra.volume".as_ptr(),
                        volume.as_ptr(),
                        std::ptr::null::<c_char>(),
                    );
                }
            }
            unsafe {
                ca_context_destroy(context);
            }
        });
        Self(sender)
    }
    pub fn play(&self, name: String, volume: f64) {
        let _ = self.0.send((name, volume));
    }
}
