//! Way Shell's application, services and GTK interface on one GLib main loop.

pub mod application;
pub mod platform;
pub mod resources;
pub mod services;
pub mod ui;

/// Keep GApplication and desktop integrations on their existing identity.
pub const APPLICATION_ID: &str = "org.ldelossa.way-shell";
