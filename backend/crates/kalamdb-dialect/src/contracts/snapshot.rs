//! In-memory contract snapshot produced by the canonical compiler.

use std::collections::{BTreeMap, BTreeSet};

use arrow::datatypes::DataType;
use kalamdb_commons::{
    models::{
        RoutineId, RoutineKind, RoutineNullSafety, RoutineParallelSafety, RoutineParameterMode,
        RoutineSecurityMode, RoutineVolatility, TypeId,
    },
    KalamDataType,
};
use serde::{Deserialize, Serialize};

use crate::ddl::{DomainCheckClause, ExecuteGrantee};

/// Current fresh-start contract and protocol epoch.
pub const CURRENT_CONTRACT_EPOCH: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RoutineCardinality {
    #[default]
    Void,
    Scalar,
    SetOf,
    Table,
}

impl RoutineCardinality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Void => "void",
            Self::Scalar => "scalar",
            Self::SetOf => "setof",
            Self::Table => "table",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContractField {
    pub name: String,
    pub type_name: String,
    pub type_id: Option<TypeId>,
    pub data_type: Option<KalamDataType>,
    pub is_array: bool,
    pub not_null: bool,
    pub nonempty: bool,
    pub primary_key: bool,
    pub primary_key_ordinal: Option<u16>,
    pub has_default: bool,
    pub default_expr: Option<String>,
    pub is_identity: bool,
    pub is_generated: bool,
    pub is_read_only: bool,
    pub param_mode: RoutineParameterMode,
    /// Physical slot. Never reused. 0 until assigned from CREATE order or identity manifest.
    pub slot: i32,
    /// Tombstone after DROP ATTRIBUTE. Slot is never reused.
    pub dropped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractTypeKind {
    ImplicitTableRow {
        table_id: String,
        fields: Vec<ContractField>,
    },
    RowAlias {
        source: TypeId,
    },
    Composite {
        fields: Vec<ContractField>,
    },
    Enum {
        labels: Vec<String>,
    },
    Domain {
        base_type: ContractField,
        collation: Option<String>,
        default_expr: Option<String>,
        not_null: bool,
        checks: Vec<DomainCheckClause>,
    },
    /// Implicit payload type for a topic: a tagged union of its `ADD SOURCE` tables.
    TopicPayload {
        topic_id: String,
        sources: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractType {
    pub type_id: TypeId,
    pub schema: String,
    pub name: String,
    pub kind: ContractTypeKind,
    pub arrow: DataType,
    /// Catalog documentation only; excluded from the canonical contract hash.
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContractTableKind {
    #[default]
    Unspecified,
    User,
    Shared,
    Stream,
}

impl ContractTableKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::User => "user",
            Self::Shared => "shared",
            Self::Stream => "stream",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractTable {
    pub table_id: String,
    pub schema: String,
    pub name: String,
    pub kind: ContractTableKind,
    pub row_type_id: TypeId,
    pub row_alias_id: Option<TypeId>,
    pub primary_key: Vec<String>,
    pub fields: Vec<ContractField>,
}

impl Default for ContractTable {
    fn default() -> Self {
        Self {
            table_id: String::new(),
            schema: String::new(),
            name: String::new(),
            kind: ContractTableKind::Unspecified,
            row_type_id: TypeId::new("public.default"),
            row_alias_id: None,
            primary_key: Vec::new(),
            fields: Vec::new(),
        }
    }
}

impl ContractTable {
    /// Ordered primary-key fields derived from the catalog, never guessed from a field named `id`.
    pub fn ordered_key_fields(&self) -> Vec<&ContractField> {
        let mut keys: Vec<&ContractField> =
            self.fields.iter().filter(|f| !f.dropped && f.primary_key).collect();
        keys.sort_by_key(|f| f.primary_key_ordinal.unwrap_or(u16::MAX));
        keys
    }

    /// Writable fields permitted in an INSERT payload (excludes generated/read-only fields).
    pub fn writable_insert_fields(&self) -> Vec<&ContractField> {
        self.fields.iter().filter(|f| !f.dropped && !f.is_read_only).collect()
    }

    /// Required fields in an INSERT payload (non-null, writable, without default or identity).
    pub fn required_insert_fields(&self) -> Vec<&ContractField> {
        self.fields
            .iter()
            .filter(|f| {
                !f.dropped && !f.is_read_only && f.not_null && !f.has_default && !f.is_identity
            })
            .collect()
    }

    /// Mutable fields permitted in a partial UPDATE patch (excludes keys and read-only fields).
    pub fn patchable_fields(&self) -> Vec<&ContractField> {
        self.fields
            .iter()
            .filter(|f| !f.dropped && !f.primary_key && !f.is_read_only)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractRoutine {
    pub routine_id: RoutineId,
    pub signature: String,
    pub schema: String,
    pub name: String,
    pub kind: RoutineKind,
    pub cardinality: RoutineCardinality,
    pub parameters: Vec<ContractField>,
    pub return_type: Option<ContractField>,
    pub returns_table: Vec<ContractField>,
    pub language: Option<String>,
    pub security: RoutineSecurityMode,
    pub volatility: RoutineVolatility,
    pub null_safety: RoutineNullSafety,
    pub parallel_safety: RoutineParallelSafety,
    pub is_internal: bool,
    pub body: Option<String>,
    pub grants: BTreeSet<ExecuteGrantee>,
    /// Catalog documentation only; excluded from the canonical contract hash.
    pub comment: Option<String>,
}

impl Default for ContractRoutine {
    fn default() -> Self {
        Self {
            routine_id: RoutineId::new("public.default"),
            signature: String::new(),
            schema: String::new(),
            name: String::new(),
            kind: RoutineKind::default(),
            cardinality: RoutineCardinality::Void,
            parameters: Vec::new(),
            return_type: None,
            returns_table: Vec::new(),
            language: None,
            security: RoutineSecurityMode::Invoker,
            volatility: RoutineVolatility::default(),
            null_safety: RoutineNullSafety::default(),
            parallel_safety: RoutineParallelSafety::default(),
            is_internal: false,
            body: None,
            grants: BTreeSet::new(),
            comment: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractTrigger {
    pub trigger_id: String,
    pub topic_id: String,
    pub routine_id: String,
    pub principal: String,
    pub start_from: String,
    pub retries: i32,
    pub retry_backoff_ms: i64,
    pub concurrency: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContractSnapshot {
    pub schemas: BTreeSet<String>,
    pub tables: BTreeMap<String, ContractTable>,
    pub types: BTreeMap<String, ContractType>,
    pub routines: BTreeMap<String, ContractRoutine>,
    pub triggers: BTreeMap<String, ContractTrigger>,
}

/// Wire `_table` tag injected into full topic payloads (`namespace:table`).
pub fn table_payload_tag(table_id: &str) -> String {
    match table_id.rsplit_once('.') {
        Some((schema, name)) => format!("{schema}:{name}"),
        None => table_id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::table_payload_tag;

    #[test]
    fn table_payload_tag_uses_colon_not_dot() {
        assert_eq!(table_payload_tag("chat_demo.messages"), "chat_demo:messages");
        assert_eq!(table_payload_tag("chat_demo.direct_messages"), "chat_demo:direct_messages");
        assert_eq!(table_payload_tag("messages"), "messages");
    }
}
