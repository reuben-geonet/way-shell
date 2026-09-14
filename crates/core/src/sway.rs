//! Bounded i3/Sway framing and the subset of messages used by Way Shell.
use crate::wm::{Action, Output, Workspace};
use serde::Deserialize;

pub const COMMAND: u32 = 0;
pub const WORKSPACES: u32 = 1;
pub const SUBSCRIBE: u32 = 2;
pub const OUTPUTS: u32 = 3;
pub const WORKSPACE_EVENT: u32 = 1 << 31;
pub const OUTPUT_EVENT: u32 = (1 << 31) | 1;
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;
const HEADER: usize = 14;
const MAGIC: &[u8; 6] = b"i3-ipc";

#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    pub kind: u32,
    pub payload: Vec<u8>,
}

pub fn encode(kind: u32, payload: &[u8]) -> Result<Vec<u8>, String> {
    if payload.len() > MAX_PAYLOAD {
        return Err("Sway IPC payload exceeds 16 MiB".into());
    }
    let mut bytes = Vec::with_capacity(HEADER + payload.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&kind.to_le_bytes());
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

#[derive(Default)]
pub struct Decoder {
    bytes: Vec<u8>,
}

impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, String> {
        if bytes.len() > MAX_PAYLOAD + HEADER - self.bytes.len() {
            return Err("Buffered Sway IPC input exceeds 16 MiB".into());
        }
        self.bytes.extend_from_slice(bytes);
        let mut frames = Vec::new();
        let mut offset = 0;
        while self.bytes.len() - offset >= HEADER {
            let header = &self.bytes[offset..offset + HEADER];
            if &header[..6] != MAGIC {
                return Err("Invalid Sway IPC magic".into());
            }
            let size = u32::from_le_bytes(header[6..10].try_into().unwrap()) as usize;
            if size > MAX_PAYLOAD {
                return Err("Sway IPC payload exceeds 16 MiB".into());
            }
            if self.bytes.len() - offset < HEADER + size {
                break;
            }
            frames.push(Frame {
                kind: u32::from_le_bytes(header[10..14].try_into().unwrap()),
                payload: self.bytes[offset + HEADER..offset + HEADER + size].to_vec(),
            });
            offset += HEADER + size;
        }
        self.bytes.drain(..offset);
        Ok(frames)
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

pub fn workspaces(bytes: &[u8]) -> Result<Vec<Workspace>, serde_json::Error> {
    serde_json::from_slice(bytes)
}
pub fn outputs(bytes: &[u8]) -> Result<Vec<Output>, serde_json::Error> {
    serde_json::from_slice(bytes)
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceEvent {
    pub change: String,
    pub current: Option<Workspace>,
}

#[derive(Deserialize)]
struct Acknowledgement {
    success: bool,
    error: Option<String>,
}

pub fn acknowledged(bytes: &[u8], command: bool) -> Result<(), String> {
    let replies: Vec<Acknowledgement> = if command {
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid Sway command reply: {e}"))?
    } else {
        vec![
            serde_json::from_slice(bytes)
                .map_err(|e| format!("Invalid Sway subscription reply: {e}"))?,
        ]
    };
    if replies.is_empty() {
        return Err("Sway returned no command results".into());
    }
    for reply in replies {
        if !reply.success {
            return Err(reply
                .error
                .unwrap_or_else(|| "Sway rejected the request".into()));
        }
    }
    Ok(())
}

fn quote(value: &str) -> Result<String, String> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(
            "Workspace and output names must be nonempty and contain no control characters".into(),
        );
    }
    // IPC strips quote delimiters, but deliberately retains backslashes.
    // Alternate quote delimiters for a literal double quote; shell-style
    // backslash escaping would change the workspace name.
    let trailing = value
        .bytes()
        .rev()
        .take_while(|&byte| byte == b'\\')
        .count()
        % 2;
    let mut encoded = String::from("\"");
    let mut slashes = 0;
    for character in value[..value.len() - trailing].chars() {
        match character {
            '"' if slashes % 2 == 0 => encoded.push_str("\"'\"'\""),
            // Sway expands variables after removing quotes, including in IPC.
            '$' if slashes != 1 => encoded.push_str("$$"),
            _ => encoded.push(character),
        }
        slashes = if character == '\\' { slashes + 1 } else { 0 };
    }
    encoded.push('"');
    // An odd final backslash must follow the closing delimiter, otherwise it
    // escapes that delimiter. This is still part of the same argument.
    if trailing != 0 {
        encoded.push('\\');
    }
    Ok(encoded)
}

pub fn command(action: &Action) -> Result<String, String> {
    Ok(match action {
        Action::FocusWorkspace(target) if target.number >= 0 => {
            format!("workspace number {}", target.number)
        }
        Action::FocusWorkspace(target) => format!("workspace {}", quote(&target.name)?),
        Action::RenameWorkspace(name) => format!("rename workspace to {}", quote(name)?),
        Action::MoveWorkspaceToOutput(output) => {
            format!("move workspace to output {}", quote(output)?)
        }
        Action::MoveWindowToWorkspace(target) => {
            format!("move window to workspace {}", quote(&target.name)?)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wm::WorkspaceTarget;

    #[test]
    fn preserves_workspace_and_output_fixtures() {
        let workspaces = workspaces(include_bytes!(
            "../../../tests/fixtures/sway-workspaces.json"
        ))
        .unwrap();
        assert_eq!(workspaces[0].id, 17);
        assert!(workspaces[0].focused);
        assert_eq!(workspaces[1].name, "日本語");
        assert_eq!(workspaces[1].num, -1);
        assert!(workspaces[1].urgent);
        let outputs = outputs(include_bytes!("../../../tests/fixtures/sway-outputs.json")).unwrap();
        assert_eq!(outputs[0].name, "eDP-1");
        assert_eq!(outputs[1].current_workspace, None);
    }
    #[test]
    fn fragmented_and_concatenated_frames() {
        let first = encode(WORKSPACES, b"[]").unwrap();
        let mut all = first.clone();
        all.extend(encode(OUTPUTS, b"[]").unwrap());
        for step in 1..=all.len() {
            let mut decoder = Decoder::default();
            let received: Vec<_> = all
                .chunks(step)
                .flat_map(|part| decoder.push(part).unwrap())
                .collect();
            assert_eq!(
                received,
                [
                    Frame {
                        kind: WORKSPACES,
                        payload: b"[]".to_vec()
                    },
                    Frame {
                        kind: OUTPUTS,
                        payload: b"[]".to_vec()
                    }
                ]
            );
            assert!(decoder.is_empty());
        }
        for end in 1..first.len() {
            let mut decoder = Decoder::default();
            assert!(decoder.push(&first[..end]).unwrap().is_empty());
            assert!(!decoder.is_empty());
        }
    }
    #[test]
    fn malformed_messages_and_full_width_identifiers() {
        let mut invalid = encode(0, b"").unwrap();
        invalid[0] = 0;
        assert!(Decoder::default().push(&invalid).is_err());
        invalid[..6].copy_from_slice(MAGIC);
        invalid[6..10].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Decoder::default().push(&invalid).is_err());
        assert!(workspaces(b"{}").is_err());
        assert!(workspaces(b"[{\"name\":false}]").is_err());
        let ws =
            workspaces(br#"[{"id":18446744073709551615,"num":-1,"name":"large","output":null}]"#)
                .unwrap();
        assert_eq!(ws[0].id, u64::MAX);
        assert!(acknowledged(br#"{"success":false}"#, false).is_err());
        assert!(acknowledged(br#"[{"success":false,"error":"invalid command"}]"#, true).is_err());
        assert!(acknowledged(b"[]", true).is_err());
    }
    #[test]
    fn command_arguments_are_quoted() {
        assert_eq!(
            command(&Action::RenameWorkspace("work; 日本語 \"one\"".into())).unwrap(),
            r#"rename workspace to "work; 日本語 "'"'"one"'"'"""#
        );
        let target = WorkspaceTarget {
            id: None,
            number: 7,
            name: "7: web".into(),
        };
        assert_eq!(
            command(&Action::FocusWorkspace(target)).unwrap(),
            "workspace number 7"
        );
        assert!(command(&Action::RenameWorkspace("".into())).is_err());
        assert!(command(&Action::RenameWorkspace("bad\ncommand".into())).is_err());
    }
}
