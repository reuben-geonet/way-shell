use super::*;
use std::{
    os::fd::{AsFd, FromRawFd, OwnedFd},
    rc::Rc,
    sync::Arc,
};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1 as ext_device, ext_data_control_manager_v1 as ext_manager,
    ext_data_control_offer_v1 as ext_offer, ext_data_control_source_v1 as ext_source,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1 as keyboard_manager, zwp_virtual_keyboard_v1 as keyboard,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1 as wlr_device, zwlr_data_control_manager_v1 as wlr_manager,
    zwlr_data_control_offer_v1 as wlr_offer, zwlr_data_control_source_v1 as wlr_source,
};

#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "WayShellClipboardOffer")]
pub struct ClipboardOffer {
    pub id: u64,
    pub seat: u32,
    pub mimes: Vec<String>,
}
#[derive(Clone, Debug)]
struct SourceData(Vec<(String, Arc<Vec<u8>>)>);
enum Offer {
    Ext(ext_offer::ExtDataControlOfferV1),
    Wlr(wlr_offer::ZwlrDataControlOfferV1),
}
impl Offer {
    fn destroy(&self) {
        match self {
            Self::Ext(p) => p.destroy(),
            Self::Wlr(p) => p.destroy(),
        }
    }
}
enum Device {
    Ext(ext_device::ExtDataControlDeviceV1),
    Wlr(wlr_device::ZwlrDataControlDeviceV1),
}
impl Device {
    fn destroy(&self) {
        match self {
            Self::Ext(p) => p.destroy(),
            Self::Wlr(p) => p.destroy(),
        }
    }
}
#[derive(Default)]
pub(super) struct ClipboardState {
    ext: Option<(u32, ext_manager::ExtDataControlManagerV1)>,
    wlr: Option<(u32, wlr_manager::ZwlrDataControlManagerV1)>,
    manager: Option<(u32, keyboard_manager::ZwpVirtualKeyboardManagerV1)>,
    devices: BTreeMap<u32, Device>,
    keyboards: BTreeMap<u32, keyboard::ZwpVirtualKeyboardV1>,
    offers: BTreeMap<u64, (u32, Offer, Vec<String>)>,
    current: BTreeMap<u32, u64>,
    next: u64,
    seats: BTreeMap<u32, wl_seat::WlSeat>,
}
impl ClipboardState {
    pub fn current_offers(&self) -> Vec<ClipboardOffer> {
        self.current
            .iter()
            .filter_map(|(&seat, &id)| {
                let (_, _, mimes) = self.offers.get(&id)?;
                Some(ClipboardOffer {
                    id,
                    seat,
                    mimes: mimes.clone(),
                })
            })
            .collect()
    }
    pub fn available(&self) -> bool {
        !self.devices.is_empty()
    }
    pub fn global(
        &mut self,
        name: u32,
        interface: &str,
        version: u32,
        registry: &wl_registry::WlRegistry,
        qh: &QueueHandle<State>,
    ) -> bool {
        match interface {
            "ext_data_control_manager_v1" => {
                self.ext = Some((name, registry.bind(name, version.min(1), qh, ())));
                for (_, device) in std::mem::take(&mut self.devices) {
                    device.destroy();
                }
            }
            "zwlr_data_control_manager_v1" => {
                self.wlr = Some((name, registry.bind(name, version.min(2), qh, ())))
            }
            "zwp_virtual_keyboard_manager_v1" => {
                self.manager = Some((name, registry.bind(name, version.min(1), qh, ())))
            }
            _ => return false,
        }
        true
    }
    pub fn seats(&mut self, seats: &BTreeMap<u32, SeatEntry>, qh: &QueueHandle<State>) {
        for (&id, seat) in seats {
            self.seats.insert(id, seat.proxy.clone());
            if self.devices.contains_key(&id) {
                continue;
            }
            if let Some((_, manager)) = &self.ext {
                self.devices.insert(
                    id,
                    Device::Ext(manager.get_data_device(&seat.proxy, qh, id)),
                );
            } else if let Some((_, manager)) = &self.wlr {
                self.devices.insert(
                    id,
                    Device::Wlr(manager.get_data_device(&seat.proxy, qh, id)),
                );
            }
        }
    }
    pub fn remove(&mut self, id: u32) {
        if let Some(device) = self.devices.remove(&id) {
            device.destroy();
        }
        if let Some(keyboard) = self.keyboards.remove(&id) {
            keyboard.destroy();
        }
        self.seats.remove(&id);
        self.current.remove(&id);
        let ids = self
            .offers
            .iter()
            .filter_map(|(key, (seat, _, _))| (*seat == id).then_some(*key))
            .collect::<Vec<_>>();
        for key in ids {
            if let Some((_, offer, _)) = self.offers.remove(&key) {
                offer.destroy();
            }
        }
        if self.ext.as_ref().is_some_and(|(name, _)| *name == id) {
            self.ext.take();
        }
        if self.wlr.as_ref().is_some_and(|(name, _)| *name == id) {
            self.wlr.take();
        }
        if self.manager.as_ref().is_some_and(|(name, _)| *name == id) {
            self.manager.take();
        }
    }
    pub fn read(&self, id: u64, mime: &str) -> Result<OwnedFd, String> {
        let (_, offer, mimes) = self.offers.get(&id).ok_or("Clipboard offer expired")?;
        if !mimes.iter().any(|value| value == mime) {
            return Err("Clipboard format unavailable".into());
        }
        let mut fds = [-1; 2];
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
        match offer {
            Offer::Ext(p) => p.receive(mime.to_owned(), write.as_fd()),
            Offer::Wlr(p) => p.receive(mime.to_owned(), write.as_fd()),
        }
        Ok(read)
    }
    pub fn set(
        &self,
        formats: Vec<(String, Vec<u8>)>,
        primary: bool,
        qh: &QueueHandle<State>,
    ) -> Result<(), String> {
        let (&seat, device) = self
            .devices
            .iter()
            .next()
            .ok_or("Clipboard management is unavailable")?;
        let _ = seat;
        let mut data = SourceData(
            formats
                .into_iter()
                .map(|(mime, bytes)| (mime, Arc::new(bytes)))
                .collect(),
        );
        data.0.push((
            "application/x-way-shell-clipboard".into(),
            Arc::new(Vec::new()),
        ));
        match device {
            Device::Ext(device) => {
                let manager = &self.ext.as_ref().ok_or("Clipboard manager disappeared")?.1;
                let source = manager.create_data_source(qh, data.clone());
                for (mime, _) in &data.0 {
                    source.offer(mime.clone());
                }
                device.set_selection(Some(&source));
                if primary {
                    let source = manager.create_data_source(qh, data.clone());
                    for (mime, _) in &data.0 {
                        source.offer(mime.clone());
                    }
                    device.set_primary_selection(Some(&source));
                }
            }
            Device::Wlr(device) => {
                let manager = &self.wlr.as_ref().ok_or("Clipboard manager disappeared")?.1;
                let source = manager.create_data_source(qh, data.clone());
                for (mime, _) in &data.0 {
                    source.offer(mime.clone());
                }
                device.set_selection(Some(&source));
                if primary && device.version() >= 2 {
                    let source = manager.create_data_source(qh, data.clone());
                    for (mime, _) in &data.0 {
                        source.offer(mime.clone());
                    }
                    device.set_primary_selection(Some(&source));
                }
            }
        }
        Ok(())
    }
    pub fn paste(&mut self, terminal: bool, qh: &QueueHandle<State>) -> Result<(), String> {
        let (&seat, proxy) = self
            .seats
            .iter()
            .next()
            .ok_or("No keyboard seat is available")?;
        if !self.keyboards.contains_key(&seat) {
            let manager = &self
                .manager
                .as_ref()
                .ok_or("The compositor does not support automatic paste")?
                .1;
            let keyboard = manager.create_virtual_keyboard(proxy, qh, ());
            let raw = unsafe { libc::memfd_create(c"way-shell-paste".as_ptr(), libc::MFD_CLOEXEC) };
            if raw < 0 {
                keyboard.destroy();
                return Err(std::io::Error::last_os_error().to_string());
            }
            let mut fd = unsafe { std::fs::File::from_raw_fd(raw) };
            let keymap=b"xkb_keymap { xkb_keycodes { include \"evdev+aliases(qwerty)\" }; xkb_types { include \"complete\" }; xkb_compatibility { include \"complete\" }; xkb_symbols { include \"pc+us\" }; };\0";
            fd.write_all(keymap).map_err(|e| e.to_string())?;
            keyboard.keymap(1, fd.as_fd(), keymap.len() as u32);
            self.keyboards.insert(seat, keyboard);
        }
        let keyboard = &self.keyboards[&seat];
        keyboard.modifiers(if terminal { 5 } else { 4 }, 0, 0, 0);
        keyboard.key(0, 47, 1);
        keyboard.key(0, 47, 0);
        keyboard.modifiers(0, 0, 0, 0);
        Ok(())
    }
}

macro_rules! protocol {
    ($manager:ident,$device:ident,$offer:ident,$source:ident,$manager_type:ident,$device_type:ident,$offer_type:ident,$source_type:ident,$variant:ident) => {
        wayland_client::delegate_noop!(State: ignore $manager::$manager_type);
        impl Dispatch<$device::$device_type,u32> for State {
            fn event(state:&mut Self,_:&$device::$device_type,event:$device::Event,seat:&u32,_:&Connection,_:&QueueHandle<Self>) {
                match event {
                    $device::Event::DataOffer {id}=>{state.clipboard.next=state.clipboard.next.wrapping_add(1);state.clipboard.offers.insert(state.clipboard.next,(*seat,Offer::$variant(id),Vec::new()));},
                    $device::Event::Selection {id}=>{
                        let key=id.as_ref().and_then(|id|state.clipboard.offers.iter().find_map(|(key,(_,offer,_))|matches!(offer,Offer::$variant(p) if p==id).then_some(*key)));
                        if let Some(old)=state.clipboard.current.remove(seat).filter(|old|Some(*old)!=key) && let Some((_,offer,_))=state.clipboard.offers.remove(&old) {offer.destroy();}
                        if let Some(key)=key {state.clipboard.current.insert(*seat,key);}
                        let mimes=key.and_then(|key|state.clipboard.offers.get(&key)).map(|(_,_,mimes)|mimes.clone()).unwrap_or_default();
                        state.changes.push(Change::Clipboard(ClipboardOffer{id:key.unwrap_or(0),seat:*seat,mimes}));
                    },
                    $device::Event::PrimarySelection {id:Some(id)}=>{let key=state.clipboard.offers.iter().find_map(|(key,(_,offer,_))|matches!(offer,Offer::$variant(p) if *p==id).then_some(*key));if let Some(key)=key && !state.clipboard.current.values().any(|id|*id==key) && let Some((_,offer,_))=state.clipboard.offers.remove(&key){offer.destroy();}},
                    $device::Event::Finished=>{state.clipboard.remove(*seat);state.changes.push(Change::Capabilities);},
                    _=>{},
                }
            }
            wayland_client::event_created_child!(State,$device::$device_type,[0=>($offer::$offer_type,())]);
        }
        impl Dispatch<$offer::$offer_type,()> for State {
            fn event(state:&mut Self,proxy:&$offer::$offer_type,event:$offer::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {
                if let $offer::Event::Offer {mime_type}=event && let Some((_,_,mimes))=state.clipboard.offers.values_mut().find(|(_,offer,_)|matches!(offer,Offer::$variant(p) if p==proxy)) {mimes.push(mime_type);}
            }
        }
        impl Dispatch<$source::$source_type,SourceData> for State {
            fn event(state:&mut Self,proxy:&$source::$source_type,event:$source::Event,data:&SourceData,_:&Connection,_:&QueueHandle<Self>) {
                match event {
                    $source::Event::Send {mime_type,fd}=>{if let Some((_,bytes))=data.0.iter().find(|(mime,_)|*mime==mime_type){state.changes.push(Change::ClipboardWrite(Rc::new(fd),bytes.clone()));}},
                    $source::Event::Cancelled=>proxy.destroy(),
                    _=>{},
                }
            }
        }
    }
}
protocol!(
    ext_manager,
    ext_device,
    ext_offer,
    ext_source,
    ExtDataControlManagerV1,
    ExtDataControlDeviceV1,
    ExtDataControlOfferV1,
    ExtDataControlSourceV1,
    Ext
);
protocol!(
    wlr_manager,
    wlr_device,
    wlr_offer,
    wlr_source,
    ZwlrDataControlManagerV1,
    ZwlrDataControlDeviceV1,
    ZwlrDataControlOfferV1,
    ZwlrDataControlSourceV1,
    Wlr
);
wayland_client::delegate_noop!(State: ignore keyboard_manager::ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(State: ignore keyboard::ZwpVirtualKeyboardV1);
