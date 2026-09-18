use std::path::Path;
use way_shell_shortcuts::{Action, Group, Snapshot, describe_native};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub section: String,
    pub keys: Vec<String>,
    pub description: String,
    pub details: Vec<String>,
    identity: String,
}

/// Accept only a statically known argv. Shell syntax is never evaluated.
fn shell_words(text: &str) -> Option<Vec<String>> {
    let mut quote = None;
    let mut escape = false;
    for c in text.chars() {
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escape = true;
            continue;
        }
        if Some(c) == quote {
            quote = None;
            continue;
        }
        if quote.is_none() && matches!(c, '\'' | '"') {
            quote = Some(c);
            continue;
        }
        if quote != Some('\'') && matches!(c, '$' | '`') {
            return None;
        }
        if quote.is_none()
            && matches!(
                c,
                ';' | '|' | '&' | '<' | '>' | '(' | ')' | '\n' | '*' | '?'
            )
        {
            return None;
        }
    }
    shlex::split(text)
}
pub(super) fn describe_spawn(args: &[String], depth: u8) -> Option<String> {
    if depth > 4 {
        return None;
    }
    let name = Path::new(args.first()?).file_name()?.to_str()?;
    if name == "way-sh" {
        return way_shell_core::commands::describe(&args[1..]);
    }
    if name == "env" {
        let mut start = 1;
        while args
            .get(start)
            .is_some_and(|a| a.contains('=') && !a.starts_with('-'))
        {
            start += 1;
        }
        if args.get(start).is_some_and(|a| a == "--") {
            start += 1;
        }
        return describe_spawn(&args[start..], depth + 1);
    }
    if matches!(name, "sh" | "bash" | "dash" | "zsh")
        && args.len() == 3
        && matches!(args[1].as_str(), "-c" | "-lc")
    {
        return describe_spawn(&shell_words(&args[2])?, depth + 1);
    }
    None
}
pub fn rows(snapshot: &Snapshot) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for binding in &snapshot.bindings {
        if binding.title == Some(None) {
            continue;
        }
        let descriptions: Option<Vec<_>> = binding
            .actions
            .iter()
            .map(|action| {
                if let Some(description) = describe_native(action) {
                    Some((description.group, description.text))
                } else {
                    let text = match action {
                        Action::Spawn(args) => describe_spawn(args, 0),
                        Action::Shell(text) => {
                            shell_words(text).and_then(|args| describe_spawn(&args, 0))
                        }
                        _ => None,
                    }?;
                    Some((Group::WayShell, text))
                }
            })
            .collect();
        let Some(descriptions) = descriptions.filter(|d| !d.is_empty()) else {
            continue;
        };
        let group = if descriptions.iter().any(|(g, _)| *g == Group::WayShell) {
            Group::WayShell
        } else {
            descriptions[0].0
        };
        let section = if binding.mode == "default" {
            group.title().to_owned()
        } else {
            format!("Mode: {}", binding.mode)
        };
        let mut description = binding.title.clone().flatten().unwrap_or_else(|| {
            descriptions
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>()
                .join("; ")
        });
        if !binding.conditions.is_empty() {
            description.push_str(&format!(" ({})", binding.conditions.join(", ")));
        }
        let identity = format!(
            "{:?}|{:?}|{}",
            binding.actions, binding.conditions, binding.mode
        );
        let detail = format!(
            "{}:{}\n{}",
            binding.source.path.display(),
            binding.source.line,
            binding.original
        );
        if let Some(row) = rows.iter_mut().find(|row| {
            row.section == section && row.description == description && row.identity == identity
        }) {
            if !row.keys.contains(&binding.trigger) {
                row.keys.push(binding.trigger.clone());
            }
            row.details.push(detail);
        } else {
            rows.push(Row {
                section,
                keys: vec![binding.trigger.clone()],
                description,
                details: vec![detail],
                identity,
            });
        }
    }
    rows.sort_by_key(|row| {
        (
            match row.section.as_str() {
                "Way-Shell" => 0,
                "Windows and layout" => 1,
                "Workspaces and outputs" => 2,
                "Session and compositor" => 3,
                _ => 4,
            },
            row.section.clone(),
        )
    });
    rows
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_commands_must_be_static_and_in_the_catalog() {
        for text in [
            "'/opt/bin/way-sh' workspace-app-switcher toggle",
            "env X=1 way-sh workspace-app-switcher toggle",
            "sh -c 'way-sh workspace-app-switcher toggle'",
        ] {
            assert_eq!(
                describe_spawn(&shell_words(text).unwrap(), 0).as_deref(),
                Some("Choose a workspace for this window")
            );
        }
        for text in [
            "way-sh unknown toggle",
            "way-sh activities toggle extra",
            "sh -c 'way-sh activities toggle; malicious'",
            "foot",
            "swaylock",
            "grim",
            "wpctl set-volume 1 1",
        ] {
            assert!(
                describe_spawn(&shell_words(text).unwrap(), 0).is_none(),
                "{text}"
            );
        }
        for text in [
            "$(way-sh activities toggle)",
            "way-sh activities toggle && echo x",
            "way-sh $action toggle",
        ] {
            assert!(shell_words(text).is_none());
        }
    }
}
