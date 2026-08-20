use kalamdb_commons::{models::NamespaceId, TableId};
use sqlparser::ast::{AlterPolicy, AlterPolicyOperation as SqlAlterPolicyOperation, Statement};

use super::{parse_one, parse_targets, resolve_table_id, AlterPolicyOperation};
use crate::ddl::DdlResult;

/// Validated `ALTER POLICY` statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlterPolicyStatement {
    pub policy_name: String,
    pub table_id: TableId,
    pub operation: AlterPolicyOperation,
}

impl AlterPolicyStatement {
    pub fn parse(sql: &str, default_namespace: &NamespaceId) -> DdlResult<Self> {
        let Statement::AlterPolicy(AlterPolicy { name, table_name, operation }) = parse_one(sql)?
        else {
            return Err("expected ALTER POLICY statement".to_string());
        };

        let operation = match operation {
            SqlAlterPolicyOperation::Rename { new_name } => {
                AlterPolicyOperation::Rename { new_name: new_name.value }
            },
            SqlAlterPolicyOperation::Apply { to, using, with_check } => {
                AlterPolicyOperation::Apply {
                    targets: to.map(|owners| parse_targets(Some(owners))).transpose()?,
                    using_sql: using.map(|expr| expr.to_string()),
                    with_check_sql: with_check.map(|expr| expr.to_string()),
                }
            },
        };

        Ok(Self {
            policy_name: name.value,
            table_id: resolve_table_id(&table_name, default_namespace)?,
            operation,
        })
    }
}
