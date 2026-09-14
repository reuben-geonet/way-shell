//! The command-line client, independent of GTK and desktop services.

/// All client transactions must finish within this interval.
pub const RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
