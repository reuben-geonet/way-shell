//! Display-independent Way Shell contracts and domain logic.

pub mod audio;
pub mod clock;
pub mod gamma;
pub mod ipc;
mod whitepoints;

/// Stable socket filename under the session's XDG_RUNTIME_DIR.
pub const IPC_SOCKET_NAME: &str = "way-shell.sock";
