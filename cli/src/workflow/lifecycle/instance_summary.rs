use std::path::PathBuf;

use serde::Serialize;

use super::{instance_kind::InstanceKind, instance_state::InstanceState};

/// Safe display metadata. Tokens never enter this model.
#[derive(Debug, Clone, Serialize)]
pub struct InstanceSummary {
    pub name:     String,
    pub kind:     InstanceKind,
    pub state:    InstanceState,
    pub url:      Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder:   Option<PathBuf>,
    pub global:   bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user:     Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth:     Option<String>,
    #[serde(skip)]
    pub aliases:  Vec<String>,
    #[serde(skip)]
    pub identity: String,
}
