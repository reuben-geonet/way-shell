//! Display-independent Way Shell contracts and domain logic.

pub mod audio;
pub mod clock;
pub mod commands;
pub mod gamma;
pub mod ipc;
pub mod notifications;
pub mod sway;
mod whitepoints;
pub mod wm;

/// Stable socket filename under the session's XDG_RUNTIME_DIR.
pub const IPC_SOCKET_NAME: &str = "way-shell.sock";

pub mod niri;
