use kalamdb_commons::{models::NamespaceId, TableId};
use sqlparser::ast::{CreatePolicy, CreatePolicyCommand, CreatePolicyType, Statement};

use super::{parse_one, parse_targets, resolve_table_id, PolicyCommand, PolicyTarget};
use crate::ddl::DdlResult;

/// Validated, PostgreSQL-shaped `CREATE POLICY` statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePolicyStatement {
    pub policy_name: String,
    pub table_id: TableId,
    pub command: PolicyCommand,
    pub targets: Vec<PolicyTarget>,
    pub using_sql: Option<String>,
    pub with_check_sql: Option<String>,
    pub original_sql: String,
}

impl CreatePolicyStatement {
    pub fn parse(sql: &str, default_namespace: &NamespaceId) -> DdlResult<Self> {
        let Statement::CreatePolicy(CreatePolicy {
            name,
            table_name,
            policy_type,
            command,
            to,
            using,
            with_check,
        }) = parse_one(sql)?
        else {
            return Err("expected CREATE POLICY statement".to_string());
        };

        if policy_type == Some(CreatePolicyType::Restrictive) {
            return Err("RESTRICTIVE policies are not supported".to_string());
        }

        let command = match command {
            None | Some(CreatePolicyCommand::All) => PolicyCommand::All,
            Some(CreatePolicyCommand::Select) => PolicyCommand::Select,
            Some(CreatePolicyCommand::Insert) => PolicyCommand::Insert,
            Some(CreatePolicyCommand::Update) => PolicyCommand::Update,
            Some(CreatePolicyCommand::Delete) => PolicyCommand::Delete,
        };
        if command == PolicyCommand::Insert && using.is_some() {
            return Err("INSERT policies cannot define USING".to_string());
        }
        if command == PolicyCommand::Select && with_check.is_some() {
            return Err("SELECT policies cannot define WITH CHECK".to_string());
        }
        if command == PolicyCommand::Delete && with_check.is_some() {
            return Err("DELETE policies cannot define WITH CHECK".to_string());
        }

        Ok(Self {
            policy_name: name.value,
            table_id: resolve_table_id(&table_name, default_namespace)?,
            command,
            targets: parse_targets(to)?,
            using_sql: using.map(|expr| expr.to_string()),
            with_check_sql: with_check.map(|expr| expr.to_string()),
            original_sql: sql.trim().to_string(),
        })
    }
}
