//! Rust services and widgets introduced alongside the existing shell.

pub mod platform;
pub mod resources;
pub mod services;

/// Keep GApplication and desktop integrations on their existing identity.
pub const APPLICATION_ID: &str = "org.ldelossa.way-shell";
