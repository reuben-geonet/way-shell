//! PulseAudio routing on the application's GLib context. Native callbacks only
//! collect results; owned futures cancel operations before freeing callback data.
use glib::translate::ToGlibPtr;
use pulse::{context::*, operation::*, proplist::*};
use std::{
    cell::{Cell, RefCell},
    ffi::{CStr, CString, c_void},
    future::{Future, poll_fn},
    pin::Pin,
    ptr::NonNull,
    rc::{Rc, Weak},
    task::{Context, Poll, Waker},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteError {
    Unavailable,
    InvalidEndpoints,
    EndpointRemoved,
    StreamNotFound,
    InvalidName,
    Failed(String),
    Cancelled,
    Timeout,
}
impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => f.write_str("Stream routing is unavailable"),
            Self::InvalidEndpoints => f.write_str("Incompatible audio routing endpoints"),
            Self::EndpointRemoved => f.write_str("An audio routing endpoint was removed"),
            Self::StreamNotFound => f.write_str("Stream is not available through PulseAudio"),
            Self::InvalidName => f.write_str("Invalid audio routing name"),
            Self::Failed(message) => write!(f, "Stream routing failed: {message}"),
            Self::Cancelled => f.write_str("Stream routing was cancelled"),
            Self::Timeout => f.write_str("Stream routing exceeded its two-second deadline"),
        }
    }
}
impl std::error::Error for RouteError {}

use super::{AudioService, AudioState, NodeKind};
use glib::prelude::*;
use glib::subclass::prelude::*;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Endpoint {
    id: u32,
    serial: u64,
    kind: NodeKind,
    name: String,
}
impl Endpoint {
    fn from_state(state: &AudioState, id: u32) -> Result<Self, RouteError> {
        let node = state.node(id).ok_or(RouteError::EndpointRemoved)?;
        if node.serial == 0 {
            return Err(RouteError::EndpointRemoved);
        }
        Ok(Self {
            id,
            serial: node.serial,
            kind: node.kind,
            name: node.name.clone(),
        })
    }
    fn present(&self, state: &AudioState) -> bool {
        Self::from_state(state, self.id).as_ref() == Ok(self)
    }
}
impl AudioService {
    /// Move a playback stream to a sink or a capture stream to a source.
    /// The future owns its native requests and cancels them when dropped. It
    /// holds a weak service reference, so shutdown also cancels pending work.
    /// Success is reported only after the PulseAudio server acknowledges the move.
    pub fn route(
        &self,
        stream: u32,
        target: u32,
    ) -> impl Future<Output = Result<(), RouteError>> + 'static {
        let weak = self.downgrade();
        let generation = self.imp().generation.get();
        let prepared = (|| {
            let state = self.state();
            if !state.available || !self.imp().routing_enabled.get() {
                return Err(RouteError::Unavailable);
            }
            let stream = Endpoint::from_state(&state, stream)?;
            let target = Endpoint::from_state(&state, target)?;
            if !matches!(
                (stream.kind, target.kind),
                (NodeKind::OutputStream, NodeKind::Sink)
                    | (NodeKind::InputStream, NodeKind::Source)
            ) {
                return Err(RouteError::InvalidEndpoints);
            }
            let connection = self
                .imp()
                .router
                .borrow()
                .connection(self.imp().pulse_server.borrow().as_deref())?;
            Ok((stream, target, connection))
        })();
        async move {
            let (stream, target, connection) = prepared?;
            let valid = || {
                let service = weak.upgrade().ok_or(RouteError::Cancelled)?;
                if service.imp().generation.get() != generation {
                    return Err(RouteError::Cancelled);
                }
                let state = service.state();
                if !state.available || !stream.present(&state) || !target.present(&state) {
                    return Err(RouteError::EndpointRemoved);
                }
                Ok(())
            };
            let result = glib::future_with_timeout(Duration::from_secs(2), async {
                valid()?;
                let capture = stream.kind == NodeKind::InputStream;
                let index = connection.stream_index(stream.serial, capture).await?;
                valid()?;
                connection.move_stream(index, &target.name, capture).await?;
                valid()
            })
            .await;
            match result {
                Ok(result) => result,
                Err(_) => {
                    // Do not cache a stalled handshake for later user requests.
                    if !connection.is_ready() {
                        connection.close();
                    }
                    Err(RouteError::Timeout)
                }
            }
        }
    }
}

#[derive(Default)]
struct Wake(RefCell<Option<Waker>>);
impl Wake {
    fn register(&self, context: &Context<'_>) {
        self.0.replace(Some(context.waker().clone()));
    }
    fn notify(&self) {
        let waker = self.0.borrow_mut().take();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}
#[derive(Default)]
struct ConnectionState {
    closed: Cell<bool>,
    waiters: RefCell<Vec<Weak<Wake>>>,
}
impl ConnectionState {
    fn watch(&self) -> Rc<Wake> {
        let wake = Rc::new(Wake::default());
        let mut waiters = self.waiters.borrow_mut();
        waiters.retain(|w| w.strong_count() > 0);
        waiters.push(Rc::downgrade(&wake));
        wake
    }
    fn notify(&self) {
        let live: Vec<_> = {
            let mut waiters = self.waiters.borrow_mut();
            waiters.retain(|w| w.strong_count() > 0);
            waiters.iter().filter_map(Weak::upgrade).collect()
        };
        for waiter in live {
            waiter.notify();
        }
    }
}

pub(super) struct Connection {
    context: NonNull<pa_context>,
    mainloop: NonNull<pulse_glib::pa_glib_mainloop>,
    state: Box<ConnectionState>,
    // Keep the owning GLib context alive and make this wrapper thread-local.
    _owner: glib::MainContext,
    _local: std::marker::PhantomData<Rc<()>>,
}
extern "C" fn state_changed(_: *mut pa_context, data: *mut c_void) {
    // SAFETY: Connection owns this stable box until it disconnects the callback.
    let state = unsafe { &*data.cast::<ConnectionState>() };
    state.notify();
}
fn native_error(context: *mut pa_context) -> RouteError {
    // SAFETY: callers own the live context; pa_strerror returns a static string.
    let message = unsafe { CStr::from_ptr(pulse::error::pa_strerror(pa_context_errno(context))) }
        .to_string_lossy()
        .into_owned();
    RouteError::Failed(message)
}
impl Connection {
    fn new(server: Option<&str>) -> Result<Rc<Self>, RouteError> {
        let server = server
            .map(CString::new)
            .transpose()
            .map_err(|_| RouteError::InvalidName)?;
        let owner = glib::MainContext::ref_thread_default();
        // SAFETY: the mainloop is attached to and only used on this owning context.
        let mainloop =
            NonNull::new(unsafe { pulse_glib::pa_glib_mainloop_new(owner.to_glib_none().0) })
                .ok_or(RouteError::Unavailable)?;
        let context = NonNull::new(unsafe {
            pa_context_new(
                pulse_glib::pa_glib_mainloop_get_api(mainloop.as_ptr()),
                c"org.ldelossa.way-shell".as_ptr(),
            )
        });
        let Some(context) = context else {
            unsafe {
                pulse_glib::pa_glib_mainloop_free(mainloop.as_ptr());
            }
            return Err(RouteError::Unavailable);
        };
        let connection = Rc::new(Self {
            context,
            mainloop,
            state: Box::default(),
            _owner: owner,
            _local: std::marker::PhantomData,
        });
        unsafe {
            pa_context_set_state_callback(
                context.as_ptr(),
                Some(state_changed),
                (&*connection.state as *const ConnectionState)
                    .cast_mut()
                    .cast(),
            );
            if pa_context_connect(
                context.as_ptr(),
                server.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                PA_CONTEXT_NOAUTOSPAWN,
                std::ptr::null(),
            ) < 0
            {
                return Err(native_error(context.as_ptr()));
            }
        }
        Ok(connection)
    }
    fn failed(&self) -> bool {
        self.state.closed.get()
            || matches!(
                unsafe { pa_context_get_state(self.context.as_ptr()) },
                PA_CONTEXT_FAILED | PA_CONTEXT_TERMINATED
            )
    }
    fn is_ready(&self) -> bool {
        !self.state.closed.get()
            && unsafe { pa_context_get_state(self.context.as_ptr()) } == PA_CONTEXT_READY
    }
    fn close(&self) {
        if !self.state.closed.replace(true) {
            unsafe {
                pa_context_disconnect(self.context.as_ptr());
            }
            self.state.notify();
        }
    }
    async fn ready(&self) -> Result<(), RouteError> {
        let wake = self.state.watch();
        poll_fn(|cx| {
            wake.register(cx);
            if self.state.closed.get() {
                return Poll::Ready(Err(RouteError::Cancelled));
            }
            match unsafe { pa_context_get_state(self.context.as_ptr()) } {
                PA_CONTEXT_READY => Poll::Ready(Ok(())),
                PA_CONTEXT_FAILED | PA_CONTEXT_TERMINATED => {
                    Poll::Ready(Err(native_error(self.context.as_ptr())))
                }
                _ => Poll::Pending,
            }
        })
        .await
    }
    pub async fn stream_index(
        self: &Rc<Self>,
        serial: u64,
        capture: bool,
    ) -> Result<u32, RouteError> {
        self.ready().await?;
        let mut request = Request::new(self.clone(), serial);
        let data = (&mut *request.data as *mut CallbackData<u32>).cast();
        let operation = unsafe {
            if capture {
                pa_context_get_source_output_info_list(
                    self.context.as_ptr(),
                    Some(source_info),
                    data,
                )
            } else {
                pa_context_get_sink_input_info_list(self.context.as_ptr(), Some(sink_info), data)
            }
        };
        request.operation =
            Some(NonNull::new(operation).ok_or_else(|| native_error(self.context.as_ptr()))?);
        request.await
    }
    pub async fn move_stream(
        self: &Rc<Self>,
        index: u32,
        target: &str,
        capture: bool,
    ) -> Result<(), RouteError> {
        let target = CString::new(target).map_err(|_| RouteError::InvalidName)?;
        if self.failed() {
            return Err(RouteError::Unavailable);
        }
        let mut request = Request::new(self.clone(), 0);
        let data = (&mut *request.data as *mut CallbackData<()>).cast();
        let operation = unsafe {
            if capture {
                pa_context_move_source_output_by_name(
                    self.context.as_ptr(),
                    index,
                    target.as_ptr(),
                    Some(moved),
                    data,
                )
            } else {
                pa_context_move_sink_input_by_name(
                    self.context.as_ptr(),
                    index,
                    target.as_ptr(),
                    Some(moved),
                    data,
                )
            }
        };
        request.operation =
            Some(NonNull::new(operation).ok_or_else(|| native_error(self.context.as_ptr()))?);
        request.await
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        // Disconnect callbacks before releasing their userdata and native owners.
        unsafe {
            pa_context_set_state_callback(self.context.as_ptr(), None, std::ptr::null_mut());
            pa_context_disconnect(self.context.as_ptr());
            pa_context_unref(self.context.as_ptr());
            pulse_glib::pa_glib_mainloop_free(self.mainloop.as_ptr());
        }
    }
}

#[derive(Default)]
pub(super) struct Router(RefCell<Option<Rc<Connection>>>);
impl Router {
    pub fn connection(&self, server: Option<&str>) -> Result<Rc<Connection>, RouteError> {
        let mut current = self.0.borrow_mut();
        if current.as_ref().is_none_or(|c| c.failed()) {
            *current = Some(Connection::new(server)?);
        }
        Ok(current
            .as_ref()
            .expect("connection just initialized")
            .clone())
    }
}
impl Drop for Router {
    fn drop(&mut self) {
        if let Some(connection) = self.0.get_mut() {
            connection.close();
        }
    }
}

struct CallbackData<T> {
    serial: u64,
    found: Option<u32>,
    result: Option<Result<T, RouteError>>,
    wake: Rc<Wake>,
}
struct Request<T> {
    // Drop cancels this operation before `data` is released. The connection also
    // outlives every native operation, including when the Router is dropped.
    operation: Option<NonNull<pa_operation>>,
    data: Box<CallbackData<T>>,
    connection: Rc<Connection>,
}
impl<T> Request<T> {
    fn new(connection: Rc<Connection>, serial: u64) -> Self {
        let wake = connection.state.watch();
        Self {
            operation: None,
            data: Box::new(CallbackData {
                serial,
                found: None,
                result: None,
                wake,
            }),
            connection,
        }
    }
}
impl<T: Unpin> Future for Request<T> {
    type Output = Result<T, RouteError>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.data.wake.register(cx);
        if self.connection.state.closed.get() {
            return Poll::Ready(Err(RouteError::Cancelled));
        }
        if let Some(result) = self.data.result.take() {
            return Poll::Ready(result);
        }
        if self.connection.failed() {
            return Poll::Ready(Err(native_error(self.connection.context.as_ptr())));
        }
        Poll::Pending
    }
}
impl<T> Drop for Request<T> {
    fn drop(&mut self) {
        if let Some(operation) = self.operation.take() {
            // PulseAudio guarantees cancelled operations do not invoke their
            // completion callback. We still own and must release our reference.
            unsafe {
                pa_operation_cancel(operation.as_ptr());
                pa_operation_unref(operation.as_ptr());
            }
        }
    }
}
fn list_item(
    data: &mut CallbackData<u32>,
    context: *mut pa_context,
    eol: i32,
    info: Option<(u32, *mut pa_proplist)>,
) {
    if eol != 0 {
        data.result = Some(if eol < 0 {
            Err(native_error(context))
        } else {
            data.found.ok_or(RouteError::StreamNotFound)
        });
        data.wake.notify();
    } else if let Some((index, properties)) = info
        && !properties.is_null()
    {
        // Match the immutable PipeWire serial rather than a reusable global ID.
        let serial = unsafe { pa_proplist_gets(properties, c"object.serial".as_ptr()) };
        if !serial.is_null()
            && unsafe { CStr::from_ptr(serial) }
                .to_str()
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                == Some(data.serial)
        {
            data.found = Some(index);
        }
    }
}
extern "C" fn sink_info(
    context: *mut pa_context,
    info: *const pa_sink_input_info,
    eol: i32,
    data: *mut c_void,
) {
    // SAFETY: Request owns callback data; native info is borrowed for this call.
    unsafe {
        list_item(
            &mut *data.cast::<CallbackData<u32>>(),
            context,
            eol,
            info.as_ref().map(|i| (i.index, i.proplist)),
        );
    }
}
extern "C" fn source_info(
    context: *mut pa_context,
    info: *const pa_source_output_info,
    eol: i32,
    data: *mut c_void,
) {
    unsafe {
        list_item(
            &mut *data.cast::<CallbackData<u32>>(),
            context,
            eol,
            info.as_ref().map(|i| (i.index, i.proplist)),
        );
    }
}
extern "C" fn moved(context: *mut pa_context, success: i32, data: *mut c_void) {
    let data = unsafe { &mut *data.cast::<CallbackData<()>>() };
    data.result = Some(if success != 0 {
        Ok(())
    } else {
        Err(native_error(context))
    });
    data.wake.notify();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(dead_code)]
    mod fixture {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/common/audio.rs"
        ));
    }

    #[test]
    fn missing_stream_and_rejected_move_return_native_results() {
        let mut daemon = fixture::Daemon::new();
        daemon.start_pulse();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let connection = Connection::new(Some(&daemon.pulse_server())).unwrap();
                context.block_on(async {
                    assert_eq!(
                        connection.stream_index(u64::MAX, false).await,
                        Err(RouteError::StreamNotFound)
                    );
                    assert_eq!(
                        connection.stream_index(u64::MAX, true).await,
                        Err(RouteError::StreamNotFound)
                    );
                    assert!(matches!(
                        connection
                            .move_stream(u32::MAX, "missing-sink", false)
                            .await,
                        Err(RouteError::Failed(_))
                    ));
                    assert!(matches!(
                        connection
                            .move_stream(u32::MAX, "missing-source", true)
                            .await,
                        Err(RouteError::Failed(_))
                    ));
                });
            })
            .unwrap();
    }

    #[test]
    fn serial_matching_rejects_malformed_and_reused_identifiers() {
        let properties = unsafe { pa_proplist_new() };
        assert!(!properties.is_null());
        let mut data = CallbackData {
            serial: u64::MAX,
            found: None,
            result: None,
            wake: Rc::default(),
        };
        for value in [c"18446744073709551615x", c"-1", c"", c"42"] {
            unsafe {
                pa_proplist_sets(properties, c"object.serial".as_ptr(), value.as_ptr());
            }
            list_item(&mut data, std::ptr::null_mut(), 0, Some((17, properties)));
        }
        list_item(&mut data, std::ptr::null_mut(), 0, None);
        list_item(
            &mut data,
            std::ptr::null_mut(),
            0,
            Some((17, std::ptr::null_mut())),
        );
        list_item(&mut data, std::ptr::null_mut(), 1, None);
        assert_eq!(data.result, Some(Err(RouteError::StreamNotFound)));
        unsafe {
            pa_proplist_sets(
                properties,
                c"object.serial".as_ptr(),
                c"18446744073709551615".as_ptr(),
            );
        }
        list_item(&mut data, std::ptr::null_mut(), 0, Some((23, properties)));
        list_item(&mut data, std::ptr::null_mut(), 1, None);
        assert_eq!(data.result, Some(Ok(23)));
        unsafe {
            pa_proplist_free(properties);
        }

        let state = AudioState {
            available: true,
            nodes: vec![super::super::AudioNode {
                id: 4,
                serial: 12,
                kind: NodeKind::Sink,
                name: "same-name".into(),
                description: String::new(),
                nickname: String::new(),
                application: String::new(),
                media: String::new(),
                state: super::super::NodeState::Idle,
                volume: None,
            }],
            ..AudioState::default()
        };
        let endpoint = Endpoint::from_state(&state, 4).unwrap();
        let mut reused = state.clone();
        reused.nodes[0].serial += 1;
        assert!(!endpoint.present(&reused));
        assert!(endpoint.present(&state));
    }
}
