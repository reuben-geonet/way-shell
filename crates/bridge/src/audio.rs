//! Temporary stable C records and GObject signals for the existing audio widgets.
use glib::{ffi, prelude::*, subclass::prelude::*, translate::*};
use std::{
    cell::{RefCell, UnsafeCell},
    collections::BTreeMap,
    ffi::{CStr, CString, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::OnceLock,
};
use way_shell::services::audio::{
    AudioLink, AudioNode, AudioService, AudioState, Direction, NodeKind, NodeState,
};

#[repr(C)]
#[derive(Default)]
struct DeviceRecord {
    kind: i32,
    id: u32,
    ports: [i32; 12],
    name: *const c_char,
    proper_name: *const c_char,
    media_class: *const c_char,
    nickname: *const c_char,
    volume: f64,
    mute: i32,
    active: i32,
    step: f64,
    base: f64,
    state: i32,
    last_volume: f64,
}
#[repr(C)]
#[derive(Default)]
struct StreamRecord {
    kind: i32,
    id: u32,
    ports: [i32; 12],
    name: *const c_char,
    application: *const c_char,
    media_class: *const c_char,
    media: *const c_char,
    volume: f64,
    mute: i32,
    active: i32,
    step: f64,
    base: f64,
    state: i32,
}
#[repr(C)]
#[derive(Default)]
struct LinkRecord {
    kind: i32,
    id: u32,
    input_node: u32,
    output_node: u32,
    input_port: u32,
    output_port: u32,
}
// C holds borrowed addresses across callbacks. UnsafeCell permits its temporary
// mutable record ABI; Rust updates occur only on the owning GLib main thread.
enum Record {
    Device(Box<UnsafeCell<DeviceRecord>>, [Option<CString>; 4]),
    Stream(Box<UnsafeCell<StreamRecord>>, [Option<CString>; 4]),
    Link(Box<UnsafeCell<LinkRecord>>),
}
fn strings(values: [&str; 4]) -> [Option<CString>; 4] {
    values.map(|v| {
        (!v.is_empty()).then(|| CString::new(v).expect("native PipeWire strings exclude NUL"))
    })
}
fn pointer(string: &Option<CString>) -> *const c_char {
    string.as_ref().map_or(std::ptr::null(), |s| s.as_ptr())
}
fn kind(kind: NodeKind) -> i32 {
    match kind {
        NodeKind::Sink => 1,
        NodeKind::Source => 2,
        NodeKind::InputStream => 3,
        NodeKind::OutputStream => 4,
    }
}
fn state(state: NodeState) -> i32 {
    match state {
        NodeState::Error => -1,
        NodeState::Creating => 0,
        NodeState::Suspended => 1,
        NodeState::Idle => 2,
        NodeState::Running => 3,
        NodeState::Unknown => -1,
    }
}
impl Record {
    fn pointer(&self) -> *mut c_void {
        match self {
            Self::Device(r, _) => r.get().cast(),
            Self::Stream(r, _) => r.get().cast(),
            Self::Link(r) => r.get().cast(),
        }
    }
    fn kind(&self) -> i32 {
        match self {
            Self::Device(r, _) => unsafe { (*r.get()).kind },
            Self::Stream(r, _) => unsafe { (*r.get()).kind },
            Self::Link(_) => 5,
        }
    }
    fn node(node: &AudioNode) -> Self {
        if matches!(node.kind, NodeKind::Sink | NodeKind::Source) {
            Self::Device(Box::default(), Default::default())
        } else {
            Self::Stream(Box::default(), Default::default())
        }
    }
    fn update(&mut self, node: &AudioNode, snapshot: &AudioState) {
        let mut ports = [-1; 12];
        let direction = match node.kind {
            NodeKind::Sink | NodeKind::InputStream => Direction::Input,
            NodeKind::Source | NodeKind::OutputStream => Direction::Output,
        };
        for port in snapshot
            .ports
            .iter()
            .filter(|p| p.node == node.id && p.direction == direction && !p.monitor)
        {
            if let Some(index) = port
                .channel
                .as_deref()
                .and_then(way_shell_core::audio::channel_index)
            {
                ports[index] = i32::try_from(port.id).unwrap_or(-1);
            }
        }
        let (volume, mute, step, base) = node.volume.as_ref().map_or((0.0, 1, 0.0, 1.0), |v| {
            (v.volume, i32::from(v.mute), v.step, v.base)
        });
        match self {
            Self::Device(record, names) => {
                *names = strings([
                    &node.description,
                    &node.name,
                    node.kind.media_class(),
                    &node.nickname,
                ]);
                let record = unsafe { &mut *record.get() };
                let last_volume = record.last_volume;
                *record = DeviceRecord {
                    kind: kind(node.kind),
                    id: node.id,
                    ports,
                    name: pointer(&names[0]),
                    proper_name: pointer(&names[1]),
                    media_class: pointer(&names[2]),
                    nickname: pointer(&names[3]),
                    volume,
                    mute,
                    active: i32::from(node.state == NodeState::Running),
                    step,
                    base,
                    state: state(node.state),
                    last_volume,
                };
            }
            Self::Stream(record, names) => {
                *names = strings([
                    &node.description,
                    &node.application,
                    node.kind.media_class(),
                    &node.media,
                ]);
                let record = unsafe { &mut *record.get() };
                *record = StreamRecord {
                    kind: kind(node.kind),
                    id: node.id,
                    ports,
                    name: pointer(&names[0]),
                    application: pointer(&names[1]),
                    media_class: pointer(&names[2]),
                    media: pointer(&names[3]),
                    volume,
                    mute,
                    active: i32::from(node.state == NodeState::Running),
                    step,
                    base,
                    state: state(node.state),
                };
            }
            Self::Link(_) => unreachable!("record kind checked before updating"),
        }
    }
    fn link(link: &AudioLink) -> Self {
        Self::Link(Box::new(UnsafeCell::new(LinkRecord {
            kind: 5,
            id: link.id,
            input_node: link.input_node,
            input_port: link.input_port,
            output_node: link.output_node,
            output_port: link.output_port,
        })))
    }
}
struct Tables {
    arrays: [*mut ffi::GPtrArray; 4],
    database: *mut ffi::GHashTable,
}
impl Default for Tables {
    fn default() -> Self {
        unsafe {
            Self {
                arrays: std::array::from_fn(|_| ffi::g_ptr_array_new()),
                database: ffi::g_hash_table_new(
                    Some(ffi::g_direct_hash),
                    Some(ffi::g_direct_equal),
                ),
            }
        }
    }
}
impl Drop for Tables {
    fn drop(&mut self) {
        unsafe {
            for array in self.arrays {
                ffi::g_ptr_array_unref(array);
            }
            ffi::g_hash_table_unref(self.database);
        }
    }
}
impl Tables {
    fn fill(&self, records: &BTreeMap<u32, Record>) {
        unsafe {
            for array in self.arrays {
                ffi::g_ptr_array_set_size(array, 0);
            }
            ffi::g_hash_table_remove_all(self.database);
            for (&id, record) in records {
                let index = match record.kind() {
                    1 => 0,
                    2 => 1,
                    3 | 4 => 2,
                    _ => 3,
                };
                ffi::g_ptr_array_add(self.arrays[index], record.pointer());
                ffi::g_hash_table_insert(
                    self.database,
                    id as usize as *mut c_void,
                    record.pointer(),
                );
            }
        }
    }
}
mod imp {
    use super::*;
    #[derive(Default)]
    pub struct AudioAdapter {
        pub service: RefCell<Option<AudioService>>,
        pub handler: RefCell<Option<glib::SignalHandlerId>>,
        pub state: RefCell<AudioState>,
        pub(super) records: RefCell<BTreeMap<u32, Record>>,
        pub(super) tables: Tables,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for AudioAdapter {
        const NAME: &'static str = "WayShellAudioAdapter";
        type Type = super::AudioAdapter;
    }
    impl ObjectImpl for AudioAdapter {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                let mut signals: Vec<_> = [
                    "node-changed",
                    "database-changed",
                    "default-sink-changed",
                    "default-source-changed",
                    "default-sink-volume-changed",
                    "default-source-volume-changed",
                ]
                .into_iter()
                .map(|name| {
                    glib::subclass::Signal::builder(name)
                        .param_types([glib::Type::POINTER])
                        .build()
                })
                .collect();
                signals.push(
                    glib::subclass::Signal::builder("availability-changed")
                        .param_types([bool::static_type()])
                        .build(),
                );
                signals.push(
                    glib::subclass::Signal::builder("microphone-active")
                        .param_types([bool::static_type()])
                        .build(),
                );
                signals
            })
        }
        fn dispose(&self) {
            let service = self.service.borrow_mut().take();
            if let Some(service) = service {
                if let Some(handler) = self.handler.borrow_mut().take() {
                    service.disconnect(handler);
                }
                service.stop();
            }
        }
    }
}
glib::wrapper! { pub struct AudioAdapter(ObjectSubclass<imp::AudioAdapter>); }
impl AudioAdapter {
    fn new(service: AudioService) -> Self {
        let adapter: Self = glib::Object::new();
        let weak = adapter.downgrade();
        let handler = service.connect_local("changed", false, move |_| {
            if let Some(adapter) = weak.upgrade() {
                adapter.refresh();
            }
            None
        });
        adapter.imp().service.replace(Some(service));
        adapter.imp().handler.replace(Some(handler));
        adapter.refresh();
        adapter
    }
    fn refresh(&self) {
        let snapshot = self
            .imp()
            .service
            .borrow()
            .as_ref()
            .map(AudioService::state)
            .unwrap_or_default();
        self.apply(snapshot);
    }
    fn record(&self, id: Option<u32>) -> *mut c_void {
        id.and_then(|id| self.imp().records.borrow().get(&id).map(Record::pointer))
            .unwrap_or(std::ptr::null_mut())
    }
    fn default_node(&self, source: bool) -> *mut c_void {
        let state = self.imp().state.borrow();
        let id = if source {
            state.default_source
        } else {
            state.default_sink
        };
        self.record(id.filter(|id| state.node(*id).is_some_and(|n| n.volume.is_some())))
    }
    fn apply(&self, snapshot: AudioState) {
        let previous = self.imp().state.replace(snapshot.clone());
        let mut removed = Vec::new();
        {
            let mut records = self.imp().records.borrow_mut();
            let old = std::mem::take(&mut *records);
            for (id, record) in old {
                if snapshot
                    .nodes
                    .iter()
                    .any(|n| n.id == id && kind(n.kind) == record.kind())
                    || snapshot
                        .links
                        .iter()
                        .any(|l| l.id == id && record.kind() == 5)
                {
                    records.insert(id, record);
                } else {
                    removed.push(record);
                }
            }
            for node in &snapshot.nodes {
                records
                    .entry(node.id)
                    .or_insert_with(|| Record::node(node))
                    .update(node, &snapshot);
            }
            for link in &snapshot.links {
                if let Some(Record::Link(record)) = records.get_mut(&link.id) {
                    let replacement = match Record::link(link) {
                        Record::Link(r) => r.into_inner(),
                        _ => unreachable!(),
                    };
                    unsafe {
                        *record.get() = replacement;
                    }
                } else {
                    records.insert(link.id, Record::link(link));
                }
            }
            self.imp().tables.fill(&records);
        }
        // Keep removed records alive while C observers clear cached pointers.
        for (source, old_id, new_id, changed, volume_changed) in [
            (
                false,
                previous.default_sink,
                snapshot.default_sink,
                "default-sink-changed",
                "default-sink-volume-changed",
            ),
            (
                true,
                previous.default_source,
                snapshot.default_source,
                "default-source-changed",
                "default-source-volume-changed",
            ),
        ] {
            let old = old_id.and_then(|id| previous.node(id));
            let new = new_id.and_then(|id| snapshot.node(id));
            if old != new {
                let pointer = self.default_node(source);
                self.emit_by_name::<()>(changed, &[&pointer]);
                if !pointer.is_null()
                    && old_id == new_id
                    && old.zip(new).is_some_and(|(old, new)| {
                        old.volume
                            .as_ref()
                            .zip(new.volume.as_ref())
                            .is_some_and(|(old, new)| {
                                old.volume != new.volume || old.mute != new.mute
                            })
                    })
                {
                    self.emit_by_name::<()>(volume_changed, &[&pointer]);
                }
            }
        }
        if snapshot.microphone_active() != previous.microphone_active() {
            self.emit_by_name::<()>("microphone-active", &[&snapshot.microphone_active()]);
        }
        let inventory_changed = !removed.is_empty()
            || snapshot.nodes.len() != previous.nodes.len()
            || snapshot.links != previous.links
            || snapshot.ports != previous.ports
            || snapshot.nodes.iter().any(|n| {
                previous.node(n.id).is_none_or(|p| {
                    p.name != n.name
                        || p.description != n.description
                        || p.nickname != n.nickname
                        || p.application != n.application
                        || p.media != n.media
                })
            });
        if inventory_changed {
            self.emit_by_name::<()>(
                "database-changed",
                &[&(self.imp().tables.database.cast::<c_void>())],
            );
        }
        for node in &snapshot.nodes {
            if previous.node(node.id) != Some(node) {
                self.emit_by_name::<()>("node-changed", &[&self.record(Some(node.id))]);
            }
        }
        if snapshot.available != previous.available {
            self.emit_by_name::<()>("availability-changed", &[&snapshot.available]);
        }
        drop(removed);
    }
}
thread_local! {static GLOBAL:RefCell<Option<AudioAdapter>>=const{RefCell::new(None)};}
pub fn shutdown() {
    GLOBAL.with(|global| {
        if let Some(adapter) = global.borrow().as_ref()
            && let Some(service) = adapter.imp().service.borrow().clone()
        {
            service.stop();
        }
    });
}
#[unsafe(no_mangle)]
pub extern "C" fn wire_plumber_service_get_type() -> ffi::GType {
    catch_unwind(|| AudioAdapter::static_type().into_glib()).unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "C" fn way_shell_audio_global_init() -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        GLOBAL.with(|global| {
            if global.borrow().is_none() {
                global.replace(Some(AudioAdapter::new(AudioService::new())));
            }
        });
        0
    }))
    .unwrap_or(-1)
}
#[unsafe(no_mangle)]
pub extern "C" fn wire_plumber_service_get_global() -> *mut glib::gobject_ffi::GObject {
    catch_unwind(AssertUnwindSafe(|| {
        GLOBAL.with(|global| {
            global
                .borrow()
                .as_ref()
                .map_or(std::ptr::null_mut(), |a| a.as_ptr().cast())
        })
    }))
    .unwrap_or(std::ptr::null_mut())
}
unsafe fn borrowed(pointer: *mut glib::gobject_ffi::GObject) -> Option<AudioAdapter> {
    if pointer.is_null() {
        return None;
    }
    let object: Borrowed<glib::Object> = unsafe { from_glib_borrow(pointer) };
    object.downcast_ref::<AudioAdapter>().cloned()
}
macro_rules! getter {
    ($name:ident, $output:ty, $fallback:expr, $body:expr) => {
        /// # Safety
        /// The pointer is NULL or a live audio adapter borrowed for this call.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(pointer: *mut glib::gobject_ffi::GObject) -> $output {
            catch_unwind(AssertUnwindSafe(|| {
                unsafe { borrowed(pointer) }.map_or($fallback, $body)
            }))
            .unwrap_or($fallback)
        }
    };
}
getter!(
    wire_plumber_service_get_default_sink,
    *mut c_void,
    std::ptr::null_mut(),
    |a| a.default_node(false)
);
getter!(
    wire_plumber_service_get_default_source,
    *mut c_void,
    std::ptr::null_mut(),
    |a| a.default_node(true)
);
getter!(
    wire_plumber_service_get_sinks,
    *mut ffi::GPtrArray,
    std::ptr::null_mut(),
    |a| a.imp().tables.arrays[0]
);
getter!(
    wire_plumber_service_get_sources,
    *mut ffi::GPtrArray,
    std::ptr::null_mut(),
    |a| a.imp().tables.arrays[1]
);
getter!(
    wire_plumber_service_get_streams,
    *mut ffi::GPtrArray,
    std::ptr::null_mut(),
    |a| a.imp().tables.arrays[2]
);
getter!(
    wire_plumber_service_get_links,
    *mut ffi::GPtrArray,
    std::ptr::null_mut(),
    |a| a.imp().tables.arrays[3]
);
getter!(
    wire_plumber_service_get_db,
    *mut ffi::GHashTable,
    std::ptr::null_mut(),
    |a| a.imp().tables.database
);
getter!(wire_plumber_service_microphone_active, i32, 0, |a| {
    i32::from(a.imp().state.borrow().microphone_active())
});
getter!(
    way_shell_audio_ref_mixer,
    *mut glib::gobject_ffi::GObject,
    std::ptr::null_mut(),
    |a| a
        .imp()
        .service
        .borrow()
        .as_ref()
        .and_then(AudioService::compatibility_mixer)
        .map_or(std::ptr::null_mut(), |m| m.into_glib_ptr())
);
/// # Safety
/// A non-NULL media class is a readable NUL-terminated string for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wire_plumber_service_media_class_to_type(class: *const c_char) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if class.is_null() {
            return 0;
        }
        match unsafe { CStr::from_ptr(class) }.to_bytes() {
            b"Audio/Sink" => 1,
            b"Audio/Source" => 2,
            b"Stream/Input/Audio" => 3,
            b"Stream/Output/Audio" => 4,
            _ => 0,
        }
    }))
    .unwrap_or(0)
}

getter!(way_shell_audio_available, i32, 0, |a| i32::from(
    a.imp().state.borrow().available
));

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};
    use way_shell::services::audio::{AudioPort, Volume};
    fn source() -> AudioNode {
        AudioNode {
            id: 42,
            kind: NodeKind::Source,
            name: "node.name".into(),
            description: "Capture".into(),
            nickname: "Mic".into(),
            application: String::new(),
            media: String::new(),
            state: NodeState::Running,
            volume: Some(Volume {
                volume: 0.5,
                mute: false,
                step: 0.01,
                base: 1.0,
                channels: Vec::new(),
            }),
        }
    }
    #[test]
    fn stable_c_records_removal_defaults_and_channel_mapping() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let adapter: AudioAdapter = glib::Object::new();
                let source = source();
                let mut snapshot = AudioState {
                    available: true,
                    nodes: vec![source],
                    default_source: Some(42),
                    ports: vec![AudioPort {
                        id: 51,
                        node: 42,
                        index: 0,
                        direction: Direction::Output,
                        channel: Some("FL".into()),
                        monitor: false,
                    }],
                    ..AudioState::default()
                };
                adapter.apply(snapshot.clone());
                let pointer = adapter.default_node(true).cast::<DeviceRecord>();
                assert!(!pointer.is_null());
                unsafe {
                    assert_eq!((*pointer).kind, 2);
                    assert_eq!((*pointer).ports[2], 51);
                    assert_eq!((*pointer).ports[0], -1);
                    assert_eq!((*pointer).volume, 0.5);
                    assert_eq!((*pointer).step, 0.01);
                    assert_eq!(CStr::from_ptr((*pointer).name).to_bytes(), b"Capture");
                }
                let changes = Rc::new(Cell::new(0));
                let changed = changes.clone();
                adapter.connect_local("default-source-volume-changed", false, move |_| {
                    changed.set(changed.get() + 1);
                    None
                });
                snapshot.nodes[0].volume.as_mut().unwrap().volume = 0.7;
                adapter.apply(snapshot.clone());
                assert_eq!(adapter.default_node(true).cast::<DeviceRecord>(), pointer);
                assert_eq!(changes.get(), 1);
                // C mute control currently keeps its restore value in this borrowed record.
                unsafe {
                    (*pointer).last_volume = 0.7;
                }
                snapshot.nodes[0].description = "Renamed microphone".into();
                adapter.apply(snapshot.clone());
                unsafe {
                    assert_eq!((*pointer).last_volume, 0.7);
                    assert_eq!(
                        CStr::from_ptr((*pointer).name).to_bytes(),
                        b"Renamed microphone"
                    );
                }
                let cleared = Rc::new(Cell::new(false));
                let observed = cleared.clone();
                let removed_handler =
                    adapter.connect_local("default-source-changed", false, move |values| {
                        if values[1].get::<*mut c_void>().unwrap().is_null() {
                            unsafe {
                                assert_eq!((*pointer).id, 42);
                            }
                            observed.set(true);
                        }
                        None
                    });
                adapter.apply(AudioState::default());
                assert!(cleared.get());
                assert!(adapter.default_node(true).is_null());
                unsafe {
                    assert_eq!((*adapter.imp().tables.arrays[1]).len, 0);
                    assert_eq!(ffi::g_hash_table_size(adapter.imp().tables.database), 0);
                }
                assert_eq!(changes.get(), 1, "removal must not trigger the volume OSD");
                adapter.disconnect(removed_handler);
                // Missing volume data keeps the device inventoried but disables default controls.
                snapshot.nodes[0].volume = None;
                adapter.apply(snapshot);
                assert!(adapter.default_node(true).is_null());
                unsafe {
                    assert_eq!((*adapter.imp().tables.arrays[1]).len, 1);
                }
            })
            .unwrap();
    }
    #[test]
    fn live_inventory_adapter_disconnect_and_recovery() {
        let mut daemon = crate::audio_fixture::Daemon::new();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let service = AudioService::with_remote(&daemon.remote());
                let weak_service = service.downgrade();
                let adapter = AudioAdapter::new(service);
                crate::audio_fixture::wait(&context, || adapter.imp().state.borrow().available);
                let pointer = adapter.as_ptr().cast();
                unsafe {
                    let sinks = wire_plumber_service_get_sinks(pointer);
                    assert_eq!((*sinks).len, 1);
                    let sink = (*(*sinks).pdata).cast::<DeviceRecord>();
                    assert_eq!(
                        CStr::from_ptr((*sink).proper_name).to_bytes(),
                        b"way-shell-test-sink"
                    );
                    assert_eq!(
                        ffi::g_hash_table_lookup(
                            wire_plumber_service_get_db(pointer),
                            (*sink).id as usize as *mut c_void
                        ),
                        sink.cast()
                    );
                }
                daemon.stop();
                crate::audio_fixture::wait(&context, || !adapter.imp().state.borrow().available);
                unsafe {
                    assert_eq!((*wire_plumber_service_get_sinks(pointer)).len, 0);
                    assert_eq!(
                        ffi::g_hash_table_size(wire_plumber_service_get_db(pointer)),
                        0
                    );
                }
                daemon.start();
                crate::audio_fixture::wait(&context, || adapter.imp().state.borrow().available);
                let weak = adapter.downgrade();
                drop(adapter);
                context.block_on(glib::timeout_future(std::time::Duration::from_millis(40)));
                assert!(weak.upgrade().is_none());
                assert!(weak_service.upgrade().is_none());
            })
            .unwrap();
    }
    #[test]
    fn record_layout_matches_the_temporary_c_header() {
        assert_eq!(std::mem::size_of::<DeviceRecord>(), 136);
        assert_eq!(std::mem::size_of::<StreamRecord>(), 128);
        assert_eq!(std::mem::size_of::<LinkRecord>(), 24);
        assert_eq!(std::mem::offset_of!(DeviceRecord, volume), 88);
        assert_eq!(std::mem::offset_of!(StreamRecord, volume), 88);
    }
}
