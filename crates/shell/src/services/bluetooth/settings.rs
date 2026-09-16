//! An executable and quoted arguments, never a shell command.
use std::ffi::OsString;
pub fn parse_command(command: &str) -> Result<Vec<OsString>, glib::Error> {
    let mut argv = glib::shell_parse_argv(command)?;
    let executable = glib::find_program_in_path(&argv[0]).ok_or_else(|| {
        glib::Error::new(
            gio::IOErrorEnum::NotFound,
            &format!(
                "Cannot find {}. Install it or change the Bluetooth settings command.",
                argv[0].to_string_lossy()
            ),
        )
    })?;
    argv[0] = executable.into_os_string();
    Ok(argv)
}
pub fn launch(command: &str) -> Result<(), glib::Error> {
    let argv = parse_command(command)?;
    gio::Subprocess::newv(
        &argv.iter().map(AsRef::as_ref).collect::<Vec<_>>(),
        gio::SubprocessFlags::NONE,
    )?;
    Ok(())
}
