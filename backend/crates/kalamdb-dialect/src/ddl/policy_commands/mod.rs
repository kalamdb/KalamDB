use kalamdb_commons::{models::SchemaId, Role, TableId};
use sqlparser::ast::{ObjectName, Owner, Statement};

use crate::{ddl::DdlResult, parser::KalamParser};

mod alter_policy_operation;
mod alter_policy_statement;
mod create_policy_statement;
mod drop_policy_statement;

pub use alter_policy_operation::AlterPolicyOperation;
pub use alter_policy_statement::AlterPolicyStatement;
pub use create_policy_statement::CreatePolicyStatement;
pub use drop_policy_statement::DropPolicyStatement;
pub use kalamdb_commons::{PolicyCommand, PolicyTarget};

fn parse_one(sql: &str) -> DdlResult<Statement> {
    let mut statements = KalamParser::parse(sql)
        .map_err(|error| format!("failed to parse policy statement: {error}"))?;
    if statements.len() != 1 {
        return Err("policy DDL requires exactly one statement".to_string());
    }
    Ok(statements.remove(0))
}

fn resolve_table_id(name: &ObjectName, default_namespace: &SchemaId) -> DdlResult<TableId> {
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
        [table] => TableId::try_from_namespace(default_namespace.clone(), table),
        [namespace, table] => TableId::try_from_strings(namespace, table),
        _ => Err("policy table name must be <table> or <namespace>.<table>".to_string()),
    }
}

fn parse_targets(owners: Option<Vec<Owner>>) -> DdlResult<Vec<PolicyTarget>> {
    owners
        .unwrap_or_else(|| vec![Owner::Ident(sqlparser::ast::Ident::new("PUBLIC"))])
        .into_iter()
        .map(|owner| {
            let Owner::Ident(identifier) = owner else {
                return Err(format!("unsupported policy target '{owner}'"));
            };
            if identifier.value.eq_ignore_ascii_case("PUBLIC") {
                return Ok(PolicyTarget::Public);
            }
            Role::from_str_opt(&identifier.value)
                .map(PolicyTarget::Role)
                .ok_or_else(|| format!("unsupported policy target '{}'", identifier.value))
        })
        .collect()
}
