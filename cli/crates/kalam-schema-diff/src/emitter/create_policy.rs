use std::collections::BTreeMap;

use crate::model::{Policy, PolicyCommand, PolicyTarget, Table};

pub(super) fn emit_create_policy(policy: &Policy) -> String {
    let mut sql = format!(
        "CREATE POLICY {} ON {} FOR {} TO {}",
        policy.name_sql,
        policy.table_sql,
        policy.command.as_sql(),
        policy.targets_sql()
    );

    if let Some(using_sql) = &policy.using_sql {
        sql.push_str(" USING (");
        sql.push_str(using_sql);
        sql.push(')');
    }

    if let Some(with_check_sql) = &policy.with_check_sql {
        sql.push_str(" WITH CHECK (");
        sql.push_str(with_check_sql);
        sql.push(')');
    }

    sql.push(';');
    sql
}

pub(super) fn emit_starter_shared_table_policy(table: &Table, out: &mut Vec<String>) {
    out.push(format!(
        "-- shared table {} is FORCE RLS (default-deny). Copy this policy into schema.sql to keep \
         editing it there.",
        table.name_sql
    ));
    out.push(emit_create_policy(&starter_shared_table_policy(table)));
}

fn starter_shared_table_policy(table: &Table) -> Policy {
    let name_sql = format!("{}_all", table.unqualified_name());

    Policy {
        key: String::new(),
        name_sql,
        name_key: String::new(),
        table_sql: table.name_sql.clone(),
        table_key: table.key.clone(),
        command: PolicyCommand::All,
        targets: vec![PolicyTarget::Public],
        using_sql: Some("true".to_string()),
        with_check_sql: Some("true".to_string()),
    }
}

pub(super) fn policies_for_table<'a>(
    policies: &'a BTreeMap<String, Policy>,
    table_key: &str,
) -> Vec<&'a Policy> {
    policies.values().filter(|policy| policy.table_key == table_key).collect()
}
