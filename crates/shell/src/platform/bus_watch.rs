//! Main-context-owned D-Bus name watches that also handle loss of the bus.
//!
//! GIO passes a null connection to the vanished callback when a bus cannot be
//! reached or disconnects. gio 0.21's watch helpers unwrap that argument as a
//! non-null object, so use the native closure entry points until that is fixed.
use glib::translate::*;
use std::{marker::PhantomData, num::NonZeroU32, rc::Rc};

pub(crate) struct Watcher {
    id: NonZeroU32,
    // Local closures must be released on the thread where they were created.
    _thread: PhantomData<Rc<()>>,
}

impl Watcher {
    pub(crate) fn on_bus(
        bus: gio::BusType,
        name: &str,
        appeared: impl Fn(gio::DBusConnection, &str) + 'static,
        vanished: impl Fn() + 'static,
    ) -> Self {
        let appeared = appeared_closure(appeared);
        let vanished = vanished_closure(vanished);
        // SAFETY: Strings and closures remain live for the call. GIO retains
        // both closures until this watcher is unwatched on its owning context.
        let id = unsafe {
            gio::ffi::g_bus_watch_name_with_closures(
                bus.into_glib(),
                name.to_glib_none().0,
                gio::BusNameWatcherFlags::NONE.into_glib(),
                appeared.to_glib_none().0,
                vanished.to_glib_none().0,
            )
        };
        Self::from_id(id)
    }

    pub(crate) fn on_connection(
        connection: &gio::DBusConnection,
        name: &str,
        appeared: impl Fn(gio::DBusConnection, &str) + 'static,
        vanished: impl Fn() + 'static,
    ) -> Self {
        let appeared = appeared_closure(appeared);
        let vanished = vanished_closure(vanished);
        // SAFETY: GIO retains the supplied connection and closures until the
        // unique watcher ID is released by Drop.
        let id = unsafe {
            gio::ffi::g_bus_watch_name_on_connection_with_closures(
                connection.to_glib_none().0,
                name.to_glib_none().0,
                gio::BusNameWatcherFlags::NONE.into_glib(),
                appeared.to_glib_none().0,
                vanished.to_glib_none().0,
            )
        };
        Self::from_id(id)
    }

    fn from_id(id: u32) -> Self {
        Self {
            id: NonZeroU32::new(id).expect("GIO returned an invalid bus watcher ID"),
            _thread: PhantomData,
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // SAFETY: This type uniquely owns the ID and cannot cross threads.
        unsafe { gio::ffi::g_bus_unwatch_name(self.id.get()) }
    }
}

fn appeared_closure(appeared: impl Fn(gio::DBusConnection, &str) + 'static) -> glib::Closure {
    glib::Closure::new_local(move |args| {
        // A name can only appear on a live connection; its unique owner is the
        // third argument. The watched well-known name is already known.
        let connection = args[0].get::<gio::DBusConnection>().unwrap();
        let owner = args[2].get::<&str>().unwrap();
        appeared(connection, owner);
        None
    })
}

fn vanished_closure(vanished: impl Fn() + 'static) -> glib::Closure {
    glib::Closure::new_local(move |_| {
        // The connection may be null. Callers only need to clear owned state.
        vanished();
        None
    })
}
