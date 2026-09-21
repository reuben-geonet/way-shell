use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Workspace {
    pub id: u64,
    pub num: i32,
    pub name: String,
    pub output: Option<String>,
    #[serde(default)]
    pub urgent: bool,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub visible: bool,
    #[serde(default)]
    pub empty: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Output {
    pub name: String,
    pub make: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub current_workspace: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceTarget {
    pub id: Option<u64>,
    pub number: i32,
    pub name: String,
}

impl From<&Workspace> for WorkspaceTarget {
    fn from(workspace: &Workspace) -> Self {
        Self {
            id: Some(workspace.id),
            number: workspace.num,
            name: workspace.name.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    FocusWorkspace(WorkspaceTarget),
    RenameWorkspace(String),
    MoveWorkspaceToOutput(String),
    MoveWindowToWorkspace(WorkspaceTarget),
}
