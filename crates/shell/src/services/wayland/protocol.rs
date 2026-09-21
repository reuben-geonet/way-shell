//! All proxies belong to this queue and its separate Wayland connection.
use super::model::{Output, Seat, Toplevel, ignored, version};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::OpenOptions,
    io::{Seek, Write},
    os::{fd::AsFd, unix::fs::OpenOptionsExt},
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
    protocol::{wl_callback, wl_output, wl_registry, wl_seat},
};
use wayland_protocols_wlr::{
    foreign_toplevel::v1::client::{
        zwlr_foreign_toplevel_handle_v1 as handle, zwlr_foreign_toplevel_manager_v1 as foreign,
    },
    gamma_control::v1::client::{
        zwlr_gamma_control_manager_v1 as gamma_manager, zwlr_gamma_control_v1 as gamma,
    },
};

#[derive(Clone, Debug)]
pub(super) enum Change {
    Ready,
    Output(Output),
    OutputRemoved(Output),
    Toplevel(Toplevel),
    ToplevelRemoved(Toplevel),
    Capabilities,
    Gamma(bool),
    Diagnostic(String),
}
struct OutputEntry {
    proxy: wl_output::WlOutput,
    value: Output,
}
struct SeatEntry {
    proxy: wl_seat::WlSeat,
    value: Seat,
}
struct TopEntry {
    proxy: handle::ZwlrForeignToplevelHandleV1,
    pending: Toplevel,
    published: Option<Toplevel>,
}
struct GammaEntry {
    proxy: gamma::ZwlrGammaControlV1,
    size: usize,
}

#[derive(Default)]
pub(super) struct State {
    outputs: BTreeMap<u32, OutputEntry>,
    seats: BTreeMap<u32, SeatEntry>,
    toplevels: BTreeMap<u64, TopEntry>,
    next_toplevel: u64,
    foreign: Option<(u32, foreign::ZwlrForeignToplevelManagerV1)>,
    gamma_manager: Option<(u32, gamma_manager::ZwlrGammaControlManagerV1)>,
    shortcut_manager: Option<u32>,
    gamma: BTreeMap<u32, GammaEntry>,
    failed_gamma_outputs: BTreeSet<u32>,
    temperature: Option<u32>,
    ignored_apps: BTreeSet<String>,
    ignored_titles: BTreeSet<String>,
    pub changes: Vec<Change>,
    pub ready: bool,
}

impl State {
    pub fn outputs(&self) -> Vec<Output> {
        self.outputs
            .values()
            .filter(|entry| entry.value.initialized)
            .map(|entry| entry.value.clone())
            .collect()
    }
    pub fn seats(&self) -> Vec<Seat> {
        self.seats
            .values()
            .map(|entry| entry.value.clone())
            .collect()
    }
    pub fn toplevels(&self) -> Vec<Toplevel> {
        self.toplevels
            .values()
            .filter_map(|entry| entry.published.clone())
            .collect()
    }
    pub fn has_foreign_toplevel(&self) -> bool {
        self.foreign.is_some()
    }
    pub fn has_gamma(&self) -> bool {
        self.gamma_manager.is_some()
    }
    pub fn gamma_available(&self) -> bool {
        self.has_gamma()
            && self
                .outputs
                .keys()
                .any(|id| !self.failed_gamma_outputs.contains(id))
    }
    pub fn has_shortcut_inhibition(&self) -> bool {
        self.shortcut_manager.is_some()
    }
    pub fn gamma_enabled(&self) -> bool {
        self.temperature.is_some() && !self.gamma.is_empty()
    }

    pub fn ignored(&mut self, apps: &str, titles: &str) {
        self.ignored_apps = ignored(apps);
        self.ignored_titles = ignored(titles);
        let ids: Vec<_> = self.toplevels.keys().copied().collect();
        for id in ids {
            self.publish(id);
        }
    }
    fn publish(&mut self, id: u64) {
        let Some(entry) = self.toplevels.get_mut(&id) else {
            return;
        };
        if entry
            .pending
            .visible(&self.ignored_apps, &self.ignored_titles)
        {
            let value = entry.pending.clone();
            entry.published = Some(value.clone());
            self.changes.push(Change::Toplevel(value));
        } else if let Some(value) = entry.published.take() {
            self.changes.push(Change::ToplevelRemoved(value));
        }
        entry.pending.activation_event = false;
        entry.pending.entered_event = false;
    }
    fn remove_toplevel(&mut self, id: u64) {
        if let Some(entry) = self.toplevels.remove(&id) {
            if let Some(value) = entry.published {
                self.changes.push(Change::ToplevelRemoved(value));
            }
            entry.proxy.destroy();
        }
    }
    fn find_toplevel(&self, proxy: &handle::ZwlrForeignToplevelHandleV1) -> Option<u64> {
        self.toplevels
            .iter()
            .find_map(|(id, entry)| (entry.proxy == *proxy).then_some(*id))
    }
    pub fn action(&self, id: u64, action: super::ToplevelAction) -> Result<(), String> {
        let entry = self.toplevels.get(&id).ok_or("This window has closed")?;
        match action {
            super::ToplevelAction::Activate => {
                let seat = self
                    .seats
                    .values()
                    .next()
                    .ok_or("No Wayland seat is available to activate a window")?;
                entry.proxy.activate(&seat.proxy);
            }
            super::ToplevelAction::Close => entry.proxy.close(),
            super::ToplevelAction::Maximize => entry.proxy.set_maximized(),
        }
        Ok(())
    }
    pub fn set_temperature(
        &mut self,
        temperature: u32,
        qh: &QueueHandle<Self>,
    ) -> Result<(), String> {
        if !(1000..=25000).contains(&temperature) {
            return Err("Gamma temperature must be between 1000 and 25000 K".into());
        }
        if self.gamma_manager.is_none() {
            return Err("The compositor does not support gamma control".into());
        }
        let was_enabled = self.gamma_enabled();
        self.failed_gamma_outputs.clear();
        self.temperature = Some(temperature);
        let ids: Vec<_> = self.outputs.keys().copied().collect();
        for id in ids {
            self.add_gamma(id, qh);
        }
        for controller in self.gamma.values() {
            if controller.size > 0
                && let Err(error) = apply_gamma(controller, temperature)
            {
                self.changes.push(Change::Diagnostic(error));
            }
        }
        if was_enabled != self.gamma_enabled() {
            self.changes.push(Change::Gamma(self.gamma_enabled()));
        }
        self.changes.push(Change::Capabilities);
        Ok(())
    }
    pub fn disable_gamma(&mut self) {
        let was_enabled = self.gamma_enabled();
        self.temperature = None;
        for (_, entry) in std::mem::take(&mut self.gamma) {
            entry.proxy.destroy();
        }
        if was_enabled {
            self.changes.push(Change::Gamma(false));
        }
    }
    fn add_gamma(&mut self, id: u32, qh: &QueueHandle<Self>) {
        if self.temperature.is_none() || self.gamma.contains_key(&id) {
            return;
        }
        if let (Some((_, manager)), Some(output)) = (&self.gamma_manager, self.outputs.get(&id)) {
            let proxy = manager.get_gamma_control(&output.proxy, qh, id);
            self.gamma.insert(id, GammaEntry { proxy, size: 0 });
        }
    }
    pub fn close(&mut self) {
        self.disable_gamma();
        for id in self.toplevels.keys().copied().collect::<Vec<_>>() {
            self.remove_toplevel(id);
        }
        if let Some((_, manager)) = self.foreign.take() {
            manager.stop();
        }
        if let Some((_, manager)) = self.gamma_manager.take() {
            manager.destroy();
        }
        for (_, entry) in std::mem::take(&mut self.outputs) {
            if entry.proxy.version() >= 3 {
                entry.proxy.release();
            }
        }
        for (_, entry) in std::mem::take(&mut self.seats) {
            if entry.proxy.version() >= 5 {
                entry.proxy.release();
            }
        }
    }
}

fn apply_gamma(entry: &GammaEntry, temperature: u32) -> Result<(), String> {
    let ramp = way_shell_core::gamma::ramp(entry.size, temperature)
        .ok_or("Invalid compositor gamma ramp size")?;
    // The protocol transfers its own descriptor. Unlink before sharing, then
    // close our descriptor after sending; no persistent names or shared buffers.
    let path = std::env::temp_dir().join(format!("way-shell-gamma-{}", glib::uuid_string_random()));
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| format!("Could not create gamma table: {e}"))?;
    std::fs::remove_file(&path).map_err(|e| format!("Could not unlink gamma table: {e}"))?;
    let bytes: Vec<_> = ramp.into_iter().flat_map(u16::to_ne_bytes).collect();
    file.write_all(&bytes)
        .map_err(|e| format!("Could not write gamma table: {e}"))?;
    file.rewind()
        .map_err(|e| format!("Could not rewind gamma table: {e}"))?;
    entry.proxy.set_gamma(file.as_fd());
    Ok(())
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version: advertised,
            } => {
                let maximum = match interface.as_str() {
                    "wl_output" => 4,
                    "wl_seat" => 7,
                    "zwlr_foreign_toplevel_manager_v1" => 3,
                    "zwlr_gamma_control_manager_v1"
                    | "zwp_keyboard_shortcuts_inhibit_manager_v1" => 1,
                    _ => return,
                };
                let Some(version) = version(advertised, maximum) else {
                    return;
                };
                match interface.as_str() {
                    "wl_output" => {
                        let was_enabled = state.gamma_enabled();
                        let proxy = registry.bind(name, version, qh, name);
                        state.outputs.insert(
                            name,
                            OutputEntry {
                                proxy,
                                value: Output {
                                    id: name,
                                    scale: 1,
                                    ..Default::default()
                                },
                            },
                        );
                        state.add_gamma(name, qh);
                        if was_enabled != state.gamma_enabled() {
                            state.changes.push(Change::Gamma(state.gamma_enabled()));
                        }
                    }
                    "wl_seat" => {
                        let proxy = registry.bind(name, version, qh, name);
                        state.seats.insert(
                            name,
                            SeatEntry {
                                proxy,
                                value: Seat {
                                    id: name,
                                    ..Default::default()
                                },
                            },
                        );
                    }
                    "zwlr_foreign_toplevel_manager_v1" if state.foreign.is_none() => {
                        state.foreign = Some((name, registry.bind(name, version, qh, ())))
                    }
                    "zwlr_gamma_control_manager_v1" if state.gamma_manager.is_none() => {
                        state.gamma_manager = Some((name, registry.bind(name, version, qh, ())))
                    }
                    "zwp_keyboard_shortcuts_inhibit_manager_v1" => {
                        state.shortcut_manager = Some(name)
                    }
                    _ => {}
                }
                state.changes.push(Change::Capabilities);
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(entry) = state.outputs.remove(&name) {
                    state.failed_gamma_outputs.remove(&name);
                    let was_enabled = state.gamma_enabled();
                    if let Some(gamma) = state.gamma.remove(&name) {
                        gamma.proxy.destroy();
                    }
                    for top in state.toplevels.values_mut() {
                        top.pending.outputs.remove(&name);
                        if let Some(value) = &mut top.published {
                            value.outputs.remove(&name);
                        }
                    }
                    state.changes.push(Change::OutputRemoved(entry.value));
                    if entry.proxy.version() >= 3 {
                        entry.proxy.release();
                    }
                    if was_enabled != state.gamma_enabled() {
                        state.changes.push(Change::Gamma(state.gamma_enabled()));
                    }
                }
                if let Some(entry) = state.seats.remove(&name)
                    && entry.proxy.version() >= 5
                {
                    entry.proxy.release();
                }
                if state.foreign.as_ref().is_some_and(|(id, _)| *id == name) {
                    if let Some((_, manager)) = state.foreign.take() {
                        manager.stop();
                    }
                    for id in state.toplevels.keys().copied().collect::<Vec<_>>() {
                        state.remove_toplevel(id);
                    }
                }
                if state
                    .gamma_manager
                    .as_ref()
                    .is_some_and(|(id, _)| *id == name)
                {
                    state.disable_gamma();
                    state.failed_gamma_outputs.clear();
                    if let Some((_, manager)) = state.gamma_manager.take() {
                        manager.destroy();
                    }
                }
                if state.shortcut_manager == Some(name) {
                    state.shortcut_manager = None;
                }
                state.changes.push(Change::Capabilities);
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_callback::WlCallback, bool> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        second: &bool,
        connection: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if *second {
            // wl_output v1 has no done event; the second sync completes its batch.
            for entry in state.outputs.values_mut() {
                if !entry.value.initialized {
                    entry.value.initialized = true;
                    state.changes.push(Change::Output(entry.value.clone()));
                }
            }
            state.ready = true;
            state.changes.push(Change::Ready);
        } else {
            connection.display().sync(qh, true);
        }
    }
}
impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        proxy: &wl_output::WlOutput,
        event: wl_output::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(entry) = state.outputs.get_mut(id) else {
            return;
        };
        match event {
            wl_output::Event::Geometry { make, model, .. } => {
                entry.value.make = make;
                entry.value.model = model;
            }
            wl_output::Event::Name { name } => entry.value.name = Some(name),
            wl_output::Event::Description { description } => {
                entry.value.description = Some(description)
            }
            wl_output::Event::Scale { factor } => entry.value.scale = factor,
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) => {
                entry.value.width = width;
                entry.value.height = height;
            }
            wl_output::Event::Done => {
                entry.value.initialized = true;
                state.changes.push(Change::Output(entry.value.clone()));
            }
            _ => {}
        }
        if proxy.version() == 1 && state.ready {
            entry.value.initialized = true;
            state.changes.push(Change::Output(entry.value.clone()));
        }
    }
}
impl Dispatch<wl_seat::WlSeat, u32> for State {
    fn event(
        state: &mut Self,
        _: &wl_seat::WlSeat,
        event: wl_seat::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(entry) = state.seats.get_mut(id) else {
            return;
        };
        match event {
            wl_seat::Event::Name { name } => entry.value.name = Some(name),
            wl_seat::Event::Capabilities { capabilities } => {
                entry.value.capabilities = match capabilities {
                    WEnum::Value(v) => v.bits(),
                    WEnum::Unknown(v) => v,
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<foreign::ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &foreign::ZwlrForeignToplevelManagerV1,
        event: foreign::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            foreign::Event::Toplevel { toplevel } => {
                state.next_toplevel += 1;
                let id = state.next_toplevel;
                state.toplevels.insert(
                    id,
                    TopEntry {
                        proxy: toplevel,
                        pending: Toplevel {
                            id,
                            ..Default::default()
                        },
                        published: None,
                    },
                );
            }
            foreign::Event::Finished => {
                state.foreign = None;
                for id in state.toplevels.keys().copied().collect::<Vec<_>>() {
                    state.remove_toplevel(id);
                }
                state.changes.push(Change::Capabilities);
            }
            _ => {}
        }
    }
    wayland_client::event_created_child!(State, foreign::ZwlrForeignToplevelManagerV1, [0 => (handle::ZwlrForeignToplevelHandleV1, ())]);
}
impl Dispatch<handle::ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &handle::ZwlrForeignToplevelHandleV1,
        event: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(id) = state.find_toplevel(proxy) else {
            return;
        };
        let entry = state.toplevels.get_mut(&id).unwrap();
        match event {
            handle::Event::Title { title } => entry.pending.title = Some(title),
            handle::Event::AppId { app_id } => entry.pending.app_id = Some(app_id),
            handle::Event::OutputEnter { output } => {
                if let Some(id) = output.data::<u32>() {
                    entry.pending.outputs.insert(*id);
                }
                entry.pending.entered_event = true;
            }
            handle::Event::OutputLeave { output } => {
                if let Some(id) = output.data::<u32>() {
                    entry.pending.outputs.remove(id);
                }
                entry.pending.entered_event = false;
            }
            handle::Event::State { state: bytes } => {
                if let Err(error) = entry.pending.apply_state(&bytes) {
                    state.changes.push(Change::Diagnostic(error.into()));
                }
            }
            handle::Event::Parent { parent } => {
                let parent = parent.as_ref().and_then(|proxy| state.find_toplevel(proxy));
                state.toplevels.get_mut(&id).unwrap().pending.parent = parent;
            }
            handle::Event::Done => state.publish(id),
            handle::Event::Closed => state.remove_toplevel(id),
            _ => {}
        }
    }
}
wayland_client::delegate_noop!(State: ignore gamma_manager::ZwlrGammaControlManagerV1);
impl Dispatch<gamma::ZwlrGammaControlV1, u32> for State {
    fn event(
        state: &mut Self,
        proxy: &gamma::ZwlrGammaControlV1,
        event: gamma::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // A disabled/re-enabled controller can still have an old queued event.
        if !state
            .gamma
            .get(id)
            .is_some_and(|entry| entry.proxy == *proxy)
        {
            return;
        }
        match event {
            gamma::Event::GammaSize { size } => {
                let entry = state.gamma.get_mut(id).unwrap();
                entry.size = size as usize;
                if let Some(temperature) = state.temperature
                    && let Err(error) = apply_gamma(entry, temperature)
                {
                    state.changes.push(Change::Diagnostic(error));
                    state.failed_gamma_outputs.insert(*id);
                    state.changes.push(Change::Capabilities);
                    let was_enabled = state.gamma_enabled();
                    if let Some(entry) = state.gamma.remove(id) {
                        entry.proxy.destroy();
                    }
                    if was_enabled != state.gamma_enabled() {
                        state.changes.push(Change::Gamma(state.gamma_enabled()));
                    }
                }
            }
            gamma::Event::Failed => {
                state.failed_gamma_outputs.insert(*id);
                state.changes.push(Change::Capabilities);
                let was_enabled = state.gamma_enabled();
                if let Some(entry) = state.gamma.remove(id) {
                    entry.proxy.destroy();
                }
                state.changes.push(Change::Diagnostic(format!(
                    "Gamma control unavailable for output {id}; another program may own it"
                )));
                if was_enabled != state.gamma_enabled() {
                    state.changes.push(Change::Gamma(state.gamma_enabled()));
                }
            }
            _ => {}
        }
    }
}
