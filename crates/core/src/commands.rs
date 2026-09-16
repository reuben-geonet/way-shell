//! Authoritative CLI command names, opcodes and shortcut descriptions.
pub type CommandGroup = (&'static str, &'static [(&'static str, u32)]);
pub const GROUPS: &[CommandGroup] = &[
    ("message-tray", &[("open", 1)]),
    ("volume", &[("up", 2), ("down", 3), ("set", 4), ("mute", 5)]),
    (
        "brightness",
        &[
            ("up", 6),
            ("down", 7),
            ("keyboard-up", 29),
            ("keyboard-down", 30),
        ],
    ),
    (
        "theme",
        &[
            ("dark", 8),
            ("light", 9),
            ("dump-dark", 10),
            ("dump-light", 11),
        ],
    ),
    ("activities", &[("show", 12), ("hide", 13), ("toggle", 14)]),
    (
        "app-switcher",
        &[("show", 15), ("hide", 16), ("toggle", 17)],
    ),
    (
        "workspace-switcher",
        &[("show", 18), ("hide", 19), ("toggle", 20)],
    ),
    (
        "output-switcher",
        &[("show", 21), ("hide", 22), ("toggle", 23)],
    ),
    (
        "workspace-app-switcher",
        &[("show", 24), ("hide", 25), ("toggle", 26)],
    ),
    ("bluelight-filter", &[("enable", 27), ("disable", 28)]),
    (
        "rename-switcher",
        &[("show", 31), ("hide", 32), ("toggle", 33)],
    ),
    ("shortcuts", &[("show", 34), ("hide", 35), ("toggle", 36)]),
];

pub fn parse_volume(text: &str) -> Result<f32, String> {
    let value: f32 = text.parse().map_err(|_| "expected a decimal volume")?;
    let mantissa = text.split(['e', 'E']).next().unwrap_or("");
    let underflow = value == 0.0 && mantissa.bytes().any(|b| (b'1'..=b'9').contains(&b));
    if !value.is_finite() || !(0.0..=1.0).contains(&value) || value.is_subnormal() || underflow {
        return Err("volume must be finite and between 0.0 and 1.0".into());
    }
    Ok(value)
}

pub fn describe(args: &[String]) -> Option<String> {
    let (group, actions) = GROUPS
        .iter()
        .find(|(group, _)| args.first().is_some_and(|a| a == group))?;
    let (action, opcode) = actions
        .iter()
        .find(|(action, _)| args.get(1).is_some_and(|a| a == action))?;
    if args.len() != if *opcode == 4 { 3 } else { 2 } {
        return None;
    }
    let text = match *group {
        "message-tray" => "Open notifications and calendar".into(),
        "volume" => match *action {
            "up" => "Increase volume".into(),
            "down" => "Decrease volume".into(),
            "mute" => "Toggle mute".into(),
            "set" => {
                let value = parse_volume(&args[2]).ok()?;
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return None;
                }
                format!("Set volume to {:.0}%", value * 100.0)
            }
            _ => return None,
        },
        "brightness" => match *action {
            "up" => "Increase screen brightness",
            "down" => "Decrease screen brightness",
            "keyboard-up" => "Increase keyboard brightness",
            _ => "Decrease keyboard brightness",
        }
        .into(),
        "theme" => match *action {
            "dark" => "Use dark theme",
            "light" => "Use light theme",
            "dump-dark" => "Export dark theme",
            _ => "Export light theme",
        }
        .into(),
        "bluelight-filter" => format!(
            "{} blue light filter",
            if *action == "enable" {
                "Enable"
            } else {
                "Disable"
            }
        ),
        _ if *action == "hide" => format!("Close {}", group.replace('-', " ")),
        "activities" => "Search and launch applications".into(),
        "app-switcher" => "Switch applications".into(),
        "workspace-switcher" => "Choose a workspace".into(),
        "workspace-app-switcher" => "Choose a workspace for this window".into(),
        "output-switcher" => "Choose an output".into(),
        "rename-switcher" => "Rename workspace".into(),
        "shortcuts" => "Show keyboard shortcuts".into(),
        _ => return None,
    };
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_opcodes_are_unique_and_described() {
        let mut opcodes = std::collections::BTreeSet::new();
        for (group, actions) in GROUPS {
            for (action, opcode) in *actions {
                assert!(opcodes.insert(opcode));
                let mut args = vec![group.to_string(), action.to_string()];
                if *opcode == 4 {
                    args.push("0.5".into());
                }
                assert!(describe(&args).is_some());
            }
        }
        assert_eq!(opcodes.len(), 36);
        assert!(describe(&["activities".into(), "bad".into()]).is_none());
    }
}
