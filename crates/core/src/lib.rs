//! Display-independent Way Shell contracts and domain logic.

pub mod ipc;

/// Stable socket filename under the session's XDG_RUNTIME_DIR.
pub const IPC_SOCKET_NAME: &str = "way-shell.sock";
