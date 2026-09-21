//! The Niri JSON-line protocol used by Way Shell (unknown events are ignored).
use crate::wm::{Action, Output, Workspace, WorkspaceTarget};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

const MAX_LINE: usize = 16 * 1024 * 1024;
#[derive(Default)]
pub struct Decoder {
    bytes: Vec<u8>,
}
impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Value>, String> {
        let mut values = Vec::new();
        for part in bytes.split_inclusive(|&byte| byte == b'\n') {
            if part.len() > MAX_LINE - self.bytes.len() {
                return Err("Niri IPC line exceeds 16 MiB".into());
            }
            self.bytes.extend_from_slice(part);
            if part.last() == Some(&b'\n') {
                values.push(
                    serde_json::from_slice(&self.bytes)
                        .map_err(|error| format!("Invalid Niri JSON line: {error}"))?,
                );
                self.bytes.clear();
            }
        }
        Ok(values)
    }
}
pub fn encode(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).expect("JSON value is serializable");
    bytes.push(b'\n');
    bytes
}

#[derive(Debug, Deserialize)]
pub struct NiriWorkspace {
    id: u64,
    idx: u8,
    name: Option<String>,
    output: Option<String>,
    is_active: bool,
    is_focused: bool,
    #[serde(default)]
    is_urgent: bool,
    active_window_id: Option<u64>,
}
impl From<NiriWorkspace> for Workspace {
    fn from(value: NiriWorkspace) -> Self {
        Self {
            id: value.id,
            num: value.idx.into(),
            name: value.name.unwrap_or_else(|| value.idx.to_string()),
            output: value.output,
            urgent: value.is_urgent,
            focused: value.is_focused,
            visible: value.is_active,
            empty: value.active_window_id.is_none(),
        }
    }
}
#[derive(Debug, Deserialize)]
pub enum Response {
    Handled,
    Workspaces(Vec<NiriWorkspace>),
    Outputs(BTreeMap<String, Output>),
}
pub fn response(value: Value) -> Result<Response, String> {
    serde_json::from_value::<Result<Response, String>>(value)
        .map_err(|error| format!("Invalid Niri reply: {error}"))?
}
#[derive(Debug, Deserialize)]
pub enum Event {
    WorkspacesChanged {
        workspaces: Vec<NiriWorkspace>,
    },
    WorkspaceUrgencyChanged {
        id: u64,
        urgent: bool,
    },
    WorkspaceActivated {
        id: u64,
        focused: bool,
    },
    WorkspaceActiveWindowChanged {
        workspace_id: u64,
        active_window_id: Option<u64>,
    },
    #[serde(other)]
    Other,
}

pub fn event(value: Value) -> Result<Event, String> {
    let object = value
        .as_object()
        .filter(|object| object.len() == 1)
        .ok_or("Invalid Niri event envelope")?;
    let key = object.keys().next().unwrap().as_str();
    if !matches!(
        key,
        "WorkspacesChanged"
            | "WorkspaceActivated"
            | "WorkspaceUrgencyChanged"
            | "WorkspaceActiveWindowChanged"
    ) {
        return Ok(Event::Other);
    }
    serde_json::from_value(value).map_err(|e| format!("Invalid Niri event: {e}"))
}

/// Apply an event atomically. False requests a full refresh for an unknown ID.
pub fn update(workspaces: &mut Vec<Workspace>, event: Event) -> bool {
    match event {
        Event::WorkspacesChanged { workspaces: values } => {
            *workspaces = values.into_iter().map(Into::into).collect();
        }
        Event::WorkspaceActivated { id, focused } => {
            let Some(target) = workspaces.iter().find(|workspace| workspace.id == id) else {
                return false;
            };
            let output = target.output.clone();
            for workspace in workspaces.iter_mut() {
                if workspace.output == output {
                    workspace.visible = workspace.id == id;
                }
                if focused {
                    workspace.focused = workspace.id == id;
                }
            }
        }
        Event::WorkspaceUrgencyChanged { id, urgent } => {
            let Some(workspace) = workspaces.iter_mut().find(|workspace| workspace.id == id) else {
                return false;
            };
            workspace.urgent = urgent;
        }
        Event::WorkspaceActiveWindowChanged {
            workspace_id,
            active_window_id,
        } => {
            let Some(workspace) = workspaces
                .iter_mut()
                .find(|workspace| workspace.id == workspace_id)
            else {
                return false;
            };
            workspace.empty = active_window_id.is_none();
        }
        Event::Other => (),
    }
    workspaces.sort_by_key(|workspace| workspace.num);
    true
}
fn name(value: &str) -> Result<(), String> {
    if value.is_empty() || value.chars().any(char::is_control) {
        Err("Workspace and output names must be nonempty and contain no control characters".into())
    } else {
        Ok(())
    }
}
fn reference(target: &WorkspaceTarget) -> Result<Value, String> {
    if let Some(id) = target.id {
        return Ok(json!({"Id": id}));
    }
    name(&target.name)?;
    if target.name.bytes().all(|byte| byte.is_ascii_digit()) {
        let index: u8 = target
            .name
            .parse()
            .map_err(|_| "Niri workspace index must be in 1..255")?;
        if index == 0 {
            return Err("Niri workspace index must be in 1..255".into());
        }
        Ok(json!({"Index": index}))
    } else {
        Ok(json!({"Name": target.name}))
    }
}
pub fn command(action: &Action) -> Result<Value, String> {
    let action = match action {
        Action::FocusWorkspace(target) => {
            json!({"FocusWorkspace":{"reference":reference(target)?}})
        }
        Action::MoveWindowToWorkspace(target) => {
            json!({"MoveWindowToWorkspace":{"window_id":null,"reference":reference(target)?,"focus":false}})
        }
        Action::RenameWorkspace(value) => {
            name(value)?;
            json!({"SetWorkspaceName":{"name":value,"workspace":null}})
        }
        Action::MoveWorkspaceToOutput(value) => {
            name(value)?;
            json!({"MoveWorkspaceToMonitor":{"output":value,"reference":null}})
        }
    };
    Ok(json!({"Action":action}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn workspaces() -> Vec<Workspace> {
        let Response::Workspaces(values) = response(
            serde_json::from_slice(include_bytes!(
                "../../../tests/fixtures/niri-workspaces.json"
            ))
            .unwrap(),
        )
        .unwrap() else {
            panic!()
        };
        values.into_iter().map(Into::into).collect()
    }
    #[test]
    fn fixtures_preserve_identity_and_focus_on_multiple_outputs() {
        let mut values = workspaces();
        let named = &values[0];
        assert_eq!((named.id, named.num), (4294967313, 2));
        assert_eq!(named.name, "2: 日本語");
        assert!(named.visible && named.urgent && !named.focused && !named.empty);
        assert!(values[1].focused && values[1].visible && values[1].empty);
        assert_eq!(values[1].name, "1");
        assert!(update(
            &mut values,
            Event::WorkspaceActivated {
                id: 4294967313,
                focused: true
            }
        ));
        assert_eq!(values.iter().filter(|value| value.focused).count(), 1);
        assert_eq!(values.iter().filter(|value| value.visible).count(), 2);
        assert!(update(
            &mut values,
            Event::WorkspaceUrgencyChanged {
                id: 4294967313,
                urgent: false
            }
        ));
        assert!(values.iter().all(|value| !value.urgent));
        assert!(!update(
            &mut values,
            Event::WorkspaceActivated {
                id: 0,
                focused: true
            }
        ));
        let Response::Outputs(outputs) = response(
            serde_json::from_slice(include_bytes!("../../../tests/fixtures/niri-outputs.json"))
                .unwrap(),
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs["DP-1"].serial.as_deref(), Some("example"));
        assert_eq!(outputs["eDP-1"].serial, None);
    }
    #[test]
    fn actions_use_stable_ids_and_encode_names() {
        for value in ["2: 日本語", "a \"quote\" and \\slash"] {
            let command = command(&Action::FocusWorkspace(WorkspaceTarget {
                id: None,
                number: 0,
                name: value.into(),
            }))
            .unwrap();
            assert_eq!(
                command["Action"]["FocusWorkspace"]["reference"],
                json!({"Name":value})
            );
        }
        let target = WorkspaceTarget {
            id: Some(u64::MAX),
            number: 1,
            name: "1".into(),
        };
        assert_eq!(
            command(&Action::MoveWindowToWorkspace(target)).unwrap()["Action"]["MoveWindowToWorkspace"]
                ["reference"],
            json!({"Id":u64::MAX})
        );
        for value in ["0", "256", "999999999999999999999999", ""] {
            assert!(
                command(&Action::FocusWorkspace(WorkspaceTarget {
                    id: None,
                    number: 0,
                    name: value.into()
                }))
                .is_err()
            );
        }
        assert_eq!(
            command(&Action::FocusWorkspace(WorkspaceTarget {
                id: None,
                number: 7,
                name: "007".into()
            }))
            .unwrap()["Action"]["FocusWorkspace"]["reference"],
            json!({"Index":7})
        );
        let renamed = "a \"quote\" and \\slash";
        assert_eq!(
            command(&Action::RenameWorkspace(renamed.into())).unwrap()["Action"]["SetWorkspaceName"]
                ["name"],
            renamed
        );
        assert_eq!(
            command(&Action::MoveWorkspaceToOutput("DP-1".into())).unwrap()["Action"]["MoveWorkspaceToMonitor"]
                ["output"],
            "DP-1"
        );
    }
    #[test]
    fn fragments_errors_and_unknown_events() {
        let all = b"{\"Ok\":\"Handled\"}\n{\"Err\":\"denied\"}\n";
        for size in 1..all.len() {
            let mut decoder = Decoder::default();
            let values: Vec<_> = all
                .chunks(size)
                .flat_map(|part| decoder.push(part).unwrap())
                .collect();
            assert_eq!(values.len(), 2);
            assert!(matches!(response(values[0].clone()), Ok(Response::Handled)));
            assert_eq!(response(values[1].clone()).unwrap_err(), "denied");
        }
        assert!(Decoder::default().push(b"invalid\n").is_err());
        assert!(response(json!({"Ok":{"Outputs":[]}})).is_err());
        assert!(event(json!({"WorkspaceActivated":{"id":1,"focused":"bad"}})).is_err());
        assert!(matches!(
            event(json!({"FutureEvent":{"field":7}})).unwrap(),
            Event::Other
        ));
        let mut decoder = Decoder {
            bytes: vec![b' '; MAX_LINE],
        };
        assert!(decoder.push(b" ").is_err());
    }
}
