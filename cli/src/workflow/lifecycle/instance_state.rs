use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceState {
    Running,
    Stopped,
    Unavailable,
    NotChecked,
    Reachable,
    Unreachable,
}

impl InstanceState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Stopped => "Stopped",
            Self::Unavailable => "Unavailable",
            Self::NotChecked => "Not checked",
            Self::Reachable => "Reachable",
            Self::Unreachable => "Unreachable",
        }
    }
}
