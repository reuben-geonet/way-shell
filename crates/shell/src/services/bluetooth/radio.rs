//! Linux rfkill's eight-byte v1 event ABI; the descriptor is always owned.
use super::*;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
};
#[derive(Clone, Copy)]
pub(super) struct Event {
    pub soft: bool,
    pub hard: bool,
}
pub(super) struct Radio {
    file: File,
    source: glib::Source,
}
impl Drop for Radio {
    fn drop(&mut self) {
        self.source.destroy();
    }
}
pub(super) fn open() -> Option<OwnedFd> {
    // O_NONBLOCK on Linux. Rust's OpenOptions supplies O_CLOEXEC.
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(0x800)
        .open("/dev/rfkill")
        .or_else(|_| {
            OpenOptions::new()
                .read(true)
                .custom_flags(0x800)
                .open("/dev/rfkill")
        })
        .ok()
        .map(Into::into)
}
impl BluetoothService {
    pub(super) fn attach_radio(&self, fd: Option<OwnedFd>) {
        let Some(fd) = fd else { return };
        let weak: glib::SendWeakRef<Self> = self.downgrade().into();
        let source = glib::source::unix_fd_source_new(
            fd.as_raw_fd(),
            glib::IOCondition::IN | glib::IOCondition::HUP | glib::IOCondition::ERR,
            Some("bluetooth-rfkill"),
            glib::Priority::DEFAULT,
            move |_, condition| {
                let Some(s) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                s.read_radios();
                if condition.intersects(
                    glib::IOCondition::HUP | glib::IOCondition::ERR | glib::IOCondition::NVAL,
                ) {
                    s.imp().radio.borrow_mut().take();
                    s.imp().radios.borrow_mut().clear();
                    s.prune();
                    s.changed();
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            },
        );
        source.attach(Some(&glib::MainContext::ref_thread_default()));
        self.imp().radio.replace(Some(Radio {
            file: File::from(fd),
            source,
        }));
        self.read_radios();
    }
    fn read_radios(&self) {
        loop {
            let mut bytes = [0u8; 8];
            let result = self
                .imp()
                .radio
                .borrow_mut()
                .as_mut()
                .map(|r| r.file.read(&mut bytes));
            match result {
                Some(Ok(8)) => {
                    if bytes[4] != 2 {
                        continue;
                    }
                    let id = u32::from_ne_bytes(bytes[..4].try_into().unwrap());
                    if self.imp().airplane.get() && !self.busy() && bytes[5] == 2 && bytes[6] == 0 {
                        self.imp().airplane_override.set(true);
                    }
                    if bytes[5] == 1 {
                        self.imp().radios.borrow_mut().remove(&id);
                    } else {
                        self.imp().radios.borrow_mut().insert(
                            id,
                            Event {
                                soft: bytes[6] != 0,
                                hard: bytes[7] != 0,
                            },
                        );
                    }
                }
                Some(Err(e)) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                _ => break,
            }
        }
        self.prune();
        self.changed();
    }
    pub(super) fn write_radio(&self, operation: u8, index: u32, blocked: bool) -> bool {
        let mut bytes = [0u8; 8];
        bytes[..4].copy_from_slice(&index.to_ne_bytes());
        bytes[4] = 2;
        bytes[5] = operation;
        bytes[6] = u8::from(blocked);
        let result = loop {
            let result = self
                .imp()
                .radio
                .borrow_mut()
                .as_mut()
                .map(|r| r.file.write(&bytes));
            if matches!(&result, Some(Err(e)) if e.kind() == std::io::ErrorKind::Interrupted) {
                continue;
            }
            break result;
        };
        if matches!(result, Some(Ok(8))) {
            true
        } else {
            self.error("Cannot change the Bluetooth radio block. Check your session's access to /dev/rfkill.");
            false
        }
    }
}
