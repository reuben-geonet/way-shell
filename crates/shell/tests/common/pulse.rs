//! Owned clients for the private Pulse protocol server. No process environment is changed.

#[path = "audio_policy.rs"]
mod audio_policy;
pub use audio_policy::Policy;

use glib::translate::ToGlibPtr;
use pulse::{context::*, operation::*, proplist::*, sample::*, stream::*};
use std::{
    cell::Cell,
    ffi::{CStr, CString, c_void},
    ptr::{self, NonNull},
    rc::Rc,
    time::{Duration, Instant},
};

struct Mainloop(NonNull<pulse_glib::pa_glib_mainloop>);

impl Drop for Mainloop {
    fn drop(&mut self) {
        // Every context and stream has already released its reference to this loop.
        unsafe { pulse_glib::pa_glib_mainloop_free(self.0.as_ptr()) };
    }
}

struct Connection {
    raw: NonNull<pa_context>,
    _mainloop: Mainloop,
    context: glib::MainContext,
}

impl Connection {
    fn state(&self) -> pa_context_state_t {
        unsafe { pa_context_get_state(self.raw.as_ptr()) }
    }

    fn error(&self) -> String {
        unsafe {
            let message = pulse::error::pa_strerror(pa_context_errno(self.raw.as_ptr()));
            CStr::from_ptr(message).to_string_lossy().into_owned()
        }
    }

    fn check_alive(&self) {
        assert!(
            !matches!(self.state(), PA_CONTEXT_FAILED | PA_CONTEXT_TERMINATED),
            "private Pulse connection failed: {}",
            self.error()
        );
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // Streams retain an Rc<Connection>, so they are disconnected first.
        unsafe {
            pa_context_disconnect(self.raw.as_ptr());
            pa_context_unref(self.raw.as_ptr());
        }
    }
}

pub struct Client(Rc<Connection>);

impl Client {
    pub fn new(context: &glib::MainContext, server: &str) -> Self {
        assert!(
            context.is_owner(),
            "Pulse fixture needs its owning GLib context"
        );
        // An explicit server and NOAUTOSPAWN ensure tests cannot use host audio.
        assert!(
            server
                .strip_prefix("unix:")
                .is_some_and(|path| std::path::Path::new(path).is_absolute()),
            "Pulse fixture needs an explicit absolute Unix server address"
        );
        let server = CString::new(server).unwrap();
        let mainloop = Mainloop(
            NonNull::new(unsafe { pulse_glib::pa_glib_mainloop_new(context.to_glib_none().0) })
                .expect("create Pulse GLib loop"),
        );
        let raw = NonNull::new(unsafe {
            pa_context_new(
                pulse_glib::pa_glib_mainloop_get_api(mainloop.0.as_ptr()),
                c"way-shell-routing-test".as_ptr(),
            )
        })
        .expect("create Pulse context");
        let client = Self(Rc::new(Connection {
            raw,
            _mainloop: mainloop,
            context: context.clone(),
        }));
        assert_eq!(
            unsafe {
                pa_context_connect(
                    raw.as_ptr(),
                    server.as_ptr(),
                    PA_CONTEXT_NOAUTOSPAWN,
                    ptr::null(),
                )
            },
            0,
            "connect private Pulse client: {}",
            client.0.error()
        );
        wait(context, || {
            client.0.check_alive();
            client.0.state() == PA_CONTEXT_READY
        });
        client
    }

    pub fn playback(&self, name: &str, target: Option<&str>) -> Stream {
        self.stream(name, target, Direction::Playback)
    }

    pub fn capture(&self, name: &str, target: Option<&str>) -> Stream {
        self.stream(name, target, Direction::Capture)
    }

    fn stream(&self, name: &str, target: Option<&str>, direction: Direction) -> Stream {
        let name = CString::new(name).unwrap();
        let target = target.map(|target| CString::new(target).unwrap());
        let target = target
            .as_ref()
            .map_or(ptr::null(), |target| target.as_ptr());
        let spec = pa_sample_spec {
            format: PA_SAMPLE_S16NE,
            rate: 48_000,
            channels: 2,
        };
        let raw = NonNull::new(unsafe {
            pa_stream_new(self.0.raw.as_ptr(), name.as_ptr(), &spec, ptr::null())
        })
        .expect("create private Pulse stream");
        let stream = Stream {
            raw,
            connection: self.0.clone(),
            direction,
        };
        let result = unsafe {
            match direction {
                Direction::Playback => pa_stream_connect_playback(
                    raw.as_ptr(),
                    target,
                    ptr::null(),
                    PA_STREAM_START_CORKED,
                    ptr::null(),
                    ptr::null_mut(),
                ),
                Direction::Capture => pa_stream_connect_record(
                    raw.as_ptr(),
                    target,
                    ptr::null(),
                    PA_STREAM_START_CORKED,
                ),
            }
        };
        assert_eq!(
            result,
            0,
            "connect private Pulse stream: {}",
            self.0.error()
        );
        wait(&self.0.context, || {
            self.0.check_alive();
            let state = unsafe { pa_stream_get_state(raw.as_ptr()) };
            assert!(
                !matches!(state, PA_STREAM_FAILED | PA_STREAM_TERMINATED),
                "private Pulse stream failed: {}",
                self.0.error()
            );
            state == PA_STREAM_READY
        });
        stream
    }
}

#[derive(Clone, Copy)]
enum Direction {
    Playback,
    Capture,
}

pub struct Stream {
    raw: NonNull<pa_stream>,
    connection: Rc<Connection>,
    direction: Direction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamInfo {
    pub node_id: Option<u32>,
    pub serial: Option<u64>,
    pub destination_index: u32,
    pub corked: bool,
}

impl Stream {
    pub fn index(&self) -> u32 {
        unsafe { pa_stream_get_index(self.raw.as_ptr()) }
    }

    /// The server updates this name after a successful stream move.
    pub fn device_name(&self) -> Option<String> {
        let name = unsafe { pa_stream_get_device_name(self.raw.as_ptr()) };
        (!name.is_null()).then(|| unsafe { CStr::from_ptr(name).to_string_lossy().into_owned() })
    }

    /// Obtain a new server reply, independently of the application's inventory.
    pub fn info(&self) -> StreamInfo {
        let query = Box::<Query>::default();
        let userdata = (&*query as *const Query).cast_mut().cast::<c_void>();
        let operation = unsafe {
            match self.direction {
                Direction::Playback => pa_context_get_sink_input_info(
                    self.connection.raw.as_ptr(),
                    self.index(),
                    Some(playback_info),
                    userdata,
                ),
                Direction::Capture => pa_context_get_source_output_info(
                    self.connection.raw.as_ptr(),
                    self.index(),
                    Some(capture_info),
                    userdata,
                ),
            }
        };
        let pending = Pending {
            operation: NonNull::new(operation).expect("query private Pulse stream"),
            query,
        };
        wait(&self.connection.context, || {
            self.connection.check_alive();
            pending.query.done.get()
        });
        assert!(
            !pending.query.failed.get(),
            "private Pulse stream query failed"
        );
        pending
            .query
            .info
            .get()
            .expect("private Pulse stream exists")
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe {
            pa_stream_disconnect(self.raw.as_ptr());
            pa_stream_unref(self.raw.as_ptr());
        }
    }
}

#[derive(Default)]
struct Query {
    info: Cell<Option<StreamInfo>>,
    done: Cell<bool>,
    failed: Cell<bool>,
}

struct Pending {
    operation: NonNull<pa_operation>,
    query: Box<Query>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        // Cancellation prevents further callbacks before the userdata is freed,
        // including if a timeout or disconnected daemon panics in the test.
        unsafe {
            if pa_operation_get_state(self.operation.as_ptr()) == PA_OPERATION_RUNNING {
                pa_operation_cancel(self.operation.as_ptr());
            }
            pa_operation_unref(self.operation.as_ptr());
        }
    }
}

extern "C" fn playback_info(
    _: *mut pa_context,
    info: *const pa_sink_input_info,
    end: i32,
    userdata: *mut c_void,
) {
    // libpulse supplies this only while the owning Pending exists on this thread.
    let query = unsafe { &*userdata.cast::<Query>() };
    if end != 0 {
        query.failed.set(end < 0);
        query.done.set(true);
    } else if let Some(info) = unsafe { info.as_ref() } {
        query
            .info
            .set(Some(stream_info(info.proplist, info.sink, info.corked)));
    }
}

extern "C" fn capture_info(
    _: *mut pa_context,
    info: *const pa_source_output_info,
    end: i32,
    userdata: *mut c_void,
) {
    let query = unsafe { &*userdata.cast::<Query>() };
    if end != 0 {
        query.failed.set(end < 0);
        query.done.set(true);
    } else if let Some(info) = unsafe { info.as_ref() } {
        query
            .info
            .set(Some(stream_info(info.proplist, info.source, info.corked)));
    }
}

fn stream_info(properties: *mut pa_proplist, destination_index: u32, corked: i32) -> StreamInfo {
    let property = |key: &CStr| {
        if properties.is_null() {
            return None;
        }
        let value = unsafe { pa_proplist_gets(properties, key.as_ptr()) };
        if value.is_null() {
            return None;
        }
        unsafe { CStr::from_ptr(value) }
            .to_str()
            .ok()?
            .parse::<u64>()
            .ok()
    };
    StreamInfo {
        node_id: property(c"object.id").and_then(|value| value.try_into().ok()),
        serial: property(c"object.serial"),
        destination_index,
        corked: corked != 0,
    }
}

#[track_caller]
fn wait(context: &glib::MainContext, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(Instant::now() < deadline, "private Pulse fixture timed out");
        context.block_on(glib::timeout_future(Duration::from_millis(10)));
    }
}
