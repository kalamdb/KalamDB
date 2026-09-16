//! A registry pointer; process state remains authoritative in instance.json.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TrackedServer {
    pub instance_path: PathBuf,
    pub folder:        PathBuf,
}
