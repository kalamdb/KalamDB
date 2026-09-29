//! Opaque identifier for a catalog type.
//!
//! Live `CREATE TYPE` allocates a snowflake id that survives `RENAME` and
//! `SET SCHEMA`. The compiler may still use [`TypeId::from_parts`] (`schema.name`)
//! when no identity manifest is present.

use std::fmt;
#[cfg(feature = "storage")]
use std::sync::LazyLock;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::namespace_id::NamespaceId;
#[cfg(feature = "storage")]
use crate::ids::SnowflakeGenerator;
#[cfg(feature = "storage")]
use crate::StorageKey;

#[cfg(feature = "storage")]
static TYPE_ID_GENERATOR: LazyLock<SnowflakeGenerator> =
    LazyLock::new(|| SnowflakeGenerator::new(1));

/// Catalog type primary key. Not a SQL name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TypeId(String);

impl TypeId {
    /// Creates a type id. Panics if empty.
    #[inline]
    pub fn new(id: impl Into<String>) -> Self {
        match Self::try_new(id) {
            Ok(id) => id,
            Err(error) => panic!("{error}"),
        }
    }

    pub fn try_new(id: impl Into<String>) -> Result<Self, String> {
        let id = id.into();
        if id.is_empty() {
            return Err("TypeId cannot be empty".to_string());
        }
        Ok(Self(id))
    }

    /// Allocate a rename-safe incarnation id for live `CREATE TYPE`.
    #[cfg(feature = "storage")]
    pub fn generate() -> Self {
        let id = TYPE_ID_GENERATOR.next_id().expect("snowflake type id");
        Self(id.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    /// Qualified SQL alias used by contract compile when no identity manifest exists.
    pub fn from_parts(namespace: Option<&NamespaceId>, name: &str) -> Self {
        let name = name.trim();
        assert!(!name.is_empty(), "TypeId cannot be empty");
        match namespace {
            Some(namespace) => Self::new(format!("{}.{name}", namespace.as_str())),
            None => Self::new(name),
        }
    }

    /// Stable PostgreSQL object identifier for this TypeId (not the SQL name).
    /// User-defined OIDs stay in a high range so builtin pg types are not reused.
    pub fn pg_oid(&self) -> u32 {
        let mut hash: u32 = 2_166_136_261;
        for byte in self.0.as_bytes() {
            hash ^= u32::from(*byte);
            hash = hash.wrapping_mul(16_777_619);
        }
        16_384 + (hash % 2_000_000_000)
    }
}

impl fmt::Display for TypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for TypeId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for TypeId {
    fn from(id: String) -> Self {
        Self::new(id)
    }
}

impl From<&str> for TypeId {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

#[cfg(feature = "storage")]
impl StorageKey for TypeId {
    fn storage_key(&self) -> Vec<u8> {
        self.0.as_bytes().to_vec()
    }

    fn from_storage_key(bytes: &[u8]) -> Result<Self, String> {
        let value = String::from_utf8(bytes.to_vec()).map_err(|error| error.to_string())?;
        Self::try_new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_id_from_parts() {
        assert_eq!(
            TypeId::from_parts(Some(&NamespaceId::new("chat")), "message").as_str(),
            "chat.message"
        );
        assert_eq!(TypeId::from_parts(None, "address").as_str(), "address");
    }

    #[test]
    #[should_panic(expected = "TypeId cannot be empty")]
    fn type_id_empty_panics() {
        let _ = TypeId::new("");
    }

    #[cfg(feature = "storage")]
    #[test]
    fn generate_is_opaque_and_unique() {
        let a = TypeId::generate();
        let b = TypeId::generate();
        assert_ne!(a, b);
        assert!(!a.as_str().contains('.'));
        assert!(a.as_str().chars().all(|c| c.is_ascii_digit() || c == '-'));
    }
}
