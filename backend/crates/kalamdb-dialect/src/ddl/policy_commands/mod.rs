use kalamdb_commons::{models::NamespaceId, TableId};
use sqlparser::ast::{ObjectName, Owner, Statement};
use sqlparser::dialect::PostgreSqlDialect;

use crate::{ddl::DdlResult, parser::utils::parse_sql_statements};

mod alter_policy_statement;
mod alter_policy_operation;
mod create_policy_statement;
mod drop_policy_statement;
mod policy_command;
mod policy_target;

pub use alter_policy_operation::AlterPolicyOperation;
pub use alter_policy_statement::AlterPolicyStatement;
pub use create_policy_statement::CreatePolicyStatement;
pub use drop_policy_statement::DropPolicyStatement;
pub use policy_command::PolicyCommand;
pub use policy_target::PolicyTarget;

fn parse_one(sql: &str) -> DdlResult<Statement> {
    let dialect = PostgreSqlDialect {};
    let mut statements = parse_sql_statements(sql, &dialect)
        .map_err(|error| format!("failed to parse policy statement: {error}"))?;
    if statements.len() != 1 {
        return Err("policy DDL requires exactly one statement".to_string());
    }
    Ok(statements.remove(0))
}

fn resolve_table_id(name: &ObjectName, default_namespace: &NamespaceId) -> DdlResult<TableId> {
    let identifiers = name
        .0
        .iter()
        .map(|part| {
            part.as_ident()
                .map(|ident| ident.value.as_str())
                .ok_or_else(|| "policy table names must contain only identifiers".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;

    match identifiers.as_slice() {
        [table] => TableId::try_from_strings(default_namespace.as_str(), table),
        [namespace, table] => TableId::try_from_strings(namespace, table),
        _ => Err("policy table name must be <table> or <namespace>.<table>".to_string()),
    }
}

fn parse_targets(owners: Option<Vec<Owner>>) -> DdlResult<Vec<PolicyTarget>> {
    owners
        .unwrap_or_else(|| vec![Owner::Ident(sqlparser::ast::Ident::new("PUBLIC"))])
        .into_iter()
        .map(PolicyTarget::try_from)
        .collect()
}
