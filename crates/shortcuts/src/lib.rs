//! Saved compositor bindings. No compositor connection, UI, or ambient environment.
mod describe;
mod niri;
pub mod sway;

pub use describe::{Description, Group, describe_native};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
};

pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_DEPTH: usize = 32;
pub const MAX_BINDINGS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compositor {
    Sway,
    Niri,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Session {
    Desktop,
    Nested,
    #[default]
    Unknown,
}
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub home: Option<PathBuf>,
    pub environment: BTreeMap<String, String>,
    pub session: Session,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub path: PathBuf,
    pub line: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub source: Source,
    pub severity: Severity,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Native {
        name: String,
        arguments: Vec<String>,
    },
    Spawn(Vec<String>),
    Shell(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub trigger: String,
    pub mode: String,
    pub conditions: Vec<String>,
    pub actions: Vec<Action>,
    pub original: String,
    pub source: Source,
    /// None means the compositor's default title; Some(None) explicitly hides it.
    pub title: Option<Option<String>>,
    pub physical: bool,
    identity: Vec<String>,
}
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub bindings: Vec<Binding>,
    pub files: BTreeSet<PathBuf>,
    pub directories: BTreeSet<PathBuf>,
    pub diagnostics: Vec<Diagnostic>,
    pub mod_legend: bool,
    pub root_readable: bool,
}
impl Snapshot {
    pub fn complete(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
}

struct Loader<'a> {
    context: &'a Context,
    compositor: Compositor,
    snapshot: Snapshot,
    variables: BTreeMap<String, String>,
    visited: BTreeSet<PathBuf>,
    stack: BTreeSet<PathBuf>,
    total: usize,
    read_files: bool,
    definitions: usize,
    expanded: usize,
    mod_key: String,
    mod_nested: String,
}
impl<'a> Loader<'a> {
    fn new(compositor: Compositor, context: &'a Context) -> Self {
        Self {
            context,
            compositor,
            snapshot: Snapshot::default(),
            variables: BTreeMap::new(),
            visited: BTreeSet::new(),
            stack: BTreeSet::new(),
            total: 0,
            read_files: true,
            definitions: 0,
            expanded: 0,
            mod_key: "Super".into(),
            mod_nested: "Alt".into(),
        }
    }
    fn diagnostic(
        &mut self,
        path: &Path,
        line: usize,
        severity: Severity,
        message: impl Into<String>,
    ) {
        if self.snapshot.diagnostics.len() >= 256 {
            return;
        }
        self.snapshot.diagnostics.push(Diagnostic {
            source: Source {
                path: path.into(),
                line,
            },
            severity,
            message: message.into(),
        });
    }
    fn insert(&mut self, binding: Binding) {
        self.definitions += 1;
        if self.definitions > MAX_BINDINGS {
            if self.definitions == MAX_BINDINGS + 1 {
                self.diagnostic(
                    &binding.source.path,
                    binding.source.line,
                    Severity::Error,
                    "Binding limit exceeded (4096 definitions)",
                );
            }
            return;
        }
        self.snapshot.bindings.retain(|b| {
            !(b.trigger == binding.trigger
                && b.mode == binding.mode
                && b.physical == binding.physical
                && b.identity == binding.identity)
        });
        self.snapshot.bindings.push(binding);
    }
    fn watch_path(&mut self, path: &Path) {
        self.snapshot.files.insert(path.into());
        if let Some(parent) = path.parent() {
            self.snapshot.directories.insert(parent.into());
        }
    }
    fn file(&mut self, path: &Path, depth: usize, optional: bool, mode: &str) {
        self.watch_path(path);
        if depth > MAX_DEPTH || self.total >= MAX_TOTAL_BYTES || self.snapshot.files.len() > 1024 {
            self.diagnostic(
                path,
                1,
                Severity::Error,
                "Configuration traversal limit exceeded",
            );
            return;
        }
        let canonical = path.canonicalize().unwrap_or_else(|_| path.into());
        if canonical != path {
            self.watch_path(&canonical);
        }
        if self.stack.contains(&canonical) {
            self.diagnostic(
                path,
                1,
                if self.compositor == Compositor::Niri {
                    Severity::Error
                } else {
                    Severity::Warning
                },
                "Cyclic include skipped",
            );
            return;
        }
        if self.compositor == Compositor::Sway && !self.visited.insert(canonical.clone()) {
            return;
        }
        let read = || -> std::io::Result<String> {
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take((MAX_FILE_BYTES + 1) as u64)
                .read_to_end(&mut bytes)?;
            if bytes.len() > MAX_FILE_BYTES {
                return Err(std::io::Error::other("Configuration file exceeds 2 MiB"));
            }
            String::from_utf8(bytes).map_err(std::io::Error::other)
        };
        match read() {
            Ok(text) => {
                self.total += text.len();
                if self.total > MAX_TOTAL_BYTES {
                    self.diagnostic(
                        path,
                        1,
                        Severity::Error,
                        "Accumulated configuration exceeds 8 MiB",
                    );
                    return;
                }
                if depth == 0 {
                    self.snapshot.root_readable = true;
                }
                self.stack.insert(canonical.clone());
                self.text(path, &text, depth, mode);
                self.stack.remove(&canonical);
            }
            Err(error) => self.diagnostic(
                path,
                1,
                if optional && error.kind() == std::io::ErrorKind::NotFound {
                    Severity::Warning
                } else {
                    Severity::Error
                },
                format!("Cannot read configuration: {error}"),
            ),
        }
    }
    fn text(&mut self, path: &Path, text: &str, depth: usize, mode: &str) {
        if text.len() > MAX_FILE_BYTES {
            self.diagnostic(path, 1, Severity::Error, "Configuration file exceeds 2 MiB");
            return;
        }
        match self.compositor {
            Compositor::Sway => sway::parse(self, path, text, depth, mode),
            Compositor::Niri => niri::parse(self, path, text, depth),
        }
    }
    fn include(
        &mut self,
        from: &Path,
        line: usize,
        pattern: &str,
        depth: usize,
        optional: bool,
        mode: &str,
    ) {
        let expanded = if self.compositor == Compositor::Sway {
            match self.expand(pattern) {
                Ok(value) => value,
                Err(error) => {
                    self.diagnostic(from, line, Severity::Error, error);
                    return;
                }
            }
        } else {
            pattern.into()
        };
        let path = expand_home(&expanded, self.context.home.as_deref());
        let path = if path.is_absolute() {
            path
        } else {
            from.parent().unwrap_or(Path::new(".")).join(path)
        };
        if !self.read_files {
            self.watch_path(&path);
            return;
        }
        if self.compositor == Compositor::Niri {
            self.file(&path, depth + 1, optional, mode);
            return;
        }
        // Watch the literal prefix as well as matched parents: new glob matches matter.
        let prefix: PathBuf = path
            .components()
            .take_while(|part| !part.as_os_str().to_string_lossy().contains(['*', '?', '[']))
            .collect();
        self.snapshot.directories.insert(if prefix == path {
            path.parent().unwrap_or(Path::new("/")).into()
        } else {
            prefix
        });
        for pattern in path.ancestors().skip(1).take(MAX_DEPTH) {
            if !pattern.to_string_lossy().contains(['*', '?', '[']) {
                break;
            }
            if let Ok(matches) = glob::glob(&pattern.to_string_lossy()) {
                for directory in matches.take(1024).flatten().filter(|p| p.is_dir()) {
                    if self.snapshot.directories.len() >= 1024 {
                        self.diagnostic(
                            from,
                            line,
                            Severity::Warning,
                            "Include directory watch limit reached",
                        );
                        break;
                    }
                    self.snapshot.directories.insert(directory);
                }
            }
        }
        match glob::glob(&path.to_string_lossy()) {
            Ok(paths) => {
                let paths: Vec<_> = paths.take(1025).collect();
                if paths.len() > 1024 {
                    self.diagnostic(from, line, Severity::Error, "Include match limit exceeded");
                    return;
                }
                if paths.is_empty() {
                    self.watch_path(&path);
                }
                for entry in paths {
                    match entry {
                        Ok(path) => self.file(&path, depth + 1, false, mode),
                        Err(error) => {
                            self.diagnostic(from, line, Severity::Error, error.to_string())
                        }
                    }
                }
            }
            Err(error) => self.diagnostic(
                from,
                line,
                Severity::Error,
                format!("Invalid include pattern: {error}"),
            ),
        }
    }
    fn expand(&mut self, text: &str) -> Result<String, String> {
        let mut out = String::new();
        let mut rest = text;
        while let Some(index) = rest.find('$') {
            if out.len() > MAX_FILE_BYTES {
                return Err("Variable expansion limit exceeded".into());
            }
            out.push_str(&rest[..index]);
            rest = &rest[index + 1..];
            if let Some(after) = rest.strip_prefix('{')
                && let Some(end) = after.find('}')
            {
                let name = &after[..end];
                if let Some(value) = self
                    .variables
                    .get(name)
                    .or_else(|| self.context.environment.get(name))
                {
                    out.push_str(value);
                } else {
                    out.push_str("${");
                    out.push_str(name);
                    out.push('}');
                }
                rest = &after[end + 1..];
                continue;
            }
            if let Some((name, value)) = self
                .variables
                .iter()
                .filter(|(name, _)| rest.starts_with(name.as_str()))
                .max_by_key(|(name, _)| name.len())
            {
                out.push_str(value);
                rest = &rest[name.len()..];
                continue;
            }
            let end = rest
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            let name = &rest[..end];
            if let Some(value) = self
                .variables
                .get(name)
                .or_else(|| self.context.environment.get(name))
            {
                out.push_str(value);
            } else {
                out.push('$');
                out.push_str(name);
            }
            rest = &rest[end..];
        }
        out.push_str(rest);
        self.expanded = self.expanded.saturating_add(out.len());
        if out.len() > MAX_FILE_BYTES || self.expanded > MAX_TOTAL_BYTES {
            return Err("Variable expansion limit exceeded".into());
        }
        Ok(out)
    }
    fn finish(mut self) -> Snapshot {
        if self.compositor == Compositor::Niri {
            let modifier = match self.context.session {
                Session::Desktop => Some(&self.mod_key),
                Session::Nested => Some(&self.mod_nested),
                Session::Unknown => None,
            };
            for binding in &mut self.snapshot.bindings {
                if binding.trigger.split('+').any(|key| key == "Mod") {
                    if let Some(modifier) = modifier {
                        binding.trigger = normalize_keys(
                            &binding.trigger.replace("Mod+", &format!("{modifier}+")),
                            false,
                        );
                    } else {
                        self.snapshot.mod_legend = true;
                    }
                }
            }
        }
        self.snapshot
    }
}

pub fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = home
    {
        home.join(rest)
    } else {
        path.into()
    }
}
pub fn load(compositor: Compositor, path: &Path, context: &Context) -> Snapshot {
    let mut loader = Loader::new(compositor, context);
    loader.file(path, 0, false, "default");
    loader.finish()
}
/// Parse supplied text without reading includes. Includes are reported as dependencies.
pub fn parse_text(compositor: Compositor, path: &Path, text: &str, context: &Context) -> Snapshot {
    let mut loader = Loader::new(compositor, context);
    // Text entry points deliberately never read the host filesystem.
    loader.read_files = false;
    loader.snapshot.root_readable = true;
    loader.text(path, text, 0, "default");
    loader.finish()
}
pub fn normalize_keys(trigger: &str, physical: bool) -> String {
    let mut keys: Vec<String> = trigger
        .split('+')
        .map(|key| match key.to_ascii_lowercase().as_str() {
            "control" | "ctrl" => "Ctrl".into(),
            "mod4" | "super" | "logo" | "win" => "Super".into(),
            "mod1" | "alt" => "Alt".into(),
            "shift" => "Shift".into(),
            "mod" => "Mod".into(),
            "return" | "enter" => "Enter".into(),
            "space" => "Space".into(),
            "escape" | "esc" => "Escape".into(),
            "left" => "←".into(),
            "right" => "→".into(),
            "up" => "↑".into(),
            "down" => "↓".into(),
            "tab" => "Tab".into(),
            "grave" => "`".into(),
            "xf86audioraisevolume" => "Volume up".into(),
            "xf86audiolowervolume" => "Volume down".into(),
            "xf86audiomute" => "Mute".into(),
            "xf86audioplay" => "Play/pause".into(),
            "xf86audionext" => "Next track".into(),
            "xf86audioprev" => "Previous track".into(),
            "xf86monbrightnessup" => "Brightness up".into(),
            "xf86monbrightnessdown" => "Brightness down".into(),
            _ if physical && key.chars().all(|c| c.is_ascii_digit()) => format!("Keycode {key}"),
            _ if key.len() == 1 => key.to_uppercase(),
            _ => key.into(),
        })
        .collect();
    if keys.len() > 1 {
        let last = keys.pop().unwrap();
        keys.sort();
        keys.dedup();
        keys.push(last);
    }
    keys.join("+")
}
