/// Look up schemas explicitly so a missing installation produces a diagnostic
/// instead of GSettings terminating the process.
pub fn open(id: &str) -> Result<gio::Settings, glib::BoolError> {
    let source = gio::SettingsSchemaSource::default()
        .ok_or_else(|| glib::bool_error!("No GSettings schema source is installed"))?;
    let schema = source
        .lookup(id, true)
        .ok_or_else(|| glib::bool_error!("Missing GSettings schema {id}"))?;
    Ok(gio::Settings::new_full(
        &schema,
        gio::SettingsBackend::NONE,
        None,
    ))
}
