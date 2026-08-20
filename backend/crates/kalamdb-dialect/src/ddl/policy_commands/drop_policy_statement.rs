use kalamdb_commons::{models::NamespaceId, TableId};
use sqlparser::ast::{DropBehavior, DropPolicy, Statement};

use super::{parse_one, resolve_table_id};
use crate::ddl::DdlResult;

/// Validated `DROP POLICY` statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropPolicyStatement {
    pub policy_name: String,
    pub table_id: TableId,
    pub if_exists: bool,
}

impl DropPolicyStatement {
    pub fn parse(sql: &str, default_namespace: &NamespaceId) -> DdlResult<Self> {
        let Statement::DropPolicy(DropPolicy {
            if_exists,
            name,
            table_name,
            drop_behavior,
        }) = parse_one(sql)?
        else {
            return Err("expected DROP POLICY statement".to_string());
        };

        if drop_behavior == Some(DropBehavior::Cascade) {
            return Err("DROP POLICY CASCADE is not supported".to_string());
        }

        Ok(Self {
            policy_name: name.value,
            table_id: resolve_table_id(&table_name, default_namespace)?,
            if_exists,
        })
    }
}
