//! Only compositor startup metadata needed to locate saved configuration.
use super::*;
use std::io::Read;

#[derive(Clone, Debug, Default)]
pub struct ProcessContext {
    pub args: Vec<String>,
    pub environment: std::collections::BTreeMap<String, String>,
    pub cwd: Option<PathBuf>,
    pub environment_available: bool,
}
#[derive(Clone, Debug)]
pub struct Discovery {
    pub path: Option<PathBuf>,
    pub context: Context,
    pub diagnostics: Vec<String>,
    pub dependencies: Vec<PathBuf>,
}

pub fn niri_path(
    override_path: &str,
    process: Option<&ProcessContext>,
    ambient: &Context,
) -> Discovery {
    let mut context = ambient.clone();
    let mut diagnostics = Vec::new();
    if let Some(process) = process.filter(|process| process.environment_available) {
        for key in [
            "NIRI_CONFIG",
            "HOME",
            "XDG_CONFIG_HOME",
            "WAYLAND_DISPLAY",
            "DISPLAY",
        ] {
            context.environment.remove(key);
        }
        context.environment.extend(process.environment.clone());
        if let Some(home) = process.environment.get("HOME") {
            context.home = Some(home.into());
        }
        context.session = if ["WAYLAND_DISPLAY", "DISPLAY"].iter().any(|key| {
            process
                .environment
                .get(*key)
                .is_some_and(|value| !value.is_empty())
        }) {
            Session::Nested
        } else {
            Session::Desktop
        };
    } else {
        diagnostics.push("Compositor startup metadata is unavailable. Set niri-config-path if the discovered file is incorrect; Mod is left unresolved.".into());
    }
    let mut result = Discovery {
        path: None,
        context,
        diagnostics,
        dependencies: Vec::new(),
    };
    if !override_path.is_empty() {
        result.path = explicit(override_path, ambient, &mut result.diagnostics);
        return result;
    }
    let startup = process.and_then(|process| {
        process
            .args
            .iter()
            .enumerate()
            .skip(1)
            .find_map(|(index, arg)| {
                if arg == "--config" || arg == "-c" {
                    Some(process.args.get(index + 1).cloned().unwrap_or_default())
                } else {
                    arg.strip_prefix("--config=")
                        .or_else(|| arg.strip_prefix("-c").filter(|s| !s.is_empty()))
                        .map(str::to_owned)
                }
            })
    });
    let selected = startup.or_else(|| {
        result
            .context
            .environment
            .get("NIRI_CONFIG")
            .filter(|s| !s.is_empty())
            .cloned()
    });
    if let Some(selected) = selected {
        let path = PathBuf::from(selected);
        result.path = if path.is_absolute() {
            Some(path)
        } else if let Some(cwd) = process.and_then(|p| p.cwd.as_ref()) {
            Some(cwd.join(path))
        } else {
            result
                .diagnostics
                .push("Cannot resolve a relative Niri startup path; set niri-config-path.".into());
            None
        };
    } else {
        let config = result
            .context
            .environment
            .get("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| result.context.home.as_ref().map(|h| h.join(".config")));
        let system = PathBuf::from("/etc/niri/config.kdl");
        if let Some(user) = config.map(|p| p.join("niri/config.kdl")) {
            result.dependencies.extend([user.clone(), system.clone()]);
            result.path = Some(if user.exists() || !system.exists() {
                user
            } else {
                system
            });
        } else {
            result.path = Some(system);
        }
    }
    result
}
pub(super) fn explicit(
    value: &str,
    context: &Context,
    diagnostics: &mut Vec<String>,
) -> Option<PathBuf> {
    let path = way_shell_shortcuts::expand_home(value, context.home.as_deref());
    if path.is_absolute() {
        Some(path)
    } else {
        diagnostics
            .push("Configuration override must be an absolute path or start with ~/.".into());
        None
    }
}

pub(super) fn process_context(socket: &Path) -> Option<ProcessContext> {
    let client = gio::SocketClient::new();
    client.set_timeout(1);
    let connection = gio::prelude::SocketClientExt::connect(
        &client,
        &gio::UnixSocketAddress::new(socket),
        gio::Cancellable::NONE,
    )
    .ok()?;
    let pid = connection.socket().credentials().ok()?.unix_pid().ok()?;
    let base = PathBuf::from(format!("/proc/{pid}"));
    let read = |name| -> Option<Vec<u8>> {
        let mut data = Vec::new();
        std::fs::File::open(base.join(name))
            .ok()?
            .take(256 * 1024)
            .read_to_end(&mut data)
            .ok()?;
        Some(data)
    };
    let command_line = read("cmdline");
    let process_environment = read("environ");
    if command_line.is_none() && process_environment.is_none() {
        return None;
    }
    let environment_available = process_environment.is_some();
    let args = command_line
        .unwrap_or_default()
        .split(|b| *b == 0)
        .filter(|b| !b.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    // Keep only relevant variables; never publish or log the process environment.
    let environment = process_environment
        .unwrap_or_default()
        .split(|b| *b == 0)
        .filter_map(|bytes| {
            let value = std::str::from_utf8(bytes).ok()?;
            let (key, value) = value.split_once('=')?;
            matches!(
                key,
                "NIRI_CONFIG" | "HOME" | "XDG_CONFIG_HOME" | "WAYLAND_DISPLAY" | "DISPLAY"
            )
            .then(|| (key.into(), value.into()))
        })
        .collect();
    Some(ProcessContext {
        args,
        environment,
        cwd: std::fs::read_link(base.join("cwd")).ok(),
        environment_available,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precedence_and_relative_startup_paths() {
        let ambient = Context {
            home: Some("/home/test".into()),
            ..Default::default()
        };
        let process = ProcessContext {
            args: vec!["niri".into(), "--config".into(), "custom.kdl".into()],
            cwd: Some("/work".into()),
            environment_available: true,
            environment: [("NIRI_CONFIG".into(), "/ignored".into())].into(),
        };
        assert_eq!(
            niri_path("", Some(&process), &ambient).path,
            Some("/work/custom.kdl".into())
        );
        assert_eq!(
            niri_path("~/explicit", Some(&process), &ambient).path,
            Some("/home/test/explicit".into())
        );
        assert!(
            niri_path("relative", Some(&process), &ambient)
                .path
                .is_none()
        );
        let mut partial = process.clone();
        partial.environment_available = false;
        let result = niri_path("", Some(&partial), &ambient);
        assert_eq!(result.path, Some("/work/custom.kdl".into()));
        assert_eq!(result.context.session, Session::Unknown);
        let mut process = process;
        process.args = vec!["niri".into()];
        assert_eq!(
            niri_path("", Some(&process), &ambient).path,
            Some("/ignored".into())
        );
        let result = niri_path("", None, &ambient);
        assert!(!result.diagnostics.is_empty());
        assert_eq!(result.context.session, Session::Unknown);
    }
}
