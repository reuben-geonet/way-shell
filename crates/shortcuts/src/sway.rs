//! A small lexer for Sway configuration and binding commands; never a shell.
use super::*;

#[derive(Debug)]
pub struct Word {
    pub value: String,
    pub start: usize,
    pub end: usize,
}
/// Tokenize quoted and escaped words while retaining source ranges.
pub fn words(text: &str) -> Result<Vec<Word>, String> {
    let mut result = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        let mut value = String::new();
        let mut quote = None;
        let mut current = Some((start, ch));
        let mut end = start;
        while let Some((index, ch)) = current {
            end = index + ch.len_utf8();
            if ch == '\\' && quote != Some('\'') {
                let Some((index, ch)) = chars.next() else {
                    return Err("Unfinished escape".into());
                };
                end = index + ch.len_utf8();
                value.push(ch);
            } else if Some(ch) == quote {
                quote = None;
            } else if quote.is_none() && matches!(ch, '\'' | '"') {
                quote = Some(ch);
            } else if quote.is_none() && ch.is_whitespace() {
                break;
            } else {
                value.push(ch);
            }
            current = chars.next();
        }
        if quote.is_some() {
            return Err("Unterminated quoted word".into());
        }
        result.push(Word { value, start, end });
    }
    Ok(result)
}

/// Split delimiters outside strings/criteria. Used for command sequences too.
pub fn split_commands(text: &str) -> Vec<String> {
    let mut quote = None;
    let mut escape = false;
    let mut criteria = 0;
    let mut start = 0;
    let mut parts = Vec::new();
    for (i, c) in text.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' {
            escape = true;
            continue;
        }
        if Some(c) == quote {
            quote = None;
            continue;
        }
        if quote.is_some() {
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '[' => criteria += 1,
            ']' => criteria -= 1,
            ',' | ';' if criteria == 0 => {
                parts.push(text[start..i].trim().into());
                start = i + 1;
            }
            _ => (),
        }
    }
    parts.push(text[start..].trim().into());
    parts
}

// Newlines and configuration braces are structural; semicolons remain part of
// the binding's command sequence. Quoted shell fragments are kept verbatim.
fn statements(text: &str) -> Vec<(String, usize, char)> {
    let mut parts = Vec::new();
    let mut buffer = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    let mut line = 1;
    let mut start = 1;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\n' {
            line += 1;
        }
        if comment {
            if c != '\n' {
                continue;
            }
            comment = false;
        }
        if escaped {
            escaped = false;
            if c == '\n' {
                buffer.pop();
                continue;
            }
            buffer.push(c);
            continue;
        }
        if c == '\\' {
            escaped = true;
            buffer.push(c);
            continue;
        }
        if Some(c) == quote {
            quote = None;
            buffer.push(c);
            continue;
        }
        if quote.is_some() && c != '\n' {
            buffer.push(c);
            continue;
        }
        if matches!(c, '\'' | '"') {
            quote = Some(c);
            buffer.push(c);
            continue;
        }
        if c == '#' {
            comment = true;
            continue;
        }
        let structural_brace = (c == '{'
            && buffer.ends_with(char::is_whitespace)
            && chars.peek().is_none_or(|c| c.is_whitespace() || *c == '}'))
            || (c == '}'
                && (buffer.trim().is_empty() || buffer.ends_with(char::is_whitespace))
                && chars.peek().is_none_or(|c| c.is_whitespace() || *c == '}'));
        if c == '\n' || structural_brace {
            if c == '}' && !buffer.trim().is_empty() {
                parts.push((buffer.trim().into(), start, '\n'));
                buffer.clear();
            }
            parts.push((buffer.trim().into(), start, c));
            buffer.clear();
            start = line;
            quote = None;
        } else {
            buffer.push(c);
        }
    }
    if !buffer.trim().is_empty() {
        parts.push((buffer.trim().into(), start, '\n'));
    }
    parts
}

pub(super) fn parse(
    loader: &mut Loader<'_>,
    path: &Path,
    text: &str,
    depth: usize,
    initial_mode: &str,
) {
    let mut contexts = vec![(initial_mode.to_owned(), String::new(), false)];
    for (raw, line, ending) in statements(text) {
        if ending == '}' {
            if contexts.len() == 1 {
                loader.diagnostic(path, line, Severity::Error, "Unexpected closing brace");
            } else {
                contexts.pop();
            }
            continue;
        }
        if raw.is_empty() {
            continue;
        }
        let (mode, prefix, skip) = contexts.last().unwrap().clone();
        let expanded = match loader.expand(&raw) {
            Ok(value) => value,
            Err(error) => {
                loader.diagnostic(path, line, Severity::Error, error);
                return;
            }
        };
        let full = if prefix.is_empty() {
            expanded
        } else {
            format!("{prefix} {expanded}")
        };
        let tokens = match words(&full) {
            Ok(tokens) => tokens,
            Err(error) => {
                loader.diagnostic(path, line, Severity::Error, error);
                continue;
            }
        };
        let Some(first) = tokens.first().map(|w| w.value.as_str()) else {
            continue;
        };
        if ending == '{' {
            if contexts.len() >= MAX_DEPTH {
                loader.diagnostic(path, line, Severity::Error, "Block nesting limit exceeded");
                return;
            }
            if !skip && first == "mode" {
                let name = tokens
                    .iter()
                    .skip(1)
                    .find(|w| !w.value.starts_with("--"))
                    .map(|w| w.value.clone())
                    .unwrap_or_default();
                contexts.push((name, String::new(), false));
            } else if !skip && matches!(first, "bindsym" | "bindcode" | "unbindsym" | "unbindcode")
            {
                contexts.push((mode, full, false));
            } else {
                contexts.push((mode, String::new(), true));
            }
            continue;
        }
        if skip {
            continue;
        }
        match first {
            "set" if tokens.len() >= 3 => {
                // The name itself must not undergo expansion on reassignment.
                let original = words(&raw).unwrap_or_default();
                if let Some(name) = original.get(1).and_then(|t| t.value.strip_prefix('$')) {
                    let value = full[tokens[2].start..].trim();
                    let value = if tokens.len() == 3 {
                        tokens[2].value.clone()
                    } else {
                        value.into()
                    };
                    loader.variables.insert(name.into(), value);
                }
            }
            "include" => {
                for token in tokens.iter().skip(1) {
                    loader.include(path, line, &token.value, depth, false, &mode);
                }
            }
            "bindsym" | "bindcode" | "unbindsym" | "unbindcode" => {
                let physical = first.ends_with("code");
                let unbind = first.starts_with("unbind");
                let mut index = 1;
                let mut conditions = Vec::new();
                let mut identity = Vec::new();
                let mut device = "*".to_owned();
                let mut border = false;
                let mut contents = false;
                let mut exclude_titlebar = false;
                while let Some(token) = tokens.get(index).filter(|t| t.value.starts_with("--")) {
                    let option = token.value.clone();
                    if option != "--no-warn" {
                        conditions.push(option.clone());
                    }
                    match option.as_str() {
                        "--release" | "--locked" | "--inhibited" => identity.push(option),
                        "--whole-window" => {
                            border = true;
                            contents = true;
                        }
                        "--border" => border = true,
                        "--exclude-titlebar" => exclude_titlebar = true,
                        "--to-code" if !physical => identity.push(option),
                        "--to-code" | "--no-warn" | "--no-repeat" => (),
                        _ if option.starts_with("--input-device=") => {
                            device = option["--input-device=".len()..].into();
                        }
                        _ => loader.diagnostic(
                            path,
                            line,
                            Severity::Error,
                            format!("Unsupported binding option: {option}"),
                        ),
                    }
                    index += 1;
                }
                let Some(key) = tokens.get(index) else {
                    loader.diagnostic(path, line, Severity::Error, "Binding has no trigger");
                    continue;
                };
                let trigger = normalize_keys(&key.value, physical);
                identity.push(format!("device={device}"));
                let mouse = border
                    || contents
                    || exclude_titlebar
                    || key
                        .value
                        .split('+')
                        .any(|key| key.starts_with("button") || key.starts_with("BTN_"));
                if mouse {
                    identity.push(format!("mouse={border},{contents},{}", !exclude_titlebar));
                }
                if trigger.contains('$') {
                    loader.diagnostic(
                        path,
                        line,
                        Severity::Warning,
                        "Unresolved variable in binding trigger",
                    );
                }
                identity.sort();
                identity.dedup();
                conditions.sort();
                conditions.dedup();
                if unbind {
                    loader.snapshot.bindings.retain(|b| {
                        !(b.trigger == trigger
                            && b.mode == mode
                            && b.physical == physical
                            && b.identity == identity)
                    });
                    continue;
                }
                let Some(command) = tokens.get(index + 1) else {
                    loader.diagnostic(path, line, Severity::Error, "Binding has no action");
                    continue;
                };
                let original = full[command.start..].trim().to_owned();
                let mut actions = Vec::new();
                for part in split_commands(&original) {
                    let mut command = part.as_str();
                    if command.starts_with('[') {
                        if let Some(end) = criteria_end(command) {
                            conditions.push(command[..=end].into());
                            command = command[end + 1..].trim();
                        } else {
                            loader.diagnostic(
                                path,
                                line,
                                Severity::Error,
                                "Unclosed command criteria",
                            );
                            continue;
                        }
                    }
                    let args = match words(command) {
                        Ok(args) => args,
                        Err(error) => {
                            loader.diagnostic(path, line, Severity::Error, error);
                            continue;
                        }
                    };
                    if let Some(name) = args.first() {
                        if matches!(name.value.as_str(), "exec" | "exec_always") {
                            let start = args.iter().skip(1).find(|a| a.value != "--no-startup-id");
                            if let Some(start) = start {
                                actions.push(Action::Shell(command[start.start..].trim().into()));
                            }
                        } else {
                            actions.push(Action::Native {
                                name: name.value.clone(),
                                arguments: args.iter().skip(1).map(|w| w.value.clone()).collect(),
                            });
                        }
                    }
                }
                loader.insert(Binding {
                    trigger,
                    mode,
                    conditions,
                    actions,
                    original,
                    source: Source {
                        path: path.into(),
                        line,
                    },
                    title: None,
                    physical,
                    identity,
                });
            }
            _ => (),
        }
    }
    if contexts.len() != 1 {
        loader.diagnostic(
            path,
            text.lines().count(),
            Severity::Error,
            "Unclosed configuration block",
        );
    }
}

fn criteria_end(text: &str) -> Option<usize> {
    let mut quote = None;
    let mut escape = false;
    for (i, c) in text.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' {
            escape = true;
            continue;
        }
        if Some(c) == quote {
            quote = None;
        } else if quote.is_none() {
            if matches!(c, '"' | '\'') {
                quote = Some(c);
            } else if c == ']' {
                return Some(i);
            }
        }
    }
    None
}
