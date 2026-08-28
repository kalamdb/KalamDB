use std::collections::BTreeMap;

use sqlparser::{
    ast::{CreatePolicy, CreatePolicyCommand, CreatePolicyType, Owner, Statement},
    dialect::PostgreSqlDialect,
    parser::Parser,
};

use crate::{
    diff::SchemaDiffError,
    model::{policy_key, Policy, PolicyCommand, PolicyTarget, Schema, Table, TableKind},
    sql::{eq_ci, normalize_ident_key, normalize_object_key, normalize_sql_fragment, word_spans},
};

pub(super) fn is_create_policy(sql: &str) -> bool {
    let words = word_spans(sql);

    words.len() >= 2 && eq_ci(words[0].text, "CREATE") && eq_ci(words[1].text, "POLICY")
}

pub(super) fn parse_create_policy(path: &str, sql: &str) -> Result<Policy, SchemaDiffError> {
    let dialect = PostgreSqlDialect {};
    let parsed = Parser::parse_sql(&dialect, sql).map_err(|source| {
        policy_parse_error(
            path,
            &format!("failed to parse CREATE POLICY:\n{sql}\nparser error: {source}"),
        )
    })?;

    let Some(Statement::CreatePolicy(CreatePolicy {
        name,
        table_name,
        policy_type,
        command,
        to,
        using,
        with_check,
    })) = parsed.into_iter().next()
    else {
        return Err(policy_parse_error(path, "expected CREATE POLICY statement"));
    };

    if policy_type == Some(CreatePolicyType::Restrictive) {
        return Err(policy_parse_error(path, "RESTRICTIVE policies are not supported"));
    }

    let command = match command {
        None | Some(CreatePolicyCommand::All) => PolicyCommand::All,
        Some(CreatePolicyCommand::Select) => PolicyCommand::Select,
        Some(CreatePolicyCommand::Insert) => PolicyCommand::Insert,
        Some(CreatePolicyCommand::Update) => PolicyCommand::Update,
        Some(CreatePolicyCommand::Delete) => PolicyCommand::Delete,
    };
    let using_sql = using.map(|expr| normalize_policy_expr(&expr.to_string()));
    let with_check_sql = with_check.map(|expr| normalize_policy_expr(&expr.to_string()));

    validate_policy_clauses(path, command, using_sql.as_deref(), with_check_sql.as_deref())?;

    let name_sql = name.value;
    let name_key = normalize_ident_key(&name_sql);
    let table_sql = table_name.to_string();
    let table_key = normalize_object_key(&table_sql);

    Ok(Policy {
        key: policy_key(&table_key, &name_key),
        name_sql,
        name_key,
        table_sql,
        table_key,
        command,
        targets: parse_policy_targets(path, to)?,
        using_sql,
        with_check_sql,
    })
}

pub(super) fn attach_policies(
    path: &str,
    schema: &mut Schema,
    pending_policies: Vec<Policy>,
) -> Result<(), SchemaDiffError> {
    for mut policy in pending_policies {
        resolve_policy_table(path, &schema.tables, &mut policy)?;

        let table = schema.tables.get(&policy.table_key).expect("policy table existence checked");

        if !table.is_shared() {
            let kind = table
                .kind
                .map(|kind| match kind {
                    TableKind::User => "USER",
                    TableKind::Shared => "SHARED",
                    TableKind::Stream => "STREAM",
                })
                .unwrap_or("SHARED");

            return Err(policy_parse_error(
                path,
                &format!(
                    "CREATE POLICY {} targets {}, but row-level security policies are supported \
                     only on shared tables (found {kind} table)",
                    policy.name_sql, policy.table_sql
                ),
            ));
        }

        if schema.policies.contains_key(&policy.key) {
            return Err(policy_parse_error(
                path,
                &format!("duplicate policy {} on table {}", policy.name_sql, policy.table_sql),
            ));
        }

        schema.policies.insert(policy.key.clone(), policy);
    }

    Ok(())
}

fn parse_policy_targets(
    path: &str,
    owners: Option<Vec<Owner>>,
) -> Result<Vec<PolicyTarget>, SchemaDiffError> {
    let Some(owners) = owners else {
        return Ok(vec![PolicyTarget::Public]);
    };

    if owners.is_empty() {
        return Ok(vec![PolicyTarget::Public]);
    }

    owners
        .into_iter()
        .map(|owner| {
            let Owner::Ident(identifier) = owner else {
                return Err(policy_parse_error(
                    path,
                    &format!("unsupported policy target '{owner}'"),
                ));
            };

            PolicyTarget::parse(&identifier.value)
                .map_err(|message| policy_parse_error(path, &message))
        })
        .collect()
}

fn validate_policy_clauses(
    path: &str,
    command: PolicyCommand,
    using_sql: Option<&str>,
    with_check_sql: Option<&str>,
) -> Result<(), SchemaDiffError> {
    match command {
        PolicyCommand::Insert if using_sql.is_some() => {
            Err(policy_parse_error(path, "INSERT policies cannot define USING"))
        },
        PolicyCommand::Select if with_check_sql.is_some() => {
            Err(policy_parse_error(path, "SELECT policies cannot define WITH CHECK"))
        },
        PolicyCommand::Delete if with_check_sql.is_some() => {
            Err(policy_parse_error(path, "DELETE policies cannot define WITH CHECK"))
        },
        _ => Ok(()),
    }
}

fn resolve_policy_table(
    path: &str,
    tables: &BTreeMap<String, Table>,
    policy: &mut Policy,
) -> Result<(), SchemaDiffError> {
    if tables.contains_key(&policy.table_key) {
        if let Some(table) = tables.get(&policy.table_key) {
            policy.table_sql = table.name_sql.clone();
        }
        return Ok(());
    }

    if !policy.table_key.contains('.') {
        let default_key = format!("default.{}", policy.table_key);
        if tables.contains_key(&default_key) {
            policy.table_key = default_key;
            if let Some(table) = tables.get(&policy.table_key) {
                policy.table_sql = table.name_sql.clone();
            }
            policy.refresh_key();
            return Ok(());
        }

        let qualified_matches = tables
            .keys()
            .filter(|key| key.rsplit('.').next() == Some(policy.table_key.as_str()))
            .cloned()
            .collect::<Vec<_>>();

        if qualified_matches.len() == 1 {
            policy.table_key = qualified_matches[0].clone();
            if let Some(table) = tables.get(&policy.table_key) {
                policy.table_sql = table.name_sql.clone();
            }
            policy.refresh_key();
            return Ok(());
        }

        if !qualified_matches.is_empty() {
            return Err(policy_parse_error(
                path,
                &format!(
                    "CREATE POLICY {} table {} is not defined in schema.sql; found {}. Use the \
                     qualified table name.",
                    policy.name_sql,
                    policy.table_sql,
                    qualified_matches.join(", ")
                ),
            ));
        }
    }

    Err(policy_parse_error(
        path,
        &format!(
            "CREATE POLICY {} table {} must be defined in schema.sql",
            policy.name_sql, policy.table_sql
        ),
    ))
}

fn normalize_policy_expr(value: &str) -> String {
    let normalized = normalize_sql_fragment(value);
    normalized
        .replace("CURRENT_USER ()", "CURRENT_USER")
        .replace("CURRENT_USER()", "CURRENT_USER")
}

fn policy_parse_error(path: &str, message: &str) -> SchemaDiffError {
    SchemaDiffError::Parse {
        message: format!("{path}: {message}"),
    }
}
