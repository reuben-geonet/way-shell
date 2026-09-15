//! Temporary notification records for the remaining C widgets.
use glib::{ffi, prelude::*, subclass::prelude::*, translate::*};
use std::{
    cell::{Cell, RefCell, UnsafeCell},
    collections::VecDeque,
    ffi::{CStr, CString, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::OnceLock,
};
use way_shell::services::notifications::{
    CloseReason, Notification, NotificationEvent, NotificationRequest, NotificationsService,
};
use way_shell_core::notifications::Origin;

#[cfg(test)]
use crate::media_fixture as fixture;

#[repr(C)]
#[derive(Default)]
struct ImageRecord {
    width: u32,
    height: u32,
    rowstride: u32,
    has_alpha: i32,
    bits_per_sample: u32,
    channels: u32,
    data: *mut c_char,
}

/// Layout of `Notification` in notifications_service.h. The unused raw hints
/// pointer stays null; parsed fields are owned by the permanent Rust service.
#[repr(C)]
#[derive(Default)]
pub struct NotificationRecord {
    app_name: *mut c_char,
    app_icon: *mut c_char,
    summary: *mut c_char,
    body: *mut c_char,
    actions: *mut *mut c_char,
    hints: *mut ffi::GVariant,
    category: *mut c_char,
    desktop_entry: *mut c_char,
    image_path: *mut c_char,
    img_data: ImageRecord,
    id: i32,
    replaces_id: u32,
    expire_timeout: i32,
    action_icons: i32,
    resident: i32,
    transient: i32,
    urgency: u8,
    is_internal: i32,
    created_on: *mut ffi::GDateTime,
}

/// Preserve the legacy C record's mutable string contract while the permanent
/// domain snapshot remains immutable. Each byte has explicit interior
/// mutability; newer widgets copy text before trimming it.
struct MutableText(Box<[UnsafeCell<u8>]>);
impl MutableText {
    fn new(value: &str) -> Self {
        let bytes = CString::new(value)
            .unwrap_or_default()
            .into_bytes_with_nul();
        Self(bytes.into_iter().map(UnsafeCell::new).collect())
    }
    fn pointer(&self) -> *mut c_char {
        self.0[0].get().cast()
    }
}

struct Record {
    value: Box<UnsafeCell<NotificationRecord>>,
    _strings: [Option<MutableText>; 7],
    _actions: Vec<MutableText>,
    _action_pointers: Box<[*mut c_char]>,
    _pixels: Box<[UnsafeCell<u8>]>,
    _created_on: glib::DateTime,
    id: u32,
}
impl Record {
    fn new(notification: &Notification) -> Self {
        let request = &notification.request;
        let hints = &request.hints;
        let strings = [
            Some(request.app_name.as_str()),
            Some(request.app_icon.as_str()),
            Some(request.summary.as_str()),
            Some(request.body.as_str()),
            hints.category.as_deref(),
            hints.desktop_entry.as_deref(),
            hints.image_path.as_deref(),
        ]
        .map(|value| value.map(MutableText::new));
        let [
            app_name,
            app_icon,
            summary,
            body,
            category,
            desktop_entry,
            image_path,
        ] = strings.each_ref().map(|value| {
            value
                .as_ref()
                .map_or(std::ptr::null_mut(), MutableText::pointer)
        });
        let actions = request
            .actions
            .iter()
            .flat_map(|action| {
                [
                    MutableText::new(&action.key),
                    MutableText::new(&action.label),
                ]
            })
            .collect::<Vec<_>>();
        let mut action_pointers = actions
            .iter()
            .map(MutableText::pointer)
            .chain(std::iter::once(std::ptr::null_mut()))
            .collect::<Box<[_]>>();
        let pixels = hints.image.as_ref().map_or_else(
            || Box::new([]) as Box<[UnsafeCell<u8>]>,
            |image| image.data().iter().copied().map(UnsafeCell::new).collect(),
        );
        let img_data = hints
            .image
            .as_ref()
            .map_or_else(ImageRecord::default, |image| ImageRecord {
                width: image.width(),
                height: image.height(),
                rowstride: image.rowstride(),
                has_alpha: i32::from(image.has_alpha()),
                bits_per_sample: image.bits_per_sample(),
                channels: image.channels(),
                data: pixels
                    .first()
                    .map_or(std::ptr::null_mut(), |byte| byte.get().cast()),
            });
        let created_on =
            glib::DateTime::from_unix_local(notification.created_on_us.div_euclid(1_000_000))
                .and_then(|date| {
                    date.add(glib::TimeSpan::from_microseconds(
                        notification.created_on_us.rem_euclid(1_000_000),
                    ))
                })
                // The service supplies real wall-clock time. Still keep the C
                // widget's required date non-null for manually built bad snapshots.
                .unwrap_or_else(|_| {
                    glib::DateTime::from_unix_local(0).expect("The Unix epoch is representable")
                });
        let value = NotificationRecord {
            app_name,
            app_icon,
            summary,
            body,
            actions: action_pointers.as_mut_ptr(),
            category,
            desktop_entry,
            image_path,
            img_data,
            id: notification.id as i32,
            replaces_id: request.replaces_id,
            expire_timeout: request.expire_timeout,
            action_icons: i32::from(hints.action_icons),
            resident: i32::from(hints.resident),
            transient: i32::from(hints.transient),
            urgency: hints.urgency,
            is_internal: i32::from(request.origin == Origin::Internal),
            created_on: created_on.to_glib_none().0,
            ..Default::default()
        };
        Self {
            value: Box::new(UnsafeCell::new(value)),
            _strings: strings,
            _actions: actions,
            _action_pointers: action_pointers,
            _pixels: pixels,
            _created_on: created_on,
            id: notification.id,
        }
    }

    fn pointer(&self) -> *mut c_void {
        self.value.get().cast()
    }
}

glib::wrapper! {
    pub struct NotificationArray(Shared<ffi::GPtrArray>);
    match fn {
        ref => |pointer| unsafe { ffi::g_ptr_array_ref(pointer) },
        unref => |pointer| unsafe { ffi::g_ptr_array_unref(pointer) },
        type_ => || ffi::g_ptr_array_get_type(),
    }
}
impl Default for NotificationArray {
    fn default() -> Self {
        unsafe { from_glib_full(ffi::g_ptr_array_new()) }
    }
}
impl NotificationArray {
    fn pointer(&self) -> *mut ffi::GPtrArray {
        self.to_glib_none().0
    }
    fn fill(&self, records: &[Record]) {
        unsafe {
            ffi::g_ptr_array_set_size(self.pointer(), 0);
            for record in records {
                ffi::g_ptr_array_add(self.pointer(), record.pointer());
            }
        }
    }
}

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct NotificationsAdapter {
        pub service: RefCell<Option<NotificationsService>>,
        pub handler: RefCell<Option<glib::SignalHandlerId>>,
        pub pending: RefCell<VecDeque<NotificationEvent>>,
        pub applying: Cell<bool>,
        pub(super) records: RefCell<Vec<Record>>,
        pub notifications: NotificationArray,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for NotificationsAdapter {
        const NAME: &'static str = "WayShellNotificationsAdapter";
        type Type = super::NotificationsAdapter;
    }
    impl ObjectImpl for NotificationsAdapter {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                let mut signals = [
                    "notification-added",
                    "notification-closed",
                    "notification-replaced",
                ]
                .into_iter()
                .map(|name| {
                    glib::subclass::Signal::builder(name)
                        .param_types([
                            NotificationArray::static_type(),
                            u32::static_type(),
                            u32::static_type(),
                        ])
                        .build()
                })
                .collect::<Vec<_>>();
                signals.push(
                    glib::subclass::Signal::builder("notification-changed")
                        .param_types([NotificationArray::static_type()])
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
            self.pending.borrow_mut().clear();
        }
    }
}
glib::wrapper! { pub struct NotificationsAdapter(ObjectSubclass<imp::NotificationsAdapter>); }

impl NotificationsAdapter {
    fn new(service: NotificationsService) -> Self {
        let adapter: Self = glib::Object::new();
        let weak = adapter.downgrade();
        let handler = service.connect_local("event", false, move |values| {
            if let Some(adapter) = weak.upgrade()
                && let Ok(event) = values[1].get::<NotificationEvent>()
            {
                adapter.apply(event);
            }
            None
        });
        let records = service
            .state()
            .notifications
            .iter()
            .map(Record::new)
            .collect::<Vec<_>>();
        adapter.imp().notifications.fill(&records);
        adapter.imp().records.replace(records);
        adapter.imp().service.replace(Some(service));
        adapter.imp().handler.replace(Some(handler));
        adapter
    }

    fn apply(&self, event: NotificationEvent) {
        self.imp().pending.borrow_mut().push_back(event);
        if self.imp().applying.replace(true) {
            return;
        }
        // Incremental events must not be coalesced. Reentrant C observers may
        // close, replace, or add records while every current observer still
        // borrows this event's array and strings.
        let mut next = self.imp().pending.borrow_mut().pop_front();
        while let Some(event) = next {
            self.apply_one(event);
            next = self.imp().pending.borrow_mut().pop_front();
        }
        self.imp().applying.set(false);
    }

    fn apply_one(&self, event: NotificationEvent) {
        match event {
            NotificationEvent::Added {
                notification,
                index,
            } => {
                let id = notification.id;
                let index = {
                    let mut records = self.imp().records.borrow_mut();
                    let index = index.min(records.len());
                    records.insert(index, Record::new(&notification));
                    self.imp().notifications.fill(&records);
                    index as u32
                };
                self.emit_by_name::<()>(
                    "notification-added",
                    &[&self.imp().notifications, &id, &index],
                );
            }
            NotificationEvent::Replaced {
                notification,
                previous: _,
                index: _,
            } => {
                let id = notification.id;
                let replaced = {
                    let mut records = self.imp().records.borrow_mut();
                    records
                        .iter()
                        .position(|record| record.id == id)
                        .map(|index| {
                            let old =
                                std::mem::replace(&mut records[index], Record::new(&notification));
                            self.imp().notifications.fill(&records);
                            (old, index as u32)
                        })
                };
                if let Some((_old, index)) = replaced {
                    // Array now contains the replacement at the original
                    // position; old strings stay alive through every callback.
                    // Replacement is not a close and emits no closed signal.
                    self.emit_by_name::<()>(
                        "notification-replaced",
                        &[&self.imp().notifications, &id, &index],
                    );
                }
            }
            NotificationEvent::Closed {
                notification,
                index: _,
                reason: _,
            } => {
                let id = notification.id;
                let index = self
                    .imp()
                    .records
                    .borrow()
                    .iter()
                    .position(|record| record.id == id);
                if let Some(index) = index {
                    let signal_index = index as u32;
                    // C reads notifications[index] in this signal. Remove the
                    // record only after observers return, preserving its array
                    // membership and backing storage until then.
                    self.emit_by_name::<()>(
                        "notification-closed",
                        &[&self.imp().notifications, &id, &signal_index],
                    );
                    let mut records = self.imp().records.borrow_mut();
                    records.remove(index);
                    self.imp().notifications.fill(&records);
                }
            }
        }
        self.emit_by_name::<()>("notification-changed", &[&self.imp().notifications]);
    }
}

thread_local! { static GLOBAL: RefCell<Option<NotificationsAdapter>> = const { RefCell::new(None) }; }

pub fn shutdown() {
    // C widgets borrow the global without retaining it. Keep its records and
    // array alive until thread exit, and release borrows before service signals.
    let adapter = GLOBAL.with(|global| global.borrow().clone());
    if let Some(adapter) = adapter {
        let service = adapter.imp().service.borrow().clone();
        if let Some(service) = service {
            service.stop();
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn notifications_service_get_type() -> ffi::GType {
    catch_unwind(|| NotificationsAdapter::static_type().into_glib()).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn notifications_service_global_init() -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        GLOBAL.with(|global| {
            if global.borrow().is_none() {
                global.replace(Some(NotificationsAdapter::new(NotificationsService::new())));
            }
            0
        })
    }))
    .unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub extern "C" fn notifications_service_get_global() -> *mut glib::gobject_ffi::GObject {
    catch_unwind(AssertUnwindSafe(|| {
        let missing = GLOBAL.with(|global| global.borrow().is_none());
        if missing && notifications_service_global_init() != 0 {
            return std::ptr::null_mut();
        }
        GLOBAL.with(|global| {
            global
                .borrow()
                .as_ref()
                .map_or(std::ptr::null_mut(), |adapter| adapter.as_ptr().cast())
        })
    }))
    .unwrap_or(std::ptr::null_mut())
}

unsafe fn borrowed(pointer: *mut glib::gobject_ffi::GObject) -> Option<NotificationsAdapter> {
    if pointer.is_null() {
        return None;
    }
    let object: Borrowed<glib::Object> = unsafe { from_glib_borrow(pointer) };
    object.downcast_ref::<NotificationsAdapter>().cloned()
}

/// # Safety
/// `pointer` is null or a live borrowed adapter on the owning application
/// thread. The returned array and records remain adapter-owned. C may trim
/// string contents in place within their existing allocation, but must not
/// change pointers or free records. Membership changes at notification signals.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn notifications_service_get_notifications(
    pointer: *mut glib::gobject_ffi::GObject,
) -> *mut ffi::GPtrArray {
    catch_unwind(AssertUnwindSafe(|| {
        unsafe { borrowed(pointer) }.map_or(std::ptr::null_mut(), |adapter| {
            adapter.imp().notifications.pointer()
        })
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// `pointer` is null or a live borrowed adapter on the owning application thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn notifications_service_closed_notification(
    pointer: *mut glib::gobject_ffi::GObject,
    id: u32,
    reason: u32,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(adapter) = (unsafe { borrowed(pointer) }) else {
            return -1;
        };
        let service = adapter.imp().service.borrow().clone();
        let reason = match reason {
            1 => CloseReason::Expired,
            2 => CloseReason::Dismissed,
            3 => CloseReason::Requested,
            _ => CloseReason::Undefined,
        };
        if service.is_some_and(|service| service.close(id, reason)) {
            0
        } else {
            -1
        }
    }))
    .unwrap_or(-1)
}

/// # Safety
/// `pointer` is null or a live adapter on the owning application thread, and
/// `action_key` is null or a readable NUL-terminated string for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn notifications_service_invoke_action(
    pointer: *mut glib::gobject_ffi::GObject,
    id: u32,
    action_key: *const c_char,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(adapter) = (unsafe { borrowed(pointer) }) else {
            return -1;
        };
        if action_key.is_null() {
            return -1;
        }
        let Ok(key) = (unsafe { CStr::from_ptr(action_key) }).to_str() else {
            return -1;
        };
        let service = adapter.imp().service.borrow().clone();
        if service.is_some_and(|service| service.invoke_action(id, key)) {
            0
        } else {
            -1
        }
    }))
    .unwrap_or(-1)
}

unsafe fn copy_text(pointer: *const c_char) -> String {
    if pointer.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
    }
}

/// # Safety
/// `pointer` is null or a live adapter. `notification` is null or a readable
/// `Notification` whose app name, icon, summary and body strings are null or
/// NUL-terminated. Those fields are copied before any observer is notified.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn notifications_service_send_notification(
    pointer: *mut glib::gobject_ffi::GObject,
    notification: *const NotificationRecord,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(adapter) = (unsafe { borrowed(pointer) }) else {
            return;
        };
        let Some(notification) = (unsafe { notification.as_ref() }) else {
            return;
        };
        let request = NotificationRequest {
            app_name: unsafe { copy_text(notification.app_name) },
            app_icon: unsafe { copy_text(notification.app_icon) },
            summary: unsafe { copy_text(notification.summary) },
            body: unsafe { copy_text(notification.body) },
            expire_timeout: 0,
            origin: Origin::Internal,
            hints: way_shell_core::notifications::Hints {
                urgency: notification.urgency,
                ..Default::default()
            },
            ..Default::default()
        };
        let service = adapter.imp().service.borrow().clone();
        if let Some(service) = service
            && let Err(error) = service.send_internal(request)
        {
            glib::g_message!(
                "way-shell",
                "Could not publish internal notification: {error}"
            );
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        mem::{offset_of, size_of},
        rc::Rc,
    };
    use way_shell_core::notifications::{Action, Hints, Image};

    fn notification(id: u32, summary: &str) -> Notification {
        Notification {
            id,
            created_on_us: 1_700_000_000_123_456,
            request: NotificationRequest {
                app_name: "Fixture".into(),
                app_icon: "dialog-information-symbolic".into(),
                summary: summary.into(),
                body: "  Body\ntext  ".into(),
                actions: vec![Action {
                    key: "reply".into(),
                    label: "Reply".into(),
                }],
                hints: Hints {
                    category: Some("im.received".into()),
                    image: Some(Image::new(2, 2, 8, false, 8, 3, (0..14).collect()).unwrap()),
                    urgency: 2,
                    ..Default::default()
                },
                ..Default::default()
            },
        }
    }

    fn added(notification: Notification, index: usize) -> NotificationEvent {
        NotificationEvent::Added {
            notification,
            index,
        }
    }

    fn closed(notification: Notification, index: usize) -> NotificationEvent {
        NotificationEvent::Closed {
            notification,
            index,
            reason: CloseReason::Dismissed,
        }
    }

    fn array(adapter: &NotificationsAdapter) -> *mut ffi::GPtrArray {
        unsafe { notifications_service_get_notifications(adapter.as_ptr().cast()) }
    }

    fn record(adapter: &NotificationsAdapter, index: usize) -> *mut NotificationRecord {
        let array = array(adapter);
        unsafe {
            assert!(index < (*array).len as usize);
            (*(*array).pdata.add(index)).cast()
        }
    }

    #[test]
    fn c_record_layout_matches_the_x86_64_notification_header() {
        assert_eq!(size_of::<*mut c_void>(), 8);
        assert_eq!(size_of::<ImageRecord>(), 32);
        assert_eq!(offset_of!(ImageRecord, data), 24);
        assert_eq!(size_of::<NotificationRecord>(), 144);
        assert_eq!(offset_of!(NotificationRecord, actions), 32);
        assert_eq!(offset_of!(NotificationRecord, hints), 40);
        assert_eq!(offset_of!(NotificationRecord, image_path), 64);
        assert_eq!(offset_of!(NotificationRecord, img_data), 72);
        assert_eq!(offset_of!(NotificationRecord, id), 104);
        assert_eq!(offset_of!(NotificationRecord, urgency), 128);
        assert_eq!(offset_of!(NotificationRecord, is_internal), 132);
        assert_eq!(offset_of!(NotificationRecord, created_on), 136);
    }

    #[test]
    fn records_own_mutable_text_actions_pixels_and_creation_time() {
        let source = notification(u32::MAX, "  Title\nline  ");
        let record = Record::new(&source);
        let pointer = record.value.get();
        unsafe {
            ffi::g_strchug((*pointer).summary);
            ffi::g_strchomp((*pointer).summary);
            ffi::g_strdelimit((*pointer).summary, c"\n".as_ptr(), b' ' as c_char);
            ffi::g_strchug((*pointer).body);
            ffi::g_strchomp((*pointer).body);
            ffi::g_strdelimit((*pointer).body, c"\n".as_ptr(), b' ' as c_char);
            assert_eq!(CStr::from_ptr((*pointer).summary).to_bytes(), b"Title line");
            assert_eq!(CStr::from_ptr((*pointer).body).to_bytes(), b"Body text");
            assert_eq!((*pointer).id as u32, u32::MAX);
            assert!((*pointer).hints.is_null());
            assert_eq!(CStr::from_ptr(*(*pointer).actions).to_bytes(), b"reply");
            assert_eq!(
                CStr::from_ptr(*(*pointer).actions.add(1)).to_bytes(),
                b"Reply"
            );
            assert!((*(*pointer).actions.add(2)).is_null());
            assert_eq!(
                ffi::g_date_time_to_unix((*pointer).created_on),
                1_700_000_000
            );
            assert_eq!(
                ffi::g_date_time_get_microsecond((*pointer).created_on),
                123_456
            );
        }
        assert_eq!(source.request.summary, "  Title\nline  ");
        assert_eq!(source.request.body, "  Body\ntext  ");
        drop(source);
        unsafe {
            assert_eq!(
                std::slice::from_raw_parts((*pointer).img_data.data.cast::<u8>(), 14),
                (0..14).collect::<Vec<_>>()
            );
            assert_eq!(
                CStr::from_ptr((*pointer).category).to_bytes(),
                b"im.received"
            );
            assert_eq!(CStr::from_ptr((*pointer).summary).to_bytes(), b"Title line");
        }
    }

    #[test]
    fn closed_signals_keep_the_old_array_index_and_queue_nested_events() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let adapter: NotificationsAdapter = glib::Object::new();
                let first = notification(1, "First");
                let second = notification(2, "Second");
                adapter.apply(added(first.clone(), 0));
                adapter.apply(added(second.clone(), 1));
                let first_pointer = record(&adapter, 0);
                let log = Rc::new(RefCell::new(Vec::new()));
                let recorded = log.clone();
                let weak = adapter.downgrade();
                adapter.connect_local("notification-closed", false, move |values| {
                    let id = values[2].get::<u32>().unwrap();
                    let index = values[3].get::<u32>().unwrap();
                    recorded.borrow_mut().push(("closed", id, index));
                    let adapter = weak.upgrade().unwrap();
                    let value = record(&adapter, index as usize);
                    unsafe {
                        assert_eq!((*value).id as u32, id);
                    }
                    if id == 1 {
                        assert_eq!(value, first_pointer);
                        adapter.apply(added(notification(3, "Nested"), 1));
                        adapter.apply(closed(second.clone(), 1));
                        unsafe {
                            assert_eq!((*array(&adapter)).len, 2);
                            assert_eq!(CStr::from_ptr((*value).summary).to_bytes(), b"First");
                        }
                    }
                    None
                });
                let recorded = log.clone();
                adapter.connect_local("notification-added", false, move |values| {
                    recorded.borrow_mut().push((
                        "added",
                        values[2].get::<u32>().unwrap(),
                        values[3].get::<u32>().unwrap(),
                    ));
                    None
                });
                adapter.apply(closed(first, 0));
                assert_eq!(
                    *log.borrow(),
                    [("closed", 1, 0), ("added", 3, 1), ("closed", 2, 0)]
                );
                unsafe {
                    assert_eq!((*array(&adapter)).len, 1);
                    assert_eq!((*record(&adapter, 0)).id, 3);
                }
            })
            .unwrap();
    }

    #[test]
    fn replacement_preserves_position_and_old_borrows_without_a_close_signal() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let adapter: NotificationsAdapter = glib::Object::new();
                adapter.apply(added(notification(1, "First"), 0));
                let previous = notification(2, "  Old content  ");
                adapter.apply(added(previous.clone(), 1));
                let old_pointer = record(&adapter, 1);
                unsafe {
                    ffi::g_strchug((*old_pointer).summary);
                    ffi::g_strchomp((*old_pointer).summary);
                }
                assert_eq!(previous.request.summary, "  Old content  ");
                let stable_first = record(&adapter, 0);
                let closes = Rc::new(Cell::new(0));
                let recorded = closes.clone();
                adapter.connect_local("notification-closed", false, move |_| {
                    recorded.set(recorded.get() + 1);
                    None
                });
                let observed = Rc::new(Cell::new(false));
                let recorded = observed.clone();
                let weak = adapter.downgrade();
                adapter.connect_local("notification-replaced", false, move |values| {
                    assert_eq!(values[2].get::<u32>().unwrap(), 2);
                    assert_eq!(values[3].get::<u32>().unwrap(), 1);
                    let adapter = weak.upgrade().unwrap();
                    let replacement = record(&adapter, 1);
                    assert_ne!(replacement, old_pointer);
                    assert_eq!(record(&adapter, 0), stable_first);
                    unsafe {
                        assert_eq!(
                            CStr::from_ptr((*old_pointer).summary).to_bytes(),
                            b"Old content"
                        );
                        assert_eq!(
                            CStr::from_ptr((*replacement).summary).to_bytes(),
                            b"New content"
                        );
                    }
                    adapter.apply(added(notification(3, "Queued"), 2));
                    unsafe {
                        assert_eq!((*array(&adapter)).len, 2);
                    }
                    recorded.set(true);
                    None
                });
                // A later observer must still be able to borrow both records after
                // the earlier callback queued another event.
                adapter.connect_local("notification-replaced", false, move |_| {
                    unsafe {
                        assert_eq!(
                            CStr::from_ptr((*old_pointer).summary).to_bytes(),
                            b"Old content"
                        );
                    }
                    None
                });
                adapter.apply(NotificationEvent::Replaced {
                    notification: notification(2, "New content"),
                    previous: Box::new(previous),
                    index: 1,
                });
                assert!(observed.get());
                assert_eq!(closes.get(), 0);
                unsafe {
                    assert_eq!((*array(&adapter)).len, 3);
                }
            })
            .unwrap();
    }

    #[test]
    fn internal_c_requests_are_copied_and_adapter_callbacks_are_weak() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                // Construct without starting a bus connection; internal publication
                // is explicitly supported while the optional bus is unavailable.
                let service: NotificationsService = glib::Object::new();
                let adapter = NotificationsAdapter::new(service.clone());
                let mut source = notification(99, "  Internal request  ");
                source.request.origin = Origin::External;
                let source_record = Record::new(&source);
                unsafe {
                    notifications_service_send_notification(
                        adapter.as_ptr().cast(),
                        source_record.value.get(),
                    );
                }
                drop(source_record);
                drop(source);
                assert_eq!(service.state().notifications.len(), 1);
                let notification = service.state().notifications.remove(0);
                assert_ne!(notification.id, 0);
                assert_eq!(notification.request.origin, Origin::Internal);
                assert_eq!(notification.request.summary, "  Internal request  ");
                assert!(notification.request.actions.is_empty());
                unsafe {
                    assert_eq!((*record(&adapter, 0)).is_internal, 1);
                    assert_eq!(
                        notifications_service_invoke_action(
                            adapter.as_ptr().cast(),
                            notification.id,
                            c"reply".as_ptr()
                        ),
                        -1
                    );
                    assert_eq!(
                        notifications_service_closed_notification(
                            adapter.as_ptr().cast(),
                            notification.id,
                            2
                        ),
                        0
                    );
                    assert_eq!((*array(&adapter)).len, 0);
                    assert_eq!(
                        notifications_service_closed_notification(
                            adapter.as_ptr().cast(),
                            notification.id,
                            2
                        ),
                        -1
                    );
                    assert!(
                        notifications_service_get_notifications(std::ptr::null_mut()).is_null()
                    );
                }
                let weak = adapter.downgrade();
                drop(adapter);
                assert!(weak.upgrade().is_none());
                service
                    .send_internal(NotificationRequest {
                        summary: "After adapter disposal".into(),
                        ..Default::default()
                    })
                    .unwrap();
                let weak = service.downgrade();
                drop(service);
                assert!(weak.upgrade().is_none());
            })
            .unwrap();
    }

    #[test]
    fn shutdown_retains_the_borrowed_global_and_its_owned_history() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let service: NotificationsService = glib::Object::new();
                let adapter = NotificationsAdapter::new(service.clone());
                service
                    .send_internal(NotificationRequest {
                        summary: "Retained history".into(),
                        ..Default::default()
                    })
                    .unwrap();
                let pointer = adapter.as_ptr();
                let notifications = array(&adapter);
                let old_record = record(&adapter, 0);
                GLOBAL.with(|global| {
                    global.replace(Some(adapter));
                });
                shutdown();
                shutdown();
                assert_eq!(notifications_service_get_global(), pointer.cast());
                unsafe {
                    assert_eq!(
                        notifications_service_get_notifications(pointer.cast()),
                        notifications
                    );
                    assert_eq!((*notifications).len, 1);
                    assert_eq!(
                        CStr::from_ptr((*old_record).summary).to_bytes(),
                        b"Retained history"
                    );
                }
                GLOBAL.with(|global| global.borrow_mut().take());
                service.stop();
            })
            .unwrap();
    }

    #[test]
    fn live_dbus_notifications_reach_c_records_and_c_actions_reach_the_bus() {
        use fixture::{Bus, connect, wait};
        let (_bus, address) = Bus::start();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let server = connect(&address);
                let client = connect(&address);
                let service = NotificationsService::on_connection(&server);
                let adapter = NotificationsAdapter::new(service.clone());
                wait(&context, || service.state().available);
                let signals = Rc::new(RefCell::new(Vec::new()));
                let recorded = signals.clone();
                let _subscription = client.subscribe_to_signal(
                    Some("org.freedesktop.Notifications"),
                    Some("org.freedesktop.Notifications"),
                    None,
                    Some("/org/freedesktop/Notifications"),
                    None,
                    gio::DBusSignalFlags::NONE,
                    move |signal| {
                        recorded
                            .borrow_mut()
                            .push((signal.signal_name.to_owned(), signal.parameters.clone()))
                    },
                );
                let notify = |summary: &str, replaces: u32, timeout: i32| {
                    let parameters = (
                        "Live fixture",
                        replaces,
                        "",
                        summary,
                        "Body",
                        vec!["reply", "Reply"],
                        std::collections::HashMap::<String, glib::Variant>::new(),
                        timeout,
                    )
                        .to_variant();
                    context
                        .block_on(client.call_future(
                            Some("org.freedesktop.Notifications"),
                            "/org/freedesktop/Notifications",
                            "org.freedesktop.Notifications",
                            "Notify",
                            Some(&parameters),
                            None,
                            gio::DBusCallFlags::NONE,
                            2_000,
                        ))
                        .unwrap()
                        .get::<(u32,)>()
                        .unwrap()
                        .0
                };
                let id = notify("Original", 0, 0);
                let replacements = Rc::new(Cell::new(0));
                let observed = replacements.clone();
                adapter.connect_local("notification-replaced", false, move |values| {
                    assert_eq!(values[2].get::<u32>().unwrap(), id);
                    assert_eq!(values[3].get::<u32>().unwrap(), 0);
                    observed.set(observed.get() + 1);
                    None
                });
                assert_eq!(notify("Replacement", id, 0), id);
                assert_eq!(replacements.get(), 1);
                let pointer = adapter.as_ptr().cast();
                unsafe {
                    assert_eq!((*array(&adapter)).len, 1);
                    assert_eq!(
                        CStr::from_ptr((*record(&adapter, 0)).summary).to_bytes(),
                        b"Replacement"
                    );
                    assert_eq!(
                        notifications_service_invoke_action(pointer, id, c"reply".as_ptr()),
                        0
                    );
                    assert_eq!(
                        notifications_service_invoke_action(pointer, id, c"missing".as_ptr()),
                        -1
                    );
                    assert_eq!(notifications_service_closed_notification(pointer, id, 2), 0);
                    assert_eq!((*array(&adapter)).len, 0);
                }
                wait(&context, || {
                    signals
                        .borrow()
                        .iter()
                        .any(|(name, _)| name == "NotificationClosed")
                });
                assert_eq!(
                    signals
                        .borrow()
                        .iter()
                        .find(|(name, _)| name == "ActionInvoked")
                        .unwrap()
                        .1
                        .get::<(u32, String)>(),
                    Some((id, "reply".into()))
                );
                assert_eq!(
                    signals
                        .borrow()
                        .iter()
                        .filter(|(name, _)| name == "NotificationClosed")
                        .count(),
                    1
                );
                assert_eq!(
                    signals
                        .borrow()
                        .iter()
                        .find(|(name, _)| name == "NotificationClosed")
                        .unwrap()
                        .1
                        .get::<(u32, u32)>(),
                    Some((id, 2))
                );
                let expires = notify("Short lived", 0, 20);
                wait(&context, || unsafe { (*array(&adapter)).len == 0 });
                wait(&context, || {
                    signals.borrow().iter().any(|(name, values)| {
                        name == "NotificationClosed"
                            && values.get::<(u32, u32)>() == Some((expires, 1))
                    })
                });
                let weak = adapter.downgrade();
                drop(adapter);
                assert!(weak.upgrade().is_none());
                client.close_sync(gio::Cancellable::NONE).unwrap();
                server.close_sync(gio::Cancellable::NONE).unwrap();
            })
            .unwrap();
    }
}
