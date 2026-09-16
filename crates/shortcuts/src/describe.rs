use crate::Action;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    WayShell,
    Windows,
    Workspaces,
    Session,
}
impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Self::WayShell => "Way-Shell",
            Self::Windows => "Windows and layout",
            Self::Workspaces => "Workspaces and outputs",
            Self::Session => "Session and compositor",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Description {
    pub group: Group,
    pub text: String,
}

/// Unknown native actions stay visible and are explicitly identified as such.
/// Executable actions are left to the host's command catalog.
pub fn describe_native(action: &Action) -> Option<Description> {
    let Action::Native { name, arguments } = action else {
        return None;
    };
    let args = arguments.join(" ");
    let group = if name.contains("workspace")
        || name.contains("output")
        || name.contains("monitor")
        || name == "workspace"
        || args.contains("workspace")
        || args.contains("output")
    {
        Group::Workspaces
    } else if matches!(
        name.as_str(),
        "reload"
            | "exit"
            | "quit"
            | "suspend"
            | "open-overview"
            | "close-overview"
            | "mode"
            | "load-config-file"
            | "toggle-overview"
            | "show-hotkey-overlay"
            | "power-off-monitors"
            | "power-on-monitors"
    ) || name.starts_with("screenshot")
    {
        Group::Session
    } else {
        Group::Windows
    };
    let label = match name.as_str() {
        "kill" | "close-window" => "Close window".into(),
        "reload" | "load-config-file" => "Reload configuration".into(),
        "suspend" => "Suspend the session".into(),
        "open-overview" => "Open overview".into(),
        "close-overview" => "Close overview".into(),
        "exit" | "quit" => "End compositor session".into(),
        "mode" => "Enter binding mode".into(),
        "workspace" => "Switch to workspace".into(),
        "focus" => "Focus".into(),
        "move" => "Move".into(),
        "resize" => "Resize".into(),
        "layout" => "Set layout".into(),
        "split" => "Split".into(),
        "splith" => "Split horizontally".into(),
        "splitv" => "Split vertically".into(),
        "floating" => "Floating window".into(),
        "fullscreen" => "Fullscreen".into(),
        "scratchpad" => "Scratchpad".into(),
        "sticky" => "Show window on all workspaces".into(),
        "toggle-overview" => "Toggle overview".into(),
        "show-hotkey-overlay" => "Show compositor shortcuts".into(),
        "screenshot" => "Take a screenshot".into(),
        "screenshot-window" => "Screenshot window".into(),
        "screenshot-screen" => "Screenshot screen".into(),
        "power-off-monitors" => "Turn off displays".into(),
        "power-on-monitors" => "Turn on displays".into(),
        "fullscreen-window" => "Toggle window fullscreen".into(),
        "toggle-window-floating" => "Toggle floating window".into(),
        "maximize-column" => "Maximize column".into(),
        "expand-column-to-available-width" => "Expand column to available width".into(),
        name if KNOWN_NIRI_ACTIONS.contains(&name) => {
            let words = name.replace('-', " ");
            let mut chars = words.chars();
            format!("{}{}", chars.next().unwrap().to_uppercase(), chars.as_str())
        }
        _ => format!("Compositor action: {name}"),
    };
    Some(Description {
        group,
        text: if args.is_empty() {
            label
        } else {
            format!("{label} {args}")
        },
    })
}

// Explicit names prevent unknown compositor actions from acquiring invented descriptions.
const KNOWN_NIRI_ACTIONS: &[&str] = &[
    "focus-window-in-column",
    "focus-window-previous",
    "focus-column-left",
    "focus-column-right",
    "focus-column-first",
    "focus-column-last",
    "focus-column-right-or-first",
    "focus-column-left-or-last",
    "focus-column",
    "focus-window-or-monitor-up",
    "focus-window-or-monitor-down",
    "focus-column-or-monitor-left",
    "focus-column-or-monitor-right",
    "focus-window-down",
    "focus-window-up",
    "focus-window-down-or-column-left",
    "focus-window-down-or-column-right",
    "focus-window-up-or-column-left",
    "focus-window-up-or-column-right",
    "focus-window-or-workspace-down",
    "focus-window-or-workspace-up",
    "focus-window-top",
    "focus-window-bottom",
    "focus-window-down-or-top",
    "focus-window-up-or-bottom",
    "move-column-left",
    "move-column-right",
    "move-column-to-first",
    "move-column-to-last",
    "move-column-left-or-to-monitor-left",
    "move-column-right-or-to-monitor-right",
    "move-column-to-index",
    "move-window-down",
    "move-window-up",
    "move-window-down-or-to-workspace-down",
    "move-window-up-or-to-workspace-up",
    "consume-or-expel-window-left",
    "consume-or-expel-window-right",
    "consume-window-into-column",
    "expel-window-from-column",
    "swap-window-left",
    "swap-window-right",
    "toggle-column-tabbed-display",
    "set-column-display",
    "center-column",
    "center-window",
    "center-visible-columns",
    "focus-workspace-down",
    "focus-workspace-up",
    "focus-workspace",
    "focus-workspace-previous",
    "move-window-to-workspace-down",
    "move-window-to-workspace-up",
    "move-window-to-workspace",
    "move-column-to-workspace-down",
    "move-column-to-workspace-up",
    "move-column-to-workspace",
    "move-workspace-down",
    "move-workspace-up",
    "move-workspace-to-index",
    "move-workspace-to-monitor",
    "set-workspace-name",
    "unset-workspace-name",
    "focus-monitor-left",
    "focus-monitor-right",
    "focus-monitor-down",
    "focus-monitor-up",
    "focus-monitor-previous",
    "focus-monitor-next",
    "focus-monitor",
    "move-window-to-monitor-left",
    "move-window-to-monitor-right",
    "move-window-to-monitor-down",
    "move-window-to-monitor-up",
    "move-window-to-monitor-previous",
    "move-window-to-monitor-next",
    "move-window-to-monitor",
    "move-column-to-monitor-left",
    "move-column-to-monitor-right",
    "move-column-to-monitor-down",
    "move-column-to-monitor-up",
    "move-column-to-monitor-previous",
    "move-column-to-monitor-next",
    "move-column-to-monitor",
    "set-window-width",
    "set-window-height",
    "reset-window-height",
    "switch-preset-column-width",
    "switch-preset-column-width-back",
    "switch-preset-window-width",
    "switch-preset-window-width-back",
    "switch-preset-window-height",
    "switch-preset-window-height-back",
    "maximize-window-to-edges",
    "set-column-width",
    "switch-layout",
    "move-workspace-to-monitor-left",
    "move-workspace-to-monitor-right",
    "move-workspace-to-monitor-down",
    "move-workspace-to-monitor-up",
    "move-workspace-to-monitor-previous",
    "move-workspace-to-monitor-next",
    "move-window-to-floating",
    "move-window-to-tiling",
    "focus-floating",
    "focus-tiling",
    "switch-focus-between-floating-and-tiling",
    "toggle-window-rule-opacity",
    "toggle-windowed-fullscreen",
];
