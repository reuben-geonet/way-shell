use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::OnceLock;
use syntect::parsing::{ParseState, ScopeStack, SyntaxSet};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Text,
    Code,
    Image,
    File,
    Files,
    Link,
    Character,
    Color,
}
impl Kind {
    pub const ALL: [Self; 8] = [
        Self::Text,
        Self::Code,
        Self::Image,
        Self::File,
        Self::Files,
        Self::Link,
        Self::Character,
        Self::Color,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Code => "Code",
            Self::Image => "Image",
            Self::File => "File",
            Self::Files => "Files",
            Self::Link => "Link",
            Self::Character => "Character",
            Self::Color => "Color",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Self::Text => "way-shell-text-symbolic",
            Self::Code => "way-shell-folder-code-legacy-symbolic",
            Self::Image => "way-shell-image-symbolic",
            Self::File => "way-shell-file-symbolic",
            Self::Files => "way-shell-folder-symbolic",
            Self::Link => "way-shell-link-symbolic",
            Self::Character => "way-shell-character-symbolic",
            Self::Color => "way-shell-color-symbolic",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Content {
    pub kind: Kind,
    pub mime: String,
    pub text: String,
    #[serde(skip)]
    pub bytes: Vec<u8>,
    pub metadata: Value,
}
impl Content {
    pub fn text(text: &str) -> Self {
        Self {
            kind: Kind::Text,
            mime: "text/plain;charset=utf-8".into(),
            text: text.into(),
            bytes: text.as_bytes().to_vec(),
            metadata: Value::Null,
        }
    }
    pub fn set_text(&mut self, text: String) {
        self.bytes = text.as_bytes().to_vec();
        self.text = text;
    }
    pub fn formats(&self) -> Vec<(String, Vec<u8>)> {
        match self.kind {
            Kind::Image => vec![(self.mime.clone(), self.bytes.clone())],
            Kind::File | Kind::Files => vec![
                (
                    "x-special/gnome-copied-files".into(),
                    format!("copy\n{}", self.text).into_bytes(),
                ),
                (
                    "text/uri-list".into(),
                    self.text.replace('\n', "\r\n").into_bytes(),
                ),
            ],
            _ => vec![
                ("text/plain;charset=utf-8".into(), self.bytes.clone()),
                ("text/plain".into(), self.bytes.clone()),
                ("UTF8_STRING".into(), self.bytes.clone()),
            ],
        }
    }
}
pub fn classify(mime: &str, bytes: Vec<u8>, max_characters: usize) -> Result<Content, String> {
    if mime.starts_with("image/") {
        if bytes.is_empty() {
            return Err("Empty image".into());
        }
        return Ok(Content {
            kind: Kind::Image,
            mime: mime.into(),
            text: String::new(),
            bytes,
            metadata: Value::Null,
        });
    }
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    if text.trim().is_empty() {
        return Err("Empty clipboard content".into());
    }
    if matches!(mime, "text/uri-list" | "x-special/gnome-copied-files") {
        let paths: Vec<_> = text
            .lines()
            .map(str::trim)
            .filter(|line| {
                !line.is_empty() && !line.starts_with('#') && !matches!(*line, "copy" | "cut")
            })
            .collect();
        if paths.is_empty() {
            return Err("Empty file list".into());
        }
        let kind = if paths.len() == 1 {
            Kind::File
        } else {
            Kind::Files
        };
        let text = paths.join("\n");
        return Ok(Content {
            kind,
            mime: "text/uri-list".into(),
            bytes: text.as_bytes().to_vec(),
            text,
            metadata: json!({"operation":"copy"}),
        });
    }
    let mut content = Content::text(&text);
    let trimmed = text.trim();
    content.kind = if (trimmed.starts_with("https://") || trimmed.starts_with("http://"))
        && !trimmed.chars().any(char::is_whitespace)
    {
        Kind::Link
    } else if trimmed.graphemes(true).count() <= max_characters {
        Kind::Character
    } else if trimmed.parse::<csscolorparser::Color>().is_ok() {
        Kind::Color
    } else if let Some(language) = detect_code(trimmed) {
        content.metadata = json!({"language":language});
        Kind::Code
    } else {
        Kind::Text
    };
    Ok(content)
}
pub fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(two_face::syntax::extra_newlines)
}
fn sample(text: &str) -> &str {
    let end = text
        .char_indices()
        .nth(10_000)
        .map_or(text.len(), |(index, _)| index);
    &text[..end]
}
fn detect_code(text: &str) -> Option<String> {
    // Avoid running every grammar on ordinary prose. Syntax scoring is deliberately
    // conservative; the editor also allows explicit language selection.
    if !text.contains(['{', '}', ';', '=', '<', '>', '[', ']', '(', ')']) {
        return None;
    }
    let text = sample(text);
    let syntaxes = syntaxes();
    if text.starts_with("#!")
        && let Some(syntax) =
            syntaxes.find_syntax_by_first_line(text.lines().next().unwrap_or_default())
    {
        return Some(syntax.name.clone());
    }
    if serde_json::from_str::<Value>(text).is_ok() && text.starts_with(['{', '[']) {
        return Some("JSON".into());
    }
    // Strong signatures avoid parsing every grammar on the capture worker.
    for (name, markers) in [
        ("Rust", &["pub fn ", "impl ", "#[derive(", "fn main("][..]),
        ("Python", &["def ", "import ", "from "][..]),
        ("TypeScript", &["interface ", ": string", ": number"][..]),
        ("JavaScript", &["const ", "let ", "function ", "=>"][..]),
        ("Go", &["package main", "func "][..]),
        ("C++", &["#include", "std::"][..]),
        ("Java", &["public class ", "public static "][..]),
        ("SQL", &["SELECT ", "CREATE TABLE ", "INSERT INTO "][..]),
        ("HTML", &["<!DOCTYPE", "<html", "<div", "<span"][..]),
        ("PHP", &["<?php"][..]),
    ] {
        if markers.iter().any(|marker| text.contains(marker))
            && let Some(syntax) = syntaxes
                .find_syntax_by_token(name)
                .or_else(|| syntaxes.find_syntax_by_name(name))
        {
            return Some(syntax.name.clone());
        }
    }
    let mut best = (3, None);
    for name in ["JavaScript", "Python", "C", "CSS", "Bash", "Ruby", "YAML"] {
        let Some(syntax) = syntaxes
            .find_syntax_by_token(name)
            .or_else(|| syntaxes.find_syntax_by_name(name))
        else {
            continue;
        };
        let mut parser = ParseState::new(syntax);
        let mut score = 0;
        for line in text.split_inclusive('\n').take(80) {
            let Ok(operations) = parser.parse_line(line, syntaxes) else {
                break;
            };
            for (_, operation) in operations {
                if let syntect::parsing::ScopeStackOp::Push(scope) = operation {
                    let scope = scope.to_string();
                    if [
                        "keyword.",
                        "storage.",
                        "entity.name.function",
                        "entity.name.type",
                        "support.function",
                    ]
                    .iter()
                    .any(|prefix| scope.starts_with(prefix))
                    {
                        score += 1;
                    }
                }
            }
        }
        if score > best.0 {
            best = (score, Some(syntax.name.clone()));
        }
    }
    best.1
}
#[derive(Clone, Debug)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub role: &'static str,
}
pub fn highlight(text: &str, language: &str) -> Vec<HighlightSpan> {
    let syntaxes = syntaxes();
    let Some(syntax) = syntaxes
        .find_syntax_by_name(language)
        .or_else(|| syntaxes.find_syntax_by_token(language))
    else {
        return Vec::new();
    };
    let mut parser = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut spans = Vec::new();
    let mut offset = 0;
    for line in sample(text).split_inclusive('\n') {
        let Ok(operations) = parser.parse_line(line, syntaxes) else {
            break;
        };
        let mut last = 0;
        for (index, operation) in operations.into_iter().chain(std::iter::once((
            line.len(),
            syntect::parsing::ScopeStackOp::Noop,
        ))) {
            if index > last {
                let scopes = stack
                    .scopes
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" ");
                let role = if scopes.contains("comment") {
                    "comment"
                } else if scopes.contains("string") {
                    "string"
                } else if scopes.contains("constant.numeric") {
                    "number"
                } else if scopes.contains("keyword") || scopes.contains("storage") {
                    "keyword"
                } else if scopes.contains("entity.name") || scopes.contains("support.") {
                    "name"
                } else {
                    "text"
                };
                spans.push(HighlightSpan {
                    start: offset + last,
                    end: offset + index,
                    role,
                });
            }
            let _ = stack.apply(&operation);
            last = index;
        }
        offset += line.len();
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_text_and_recognizes_rich_content() {
        for (text, kind) in [
            ("  hello world  ", Kind::Text),
            ("👨‍👨‍👶‍👧", Kind::Character),
            ("#3584e4", Kind::Color),
            ("oklch(65% 0.2 45)", Kind::Color),
            ("https://example.org/path", Kind::Link),
            (
                "#[derive(Debug)]\npub enum State { Start, Closed }\nimpl State { pub fn get() {} }",
                Kind::Code,
            ),
        ] {
            let content = classify("text/plain", text.as_bytes().to_vec(), 1).unwrap();
            assert_eq!(content.kind, kind, "{text}");
            assert_eq!(content.bytes, text.as_bytes());
        }
    }
    #[test]
    fn copied_files_restore_both_interoperable_formats() {
        let content = classify(
            "x-special/gnome-copied-files",
            b"cut\nfile:///tmp/a%20b\nfile:///tmp/c".to_vec(),
            1,
        )
        .unwrap();
        assert_eq!(content.kind, Kind::Files);
        assert_eq!(
            content.formats()[0].1,
            b"copy\nfile:///tmp/a%20b\nfile:///tmp/c"
        );
    }
}
