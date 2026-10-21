//! RequestScan acknowledges acceptance; a newer LastScan completes the scan.
//! https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.Device.Wireless.html
use super::{NetworkService, actions::Call};
use gio::prelude::*;
use glib::subclass::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};

type Done = Box<dyn FnOnce(Result<(), glib::Error>)>;
struct Scan {
    service: glib::WeakRef<NetworkService>,
    device: String,
    baseline: Option<i64>,
    accepted: Cell<bool>,
    observed: Cell<bool>,
    generation: u64,
    id: u64,
    cancel: gio::Cancellable,
    changed: RefCell<Option<glib::SignalHandlerId>>,
    cancelled: RefCell<Option<gio::CancelledHandlerId>>,
    deadline: RefCell<Option<glib::JoinHandle<()>>>,
    cancellation: RefCell<Option<glib::JoinHandle<()>>>,
    done: RefCell<Option<Done>>,
}
impl Scan {
    fn check(&self) {
        if self.done.borrow().is_none() {
            return;
        }
        let Some(service) = self.service.upgrade() else {
            self.finish(Err(cancelled()));
            return;
        };
        let state = service.state();
        let device = state
            .devices
            .iter()
            .find(|device| device.id == self.device && device.kind == 2);
        if self.cancel.is_cancelled()
            || service.imp().generation.get() != self.generation
            || !state.available
            || !state.networking_enabled
            || !state.wireless_enabled
            || !state.wireless_hardware_enabled
            || device.is_none()
        {
            self.finish(Err(cancelled()));
            return;
        }
        if device
            .unwrap()
            .last_scan
            .is_some_and(|scan| self.baseline.is_none_or(|base| scan > base))
        {
            self.observed.set(true);
        }
        if self.accepted.get() && self.observed.get() {
            self.finish(Ok(()));
        }
    }
    fn finish(&self, result: Result<(), glib::Error>) {
        let Some(done) = self.done.borrow_mut().take() else {
            return;
        };
        if let Some(service) = self.service.upgrade() {
            if let Some(handler) = self.changed.borrow_mut().take() {
                service.disconnect(handler);
            }
            service.imp().operations.borrow_mut().remove(&self.id);
        }
        if let Some(handler) = self.cancelled.borrow_mut().take() {
            self.cancel.disconnect_cancelled(handler);
        }
        if let Some(task) = self.deadline.borrow_mut().take() {
            task.abort();
        }
        if let Some(task) = self.cancellation.borrow_mut().take() {
            task.abort();
        }
        // Cancels observation and an outstanding method call, not an accepted hardware scan.
        self.cancel.cancel();
        done(result);
    }
}
fn cancelled() -> glib::Error {
    glib::Error::new(
        gio::IOErrorEnum::Cancelled,
        "Wi-Fi scan observation cancelled",
    )
}
pub(super) fn request(
    service: &NetworkService,
    device: &str,
    done: impl FnOnce(Result<(), glib::Error>) + 'static,
) -> gio::Cancellable {
    let cancel = gio::Cancellable::new();
    let state = service.state();
    let known = state
        .devices
        .iter()
        .find(|known| known.id == device && known.kind == 2);
    let Some(known) = known else {
        done(Err(glib::Error::new(
            gio::IOErrorEnum::InvalidArgument,
            "Wi-Fi device is no longer available",
        )));
        return cancel;
    };
    let Some(client) = service
        .imp()
        .client
        .borrow()
        .clone()
        .filter(|_| state.available)
    else {
        done(Err(glib::Error::new(
            gio::IOErrorEnum::NotConnected,
            "NetworkManager is unavailable",
        )));
        return cancel;
    };
    let id = service.imp().next_operation.get();
    service.imp().next_operation.set(id.wrapping_add(1));
    service
        .imp()
        .operations
        .borrow_mut()
        .insert(id, cancel.clone());
    let scan = Rc::new(Scan {
        service: service.downgrade(),
        device: device.into(),
        baseline: known.last_scan,
        accepted: Cell::new(false),
        observed: Cell::new(false),
        generation: service.imp().generation.get(),
        id,
        cancel: cancel.clone(),
        changed: RefCell::new(None),
        cancelled: RefCell::new(None),
        deadline: RefCell::new(None),
        cancellation: RefCell::new(None),
        done: RefCell::new(Some(Box::new(done))),
    });
    let observer = scan.clone();
    scan.changed
        .replace(Some(service.connect_local("changed", false, move |_| {
            observer.check();
            None
        })));
    let context = glib::MainContext::ref_thread_default();
    let weak = Rc::downgrade(&scan);
    scan.cancelled
        .replace(cancel.connect_cancelled_local(move |_| {
            if let Some(scan) = weak.upgrade() {
                // Disconnect outside the cancellable's callback to avoid a GIO deadlock.
                let pending = scan.clone();
                scan.cancellation
                    .replace(Some(context.spawn_local(async move {
                        pending.cancellation.borrow_mut().take();
                        pending.finish(Err(cancelled()));
                    })));
            }
        }));
    let weak = Rc::downgrade(&scan);
    scan.deadline
        .replace(Some(glib::MainContext::ref_thread_default().spawn_local(
            async move {
                glib::timeout_future(Duration::from_secs(30)).await;
                if let Some(scan) = weak.upgrade() {
                    scan.deadline.borrow_mut().take();
                    scan.finish(Err(glib::Error::new(
                        gio::IOErrorEnum::TimedOut,
                        "Wi-Fi scan did not complete within 30 seconds",
                    )));
                }
            },
        )));
    scan.check();
    if scan.done.borrow().is_some() {
        let call = Call {
            path: device.into(),
            interface: "org.freedesktop.NetworkManager.Device.Wireless",
            method: "RequestScan",
            parameters: (HashMap::<String, glib::Variant>::new(),).to_variant(),
            reply: "()",
        };
        client.call(&call, &cancel, move |result| {
            if let Err(error) = result {
                scan.finish(Err(error));
            } else {
                scan.accepted.set(true);
                scan.check();
            }
        });
    }
    cancel
}
