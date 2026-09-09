//! In-memory contract snapshot produced by the canonical compiler.

use std::collections::{BTreeMap, BTreeSet};

use arrow::datatypes::DataType;
use kalamdb_commons::{
    models::{RoutineId, RoutineSecurityMode, TypeId},
    KalamDataType,
};

use crate::ddl::ExecuteGrantee;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractField {
    pub name:      String,
    pub type_name: String,
    pub type_id:   Option<TypeId>,
    pub data_type: Option<KalamDataType>,
    pub is_array:  bool,
    pub not_null:  bool,
    pub nonempty:  bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractTypeKind {
    ImplicitTableRow {
        table_id: String,
        fields:   Vec<ContractField>,
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
    /// Implicit payload type for a topic: a tagged union of its `ADD SOURCE` tables.
    TopicPayload {
        topic_id: String,
        sources:  Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractType {
    pub type_id: TypeId,
    pub schema:  String,
    pub name:    String,
    pub kind:    ContractTypeKind,
    pub arrow:   DataType,
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
    pub table_id:     String,
    pub schema:       String,
    pub name:         String,
    pub kind:         ContractTableKind,
    pub row_type_id:  TypeId,
    pub row_alias_id: Option<TypeId>,
    pub fields:       Vec<ContractField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractRoutine {
    pub routine_id:  RoutineId,
    pub schema:      String,
    pub name:        String,
    pub parameters:  Vec<ContractField>,
    pub return_type: Option<ContractField>,
    pub language:    Option<String>,
    pub security:    RoutineSecurityMode,
    pub body:        Option<String>,
    pub grants:      BTreeSet<ExecuteGrantee>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractTrigger {
    pub trigger_id:       String,
    pub topic_id:         String,
    pub routine_id:       String,
    pub principal:        String,
    pub start_from:       String,
    pub retries:          i32,
    pub retry_backoff_ms: i64,
    pub concurrency:      i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContractSnapshot {
    pub schemas:  BTreeSet<String>,
    pub tables:   BTreeMap<String, ContractTable>,
    pub types:    BTreeMap<String, ContractType>,
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
