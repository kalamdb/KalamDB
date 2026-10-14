//! Similarity metric used by vector indexes.

use serde::{Deserialize, Serialize};

/// Similarity metric used by vector indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorMetric {
    #[default]
    Cosine,
    L2,
    Dot,
}
