use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "storage")]
use crate::StorageKey;
use crate::TableId;

/// Stable identifier for one policy on one table.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PolicyId(String);

impl PolicyId {
    pub fn new(table_id: TableId, policy_name: impl AsRef<str>) -> Result<Self, String> {
        Self::from_parts(&table_id.full_name(), policy_name.as_ref())
    }

    pub fn from_parts(table_id: &str, policy_name: &str) -> Result<Self, String> {
        if table_id.is_empty() || !table_id.contains('.') {
            return Err("policy table id must be namespace-qualified".to_string());
        }
        if policy_name.is_empty() || policy_name.contains(':') {
            return Err("policy name must be non-empty and cannot contain ':'".to_string());
        }
        Ok(Self(format!("{table_id}:{policy_name}")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PolicyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for PolicyId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[cfg(feature = "storage")]
impl StorageKey for PolicyId {
    fn storage_key(&self) -> Vec<u8> {
        self.0.as_bytes().to_vec()
    }

    fn from_storage_key(bytes: &[u8]) -> Result<Self, String> {
        let value = String::from_utf8(bytes.to_vec()).map_err(|error| error.to_string())?;
        let (table_id, policy_name) =
            value.rsplit_once(':').ok_or_else(|| "invalid policy storage key".to_string())?;
        Self::from_parts(table_id, policy_name)
    }
}
