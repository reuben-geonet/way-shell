use super::*;
use knuffel::{
    ast::{Literal, SpannedNode, Value},
    span::Span,
};
type Node = SpannedNode<Span>;

fn scalar(value: &Value<Span>, text: &str) -> String {
    match &*value.literal {
        Literal::String(value) => value.to_string(),
        _ => text[value.literal.span().0..value.literal.span().1].to_owned(),
    }
}
fn line(node: &Node, text: &str) -> usize {
    text[..node.span().0]
        .bytes()
        .filter(|c| *c == b'\n')
        .count()
        + 1
}

pub(super) fn parse(loader: &mut Loader<'_>, path: &Path, text: &str, depth: usize) {
    let Some(chunks) = chunks(text) else {
        loader.diagnostic(path, 1, Severity::Error, "KDL nesting limit exceeded");
        return;
    };
    let mut sections = BTreeSet::new();
    match knuffel::parse_ast::<Span>(&path.to_string_lossy(), text) {
        Ok(doc) => nodes(loader, path, text, depth, 0, doc.nodes, &mut sections),
        Err(error) => {
            loader.diagnostic(path, 1, Severity::Error, format!("Invalid KDL: {error}"));
            // Salvage only complete, independently valid top-level parts. Never
            // guess the contents of a broken binds block or a disabled node.
            for range in chunks {
                let part = &text[range.clone()];
                if let Ok(doc) = knuffel::parse_ast::<Span>(&path.to_string_lossy(), part) {
                    let offset = text[..range.start].bytes().filter(|c| *c == b'\n').count();
                    nodes(loader, path, part, depth, offset, doc.nodes, &mut sections);
                }
            }
        }
    }
}
fn nodes(
    loader: &mut Loader<'_>,
    path: &Path,
    text: &str,
    depth: usize,
    offset: usize,
    nodes: Vec<Node>,
    sections: &mut BTreeSet<String>,
) {
    for node in nodes {
        let name = &**node.node_name;
        let location = offset + line(&node, text);
        if matches!(name, "binds" | "input") && !sections.insert(name.to_owned()) {
            loader.diagnostic(
                path,
                location,
                Severity::Error,
                format!("Duplicate {name} block in one file"),
            );
        }
        match name {
            "include" => {
                if node.arguments.len() != 1 {
                    loader.diagnostic(path, location, Severity::Error, "Include expects one path");
                    continue;
                }
                let optional = node.properties.iter().any(|(key, value)| {
                    &***key == "optional" && matches!(*value.literal, Literal::Bool(true))
                });
                loader.include(
                    path,
                    location,
                    &scalar(&node.arguments[0], text),
                    depth,
                    optional,
                    "default",
                );
            }
            "input" => {
                for child in node.children() {
                    let key = &**child.node_name;
                    if matches!(key, "mod-key" | "mod-key-nested")
                        && let Some(value) = child.arguments.first()
                    {
                        let value = normalize_keys(&scalar(value, text), false);
                        if key == "mod-key" {
                            loader.mod_key = value;
                        } else {
                            loader.mod_nested = value;
                        }
                    }
                }
            }
            "binds" => {
                let mut keys = BTreeSet::new();
                for node in node.children() {
                    let trigger = normalize_keys(&node.node_name, false);
                    let location = offset + line(node, text);
                    if !keys.insert(trigger.clone()) {
                        loader.diagnostic(
                            path,
                            location,
                            Severity::Error,
                            format!("Duplicate binding: {trigger}"),
                        );
                        continue;
                    }
                    let mut title = None;
                    let mut conditions = Vec::new();
                    for (key, value) in &node.properties {
                        if &***key == "hotkey-overlay-title" {
                            match &*value.literal {
                                Literal::Null => title = Some(None),
                                Literal::String(value) => title = Some(Some(value.to_string())),
                                _ => loader.diagnostic(
                                    path,
                                    location,
                                    Severity::Error,
                                    "hotkey-overlay-title must be a string or null",
                                ),
                            }
                        } else {
                            conditions.push(format!("{}={}", &***key, scalar(value, text)));
                        }
                    }
                    if node.children().len() != 1 {
                        loader.diagnostic(
                            path,
                            location,
                            Severity::Error,
                            "Binding expects exactly one action",
                        );
                    }
                    let actions = node
                        .children()
                        .map(|child| {
                            let name = child.node_name.to_string();
                            let mut arguments: Vec<String> =
                                child.arguments.iter().map(|v| scalar(v, text)).collect();
                            match name.as_str() {
                                "spawn" => Action::Spawn(arguments),
                                "spawn-sh" => Action::Shell(arguments.join(" ")),
                                _ => {
                                    arguments.extend(child.properties.iter().map(
                                        |(key, value)| {
                                            format!("{}={}", &***key, scalar(value, text))
                                        },
                                    ));
                                    Action::Native { name, arguments }
                                }
                            }
                        })
                        .collect();
                    loader.insert(Binding {
                        trigger,
                        mode: "default".into(),
                        conditions,
                        actions,
                        original: text[node.span().0..node.span().1].trim().into(),
                        source: Source {
                            path: path.into(),
                            line: location,
                        },
                        title,
                        physical: false,
                        identity: Vec::new(),
                    });
                }
            }
            _ => (),
        }
    }
}

// A non-recursive lexical pass bounds both KDL children and block comments.
// It also locates safe recovery boundaries, including raw strings and /- nodes.
fn chunks(text: &str) -> Option<Vec<std::ops::Range<usize>>> {
    let bytes = text.as_bytes();
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let mut depth = 0usize;
    let mut comments = 0usize;
    let mut line_comment = false;
    let mut quoted = false;
    let mut raw: Option<usize> = None;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(hashes) = raw {
            if c == b'"'
                && bytes
                    .get(i + 1..i + 1 + hashes)
                    .is_some_and(|s| s.iter().all(|c| *c == b'#'))
            {
                i += hashes + 1;
                raw = None;
            } else {
                i += 1;
            }
            continue;
        }
        if quoted {
            if c == b'\\' {
                i = (i + 2).min(bytes.len());
                continue;
            }
            if c == b'"' {
                quoted = false;
            }
            i += 1;
            continue;
        }
        if line_comment {
            if c != b'\n' {
                i += 1;
                continue;
            }
            line_comment = false;
        }
        if bytes.get(i..i + 2) == Some(b"/*") {
            comments += 1;
            if comments > MAX_DEPTH {
                return None;
            }
            i += 2;
            continue;
        }
        if comments > 0 {
            if bytes.get(i..i + 2) == Some(b"*/") {
                comments -= 1;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if bytes.get(i..i + 2) == Some(b"//") {
            line_comment = true;
            i += 2;
            continue;
        }
        if c == b'r' {
            let mut end = i + 1;
            while bytes.get(end) == Some(&b'#') {
                end += 1;
            }
            if bytes.get(end) == Some(&b'"') {
                raw = Some(end - i - 1);
                i = end + 1;
                continue;
            }
        }
        match c {
            b'"' => quoted = true,
            b'{' => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return None;
                }
            }
            b'}' => depth = depth.saturating_sub(1),
            b'\\' => {
                i += 1;
                while bytes
                    .get(i)
                    .is_some_and(|c| *c == b' ' || *c == b'\t' || *c == b'\r')
                {
                    i += 1;
                }
                if bytes.get(i) == Some(&b'\n') {
                    i += 1;
                }
                continue;
            }
            b'\n' | b';' if depth == 0 => {
                chunks.push(start..i + 1);
                start = i + 1;
            }
            _ => (),
        }
        i += 1;
    }
    if start < text.len() {
        chunks.push(start..text.len());
    }
    Some(chunks)
}
