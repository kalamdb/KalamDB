//! Catalog definition of a foreign key stored on a table.
//!
//! The constraint is checked when a row is inserted: a non-null key must match
//! a live row in the referenced table. Referential actions are not applied.

use serde::{Deserialize, Serialize};

use crate::{SchemaId, TableName};

/// What a foreign key says should happen to referencing rows.
///
/// The code is stored and reported through `pg_constraint`. Deletes and updates
/// of the referenced row are not rewritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ForeignKeyAction {
    #[default]
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl ForeignKeyAction {
    /// PostgreSQL `confupdtype` / `confdeltype` code.
    pub fn pg_code(self) -> &'static str {
        match self {
            Self::NoAction => "a",
            Self::Restrict => "r",
            Self::Cascade => "c",
            Self::SetNull => "n",
            Self::SetDefault => "d",
        }
    }

    fn is_no_action(&self) -> bool {
        matches!(self, Self::NoAction)
    }
}

/// Foreign key stored on [`super::TableDefinition`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKeyDefinition {
    pub name: String,
    pub columns: Vec<String>,
    pub referenced_namespace: SchemaId,
    pub referenced_table: TableName,
    pub referenced_columns: Vec<String>,
    #[serde(default, skip_serializing_if = "ForeignKeyAction::is_no_action")]
    pub on_update: ForeignKeyAction,
    #[serde(default, skip_serializing_if = "ForeignKeyAction::is_no_action")]
    pub on_delete: ForeignKeyAction,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn articles_author_key() -> ForeignKeyDefinition {
        ForeignKeyDefinition {
            name: "articles_author_foreign".to_string(),
            columns: vec!["author".to_string()],
            referenced_namespace: SchemaId::new("directus"),
            referenced_table: TableName::new("authors"),
            referenced_columns: vec!["id".to_string()],
            on_update: ForeignKeyAction::NoAction,
            on_delete: ForeignKeyAction::Cascade,
        }
    }

    #[test]
    fn stored_foreign_key_without_actions_is_no_action() {
        let json = r#"{
            "name": "articles_author_foreign",
            "columns": ["author"],
            "referenced_namespace": "directus",
            "referenced_table": "authors",
            "referenced_columns": ["id"]
        }"#;
        let foreign_key: ForeignKeyDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(foreign_key.on_update, ForeignKeyAction::NoAction);
        assert_eq!(foreign_key.on_delete, ForeignKeyAction::NoAction);
        assert_eq!(foreign_key.on_delete.pg_code(), "a");
    }

    #[test]
    fn cascade_delete_round_trips_and_omits_no_action() {
        let encoded = serde_json::to_string(&articles_author_key()).unwrap();
        assert!(encoded.contains("cascade"), "{encoded}");
        assert!(!encoded.contains("on_update"), "{encoded}");
        let decoded: ForeignKeyDefinition = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.on_delete.pg_code(), "c");
        assert_eq!(decoded.on_update.pg_code(), "a");
    }
}
