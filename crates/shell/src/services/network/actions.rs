//! Connection requests use libnm's owning D-Bus connection and cancellation scope.
use super::{NetworkService, connections, native};
use gio::prelude::*;
use glib::{subclass::prelude::*, variant::ObjectPath};
use std::collections::{HashMap, VecDeque};

const ROOT: &str = "/org/freedesktop/NetworkManager";
const MANAGER: &str = "org.freedesktop.NetworkManager";
const SAVED: &str = "org.freedesktop.NetworkManager.Settings.Connection";
pub(super) struct Call {
    pub path: String,
    pub interface: &'static str,
    pub method: &'static str,
    pub parameters: glib::Variant,
    pub reply: &'static str,
}
impl Call {
    pub fn manager(method: &'static str, parameters: glib::Variant, reply: &'static str) -> Self {
        Self {
            path: ROOT.into(),
            interface: MANAGER,
            method,
            parameters,
            reply,
        }
    }
}
fn object_path(value: &str) -> Result<ObjectPath, glib::Error> {
    ObjectPath::try_from(value).map_err(|_| invalid("Invalid NetworkManager object path"))
}
fn invalid(message: &str) -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::InvalidArgument, message)
}
struct Operation {
    service: glib::WeakRef<NetworkService>,
    client: native::Client,
    generation: u64,
    id: u64,
    cancel: gio::Cancellable,
    calls: VecDeque<Call>,
    done: Box<dyn FnOnce(Result<(), glib::Error>)>,
}
impl Operation {
    fn run(mut self) {
        let Some(call) = self.calls.pop_front() else {
            self.finish(Ok(()));
            return;
        };
        let client = self.client.clone();
        let cancel = self.cancel.clone();
        client.call(&call, &cancel, move |result| {
            let current = self
                .service
                .upgrade()
                .is_some_and(|service| service.imp().generation.get() == self.generation);
            if !current || self.cancel.is_cancelled() {
                self.finish(Err(glib::Error::new(
                    gio::IOErrorEnum::Cancelled,
                    "Network request cancelled",
                )));
            } else if let Err(error) = result {
                self.finish(Err(error));
            } else {
                self.run();
            }
        });
    }
    fn finish(self, result: Result<(), glib::Error>) {
        if let Some(service) = self.service.upgrade() {
            service.imp().operations.borrow_mut().remove(&self.id);
        }
        (self.done)(result);
    }
}
impl NetworkService {
    pub(super) fn execute(
        &self,
        calls: Result<Vec<Call>, glib::Error>,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) -> gio::Cancellable {
        let cancel = gio::Cancellable::new();
        let calls = match calls {
            Ok(calls) => calls,
            Err(error) => {
                done(Err(error));
                return cancel;
            }
        };
        let client = self.imp().client.borrow().clone();
        let Some(client) = client.filter(|_| self.state().available) else {
            done(Err(glib::Error::new(
                gio::IOErrorEnum::NotConnected,
                "NetworkManager is unavailable",
            )));
            return cancel;
        };
        let id = self.imp().next_operation.get();
        self.imp().next_operation.set(id.wrapping_add(1));
        self.imp()
            .operations
            .borrow_mut()
            .insert(id, cancel.clone());
        Operation {
            service: self.downgrade(),
            client,
            generation: self.imp().generation.get(),
            id,
            cancel: cancel.clone(),
            calls: calls.into(),
            done: Box::new(done),
        }
        .run();
        cancel
    }
    pub fn join_access_point(
        &self,
        device: &str,
        access_point: &str,
        password: Option<&str>,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) -> gio::Cancellable {
        self.execute(self.join_calls(device, access_point, password), done)
    }
    fn join_calls(
        &self,
        device: &str,
        access_point: &str,
        password: Option<&str>,
    ) -> Result<Vec<Call>, glib::Error> {
        let state = self.state();
        let ap = state
            .access_points
            .iter()
            .find(|ap| ap.id == access_point && ap.device == device)
            .ok_or_else(|| invalid("Wi-Fi access point is no longer available on this device"))?;
        if ap.ssid.is_empty() || ap.ssid.iter().all(|byte| *byte == 0) {
            return Err(invalid("The selected access point has no SSID"));
        }
        if password.is_some_and(|password| password.contains('\0')) {
            return Err(invalid("Wi-Fi password contains a NUL byte"));
        }
        let saved = state.connections.iter().find(|connection| {
            connection.kind == "802-11-wireless" && connection.ssid.as_ref() == Some(&ap.ssid)
        });
        let mut calls = Vec::new();
        if let Some(saved) = saved {
            if let Some(password) = password {
                let client = self
                    .imp()
                    .client
                    .borrow()
                    .clone()
                    .ok_or_else(|| invalid("NetworkManager is unavailable"))?;
                let connection = client
                    .connections()
                    .into_iter()
                    .find(|connection| connections::saved_path(connection) == saved.id)
                    .ok_or_else(|| invalid("Saved Wi-Fi connection is no longer available"))?;
                let mut settings = connections::settings(&connection)
                    .ok_or_else(|| invalid("Saved Wi-Fi settings are unavailable"))?;
                set_password(&mut settings, password);
                calls.push(Call {
                    path: saved.id.clone(),
                    interface: SAVED,
                    method: "Update",
                    parameters: (settings,).to_variant(),
                    reply: "()",
                });
            }
            calls.push(Call::manager(
                "ActivateConnection",
                (
                    object_path(&saved.id)?,
                    object_path(device)?,
                    object_path("/")?,
                )
                    .to_variant(),
                "(o)",
            ));
        } else {
            let mut settings = HashMap::from([
                (
                    "connection".into(),
                    HashMap::from([
                        ("id".into(), ap.name.to_variant()),
                        ("type".into(), "802-11-wireless".to_variant()),
                        ("autoconnect".into(), true.to_variant()),
                    ]),
                ),
                (
                    "802-11-wireless".into(),
                    HashMap::from([("ssid".into(), ap.ssid.to_variant())]),
                ),
            ]);
            if let Some(password) = password {
                set_password(&mut settings, password);
            }
            calls.push(Call::manager(
                "AddAndActivateConnection",
                (settings, object_path(device)?, object_path("/")?).to_variant(),
                "(oo)",
            ));
        }
        Ok(calls)
    }
    pub fn activate_connection(
        &self,
        connection: &str,
        device: Option<&str>,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) -> gio::Cancellable {
        let calls = (|| {
            let state = self.state();
            let saved = state
                .connections
                .iter()
                .find(|saved| saved.id == connection)
                .ok_or_else(|| invalid("Saved connection is no longer available"))?;
            let device = if saved.kind == "wireguard" {
                "/"
            } else {
                device.unwrap_or("/")
            };
            if device != "/" && !state.devices.iter().any(|known| known.id == device) {
                return Err(invalid("Network device is no longer available"));
            }
            Ok(vec![Call::manager(
                "ActivateConnection",
                (
                    object_path(connection)?,
                    object_path(device)?,
                    object_path("/")?,
                )
                    .to_variant(),
                "(o)",
            )])
        })();
        self.execute(calls, done)
    }
    pub fn disconnect_device(
        &self,
        device: &str,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) -> gio::Cancellable {
        let calls = (|| {
            let state = self.state();
            let device = state
                .devices
                .iter()
                .find(|known| known.id == device)
                .ok_or_else(|| invalid("Network device is no longer available"))?;
            device
                .active_connection
                .as_deref()
                .map(deactivate)
                .transpose()
                .map(|call| call.into_iter().collect())
        })();
        self.execute(calls, done)
    }
    pub fn set_vpn(
        &self,
        connection: &str,
        enabled: bool,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) -> gio::Cancellable {
        let calls = (|| {
            let state = self.state();
            let saved = state
                .connections
                .iter()
                .find(|saved| saved.id == connection && saved.is_vpn())
                .ok_or_else(|| invalid("VPN connection is no longer available"))?;
            if enabled {
                let device = if saved.kind == "wireguard" {
                    "/"
                } else {
                    state.primary.as_deref().unwrap_or("/")
                };
                Ok(vec![Call::manager(
                    "ActivateConnection",
                    (
                        object_path(connection)?,
                        object_path(device)?,
                        object_path("/")?,
                    )
                        .to_variant(),
                    "(o)",
                )])
            } else {
                state
                    .active_connections
                    .iter()
                    .filter(|active| active.connection.as_deref() == Some(connection))
                    .map(|active| deactivate(&active.id))
                    .collect()
            }
        })();
        self.execute(calls, done)
    }
    pub fn request_scan(
        &self,
        device: &str,
        done: impl FnOnce(Result<(), glib::Error>) + 'static,
    ) -> gio::Cancellable {
        let calls = if self
            .state()
            .devices
            .iter()
            .any(|known| known.id == device && known.kind == 2)
        {
            Ok(vec![Call {
                path: device.into(),
                interface: "org.freedesktop.NetworkManager.Device.Wireless",
                method: "RequestScan",
                parameters: (HashMap::<String, glib::Variant>::new(),).to_variant(),
                reply: "()",
            }])
        } else {
            Err(invalid("Wi-Fi device is no longer available"))
        };
        self.execute(calls, done)
    }
}
fn deactivate(active: &str) -> Result<Call, glib::Error> {
    Ok(Call::manager(
        "DeactivateConnection",
        (object_path(active)?,).to_variant(),
        "()",
    ))
}
fn set_password(settings: &mut connections::Settings, password: &str) {
    let security = settings
        .entry("802-11-wireless-security".into())
        .or_default();
    security.insert("key-mgmt".into(), "wpa-psk".to_variant());
    security.insert("psk".into(), password.to_variant());
}
