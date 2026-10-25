//! Scalar `CREATE INDEX` / `DROP INDEX` parsing.
//!
//! Vector indexes stay on the no-parentheses `USING COSINE|L2|DOT` form in
//! [`super::alter_table`]. Parentheses select the scalar catalog path.

use kalamdb_commons::models::{SchemaId, TableName};
use once_cell::sync::Lazy;
use regex::Regex;
use sqlparser::ast::{CreateIndex, Expr, IndexType, ObjectName, ObjectType, Statement};

use crate::{
    ddl::{
        alter_table::{
            resolve_table_reference, resolve_table_reference_from_str, ColumnOperation,
            StandaloneIndexName,
        },
        AlterTableStatement, DdlResult,
    },
    parser::KalamParser,
};

static ALTER_CREATE_SCALAR_INDEX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^\s*ALTER\s+TABLE\s+([a-zA-Z_][\w]*(?:\.[a-zA-Z_][\w]*)?)\s+CREATE\s+(UNIQUE\s+)?INDEX\s+(?:IF\s+NOT\s+EXISTS\s+)?([a-zA-Z_][\w]*)\s*\(([^)]+)\)\s*;?\s*$",
    )
    .unwrap()
});

static ALTER_DROP_SCALAR_INDEX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^\s*ALTER\s+TABLE\s+([a-zA-Z_][\w]*(?:\.[a-zA-Z_][\w]*)?)\s+DROP\s+INDEX\s+(?:IF\s+EXISTS\s+)?([a-zA-Z_][\w]*)\s*;?\s*$",
    )
    .unwrap()
});

static CREATE_INDEX_ON_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^\s*CREATE\s+(UNIQUE\s+)?INDEX\s+(?:IF\s+NOT\s+EXISTS\s+)?([a-zA-Z_][\w]*)\s+ON\s+([a-zA-Z_][\w]*(?:\.[a-zA-Z_][\w]*)?)\s*\(([^)]+)\)\s*;?\s*$",
    )
    .unwrap()
});

/// Parse `ALTER TABLE t CREATE INDEX name (cols)` / `DROP INDEX name`.
///
/// Returns `None` when the statement is not this form (including vector
/// `CREATE INDEX col USING COSINE` which has no parentheses).
pub fn parse_alter_table_scalar_index(
    sql: &str,
    current_namespace: &SchemaId,
) -> DdlResult<Option<AlterTableStatement>> {
    let upper = sql.to_ascii_uppercase();
    if upper.contains("CREATE") && upper.contains(" INDEX") && sql.contains('(') {
        reject_vector_using(sql)?;
    }

    if let Some(caps) = ALTER_CREATE_SCALAR_INDEX_RE.captures(sql) {
        reject_vector_using(sql)?;
        let table_ref = caps
            .get(1)
            .map(|m| m.as_str())
            .ok_or_else(|| "missing table reference in CREATE INDEX".to_string())?;
        let unique = caps.get(2).is_some();
        let if_not_exists = sql.to_ascii_uppercase().contains("IF NOT EXISTS");
        let name = caps
            .get(3)
            .map(|m| m.as_str().to_string())
            .ok_or_else(|| "missing index name in CREATE INDEX".to_string())?;
        let columns = parse_index_columns(
            caps.get(4)
                .map(|m| m.as_str())
                .ok_or_else(|| "missing column list in CREATE INDEX".to_string())?,
        )?;
        let (namespace_id, table_name) =
            resolve_table_reference_from_str(table_ref, current_namespace)?;
        return Ok(Some(AlterTableStatement {
            table_name,
            namespace_id,
            operation: ColumnOperation::CreateScalarIndex {
                name,
                columns,
                unique,
                if_not_exists,
            },
        }));
    }

    if sql.to_ascii_uppercase().contains("DROP VECTOR INDEX") {
        return Ok(None);
    }

    if let Some(caps) = ALTER_DROP_SCALAR_INDEX_RE.captures(sql) {
        let table_ref = caps
            .get(1)
            .map(|m| m.as_str())
            .ok_or_else(|| "missing table reference in DROP INDEX".to_string())?;
        let if_exists = sql.to_ascii_uppercase().contains("IF EXISTS");
        let name = caps
            .get(2)
            .map(|m| m.as_str().to_string())
            .ok_or_else(|| "missing index name in DROP INDEX".to_string())?;
        let (namespace_id, table_name) =
            resolve_table_reference_from_str(table_ref, current_namespace)?;
        return Ok(Some(AlterTableStatement {
            table_name,
            namespace_id,
            operation: ColumnOperation::DropIndex { name, if_exists },
        }));
    }

    Ok(None)
}

/// Parse PostgreSQL-shaped `CREATE [UNIQUE] INDEX name ON table (cols)`.
pub fn parse_create_index_on(
    sql: &str,
    current_namespace: &SchemaId,
) -> DdlResult<AlterTableStatement> {
    if let Ok(Statement::CreateIndex(create)) = KalamParser::parse_one(sql) {
        return create_index_statement(create, current_namespace);
    }
    parse_create_index_on_regex(sql, current_namespace)
}

fn parse_create_index_on_regex(
    sql: &str,
    current_namespace: &SchemaId,
) -> DdlResult<AlterTableStatement> {
    reject_vector_using(sql)?;
    let caps = CREATE_INDEX_ON_RE.captures(sql).ok_or_else(|| {
        "expected CREATE [UNIQUE] INDEX [IF NOT EXISTS] name ON table (columns)".to_string()
    })?;
    let unique = caps.get(1).is_some();
    let if_not_exists = sql.to_ascii_uppercase().contains("IF NOT EXISTS");
    let name = caps
        .get(2)
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| "missing index name in CREATE INDEX".to_string())?;
    let table_ref = caps
        .get(3)
        .map(|m| m.as_str())
        .ok_or_else(|| "missing table reference in CREATE INDEX".to_string())?;
    let columns = parse_index_columns(
        caps.get(4)
            .map(|m| m.as_str())
            .ok_or_else(|| "missing column list in CREATE INDEX".to_string())?,
    )?;
    let (namespace_id, table_name) =
        resolve_table_reference_from_str(table_ref, current_namespace)?;
    Ok(AlterTableStatement {
        table_name,
        namespace_id,
        operation: ColumnOperation::CreateScalarIndex {
            name,
            columns,
            unique,
            if_not_exists,
        },
    })
}

fn parse_index_columns(list: &str) -> DdlResult<Vec<String>> {
    let columns: Vec<String> = list
        .split(',')
        .map(|part| part.trim().trim_matches('"').trim_matches('`').to_string())
        .filter(|part| !part.is_empty())
        .collect();
    if columns.is_empty() {
        return Err("CREATE INDEX requires at least one column".to_string());
    }
    for column in &columns {
        if !column.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            || !column.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(format!("invalid index column name '{column}'"));
        }
    }
    let mut seen = Vec::with_capacity(columns.len());
    for column in &columns {
        if seen.iter().any(|existing: &String| existing.eq_ignore_ascii_case(column)) {
            return Err(format!("duplicate column '{column}' in CREATE INDEX"));
        }
        seen.push(column.clone());
    }
    Ok(columns)
}

/// Parse PostgreSQL `DROP INDEX [CONCURRENTLY] [IF EXISTS] name [, ...]`.
///
/// `CONCURRENTLY`, `CASCADE`, and `RESTRICT` are accepted. KalamDB drops the
/// catalog entry directly, so those clauses do not change the operation.
/// `DROP INDEX name ON table` is the MySQL form and is resolved to that table.
pub fn parse_drop_index(sql: &str, current_namespace: &SchemaId) -> DdlResult<AlterTableStatement> {
    let normalized = without_drop_index_concurrently(sql);
    let statement = KalamParser::parse_one(&normalized).map_err(|err| err.to_string())?;
    let Statement::Drop {
        object_type,
        if_exists,
        names,
        purge,
        temporary,
        table,
        ..
    } = statement
    else {
        return Err("expected DROP INDEX".to_string());
    };
    if object_type != ObjectType::Index {
        return Err("expected DROP INDEX".to_string());
    }
    if purge || temporary {
        return Err("DROP INDEX does not support PURGE or TEMPORARY".to_string());
    }
    if names.is_empty() {
        return Err("DROP INDEX requires an index name".to_string());
    }

    if let Some(table) = table {
        if names.len() != 1 {
            return Err("DROP INDEX ... ON accepts one index name".to_string());
        }
        let index = standalone_index_name(&names[0], current_namespace)?;
        let (namespace_id, table_name) = resolve_table_reference(table, current_namespace)?;
        if names[0].0.len() == 2 && index.namespace_id != namespace_id {
            return Err("DROP INDEX schema does not match the ON table schema".to_string());
        }
        return Ok(AlterTableStatement {
            table_name,
            namespace_id,
            operation: ColumnOperation::DropIndex {
                name: index.name,
                if_exists,
            },
        });
    }

    let mut indexes = Vec::with_capacity(names.len());
    for name in &names {
        indexes.push(standalone_index_name(name, current_namespace)?);
    }
    let namespace_id = indexes[0].namespace_id.clone();
    Ok(AlterTableStatement {
        table_name: TableName::new("drop_index"),
        namespace_id,
        operation: ColumnOperation::DropStandaloneIndex { indexes, if_exists },
    })
}

fn standalone_index_name(
    name: &ObjectName,
    current_namespace: &SchemaId,
) -> DdlResult<StandaloneIndexName> {
    let mut idents = Vec::with_capacity(name.0.len());
    for part in &name.0 {
        let ident = part
            .as_ident()
            .ok_or_else(|| "function-based index names are not supported".to_string())?;
        if ident.value.is_empty() {
            return Err("DROP INDEX requires an index name".to_string());
        }
        idents.push(ident.value.as_str());
    }
    match idents.as_slice() {
        [index] => Ok(StandaloneIndexName {
            namespace_id: current_namespace.clone(),
            name: (*index).to_string(),
        }),
        [schema, index] => Ok(StandaloneIndexName {
            namespace_id: SchemaId::from(*schema),
            name: (*index).to_string(),
        }),
        _ => Err("DROP INDEX name must be index_name or schema.index_name".to_string()),
    }
}

fn without_drop_index_concurrently(sql: &str) -> std::borrow::Cow<'_, str> {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)\bDROP\s+INDEX\s+CONCURRENTLY\b").expect("drop index concurrently regex")
    });
    RE.replace(sql, "DROP INDEX")
}

fn create_index_statement(
    create: CreateIndex,
    current_namespace: &SchemaId,
) -> DdlResult<AlterTableStatement> {
    match &create.using {
        None | Some(IndexType::BTree) => {},
        Some(IndexType::Custom(ident))
            if matches!(ident.value.to_ascii_uppercase().as_str(), "COSINE" | "L2" | "DOT") =>
        {
            return Err("scalar CREATE INDEX cannot mix USING COSINE/L2/DOT; that form is \
                        vector-only"
                .to_string());
        },
        Some(other) => {
            return Err(format!("index method {other} is not supported; use btree"));
        },
    }
    if !create.include.is_empty() {
        return Err("CREATE INDEX INCLUDE columns are not supported".to_string());
    }
    if create.predicate.is_some() {
        return Err("partial CREATE INDEX WHERE predicates are not supported".to_string());
    }
    if create.nulls_distinct == Some(false) {
        return Err("CREATE INDEX NULLS NOT DISTINCT is not supported".to_string());
    }
    if !create.alter_options.is_empty() || !create.index_options.is_empty() {
        return Err("CREATE INDEX storage options are not supported".to_string());
    }

    let (namespace_id, table_name) = resolve_table_reference(create.table_name, current_namespace)?;
    let mut columns = Vec::with_capacity(create.columns.len());
    for column in &create.columns {
        if column.operator_class.is_some() {
            return Err("CREATE INDEX operator classes are not supported".to_string());
        }
        columns.push(index_column_name(&column.column.expr)?);
    }
    let columns = parse_index_columns(&columns.join(", "))?;
    let name = match create.name {
        Some(name) => {
            let index = standalone_index_name(&name, current_namespace)?;
            if name.0.len() == 2 && index.namespace_id != namespace_id {
                return Err("CREATE INDEX schema does not match the table schema".to_string());
            }
            index.name
        },
        None => format!("{}_{}_idx", table_name.as_str(), columns.join("_")),
    };
    Ok(AlterTableStatement {
        table_name,
        namespace_id,
        operation: ColumnOperation::CreateScalarIndex {
            name,
            columns,
            unique: create.unique,
            if_not_exists: create.if_not_exists,
        },
    })
}

fn index_column_name(expr: &Expr) -> DdlResult<String> {
    match expr {
        Expr::Identifier(ident) if !ident.value.is_empty() => Ok(ident.value.clone()),
        Expr::CompoundIdentifier(parts) => parts
            .last()
            .map(|ident| ident.value.clone())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| "CREATE INDEX requires a column name".to_string()),
        _ => Err("index expressions are not supported; use a column name".to_string()),
    }
}

/// Parse PostgreSQL `ALTER INDEX [IF EXISTS] name RENAME TO new_name`.
pub fn parse_alter_index(
    sql: &str,
    current_namespace: &SchemaId,
) -> DdlResult<AlterTableStatement> {
    let (if_exists, normalized) = without_alter_index_if_exists(sql);
    let statement = KalamParser::parse_one(&normalized).map_err(|err| err.to_string())?;
    let Statement::AlterIndex { name, operation } = statement else {
        return Err("expected ALTER INDEX".to_string());
    };
    let sqlparser::ast::AlterIndexOperation::RenameIndex { index_name } = operation;
    if index_name.0.len() != 1 {
        return Err("ALTER INDEX new name must be an unqualified identifier".to_string());
    }
    let index = standalone_index_name(&name, current_namespace)?;
    let new_name = standalone_index_name(&index_name, current_namespace)?.name;
    if new_name.is_empty() {
        return Err("ALTER INDEX requires a new name".to_string());
    }
    Ok(AlterTableStatement {
        table_name: TableName::new("alter_index"),
        namespace_id: index.namespace_id.clone(),
        operation: ColumnOperation::RenameStandaloneIndex {
            index,
            new_name,
            if_exists,
        },
    })
}

fn without_alter_index_if_exists(sql: &str) -> (bool, std::borrow::Cow<'_, str>) {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)^(\s*ALTER\s+INDEX)\s+IF\s+EXISTS\b").expect("alter index if exists regex")
    });
    if RE.is_match(sql) {
        (true, RE.replace(sql, "$1"))
    } else {
        (false, std::borrow::Cow::Borrowed(sql))
    }
}

fn reject_vector_using(sql: &str) -> DdlResult<()> {
    if sql.to_ascii_uppercase().contains(" USING ") {
        Err("scalar CREATE INDEX cannot mix USING COSINE/L2/DOT; that form is vector-only"
            .to_string())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::models::{SchemaId, TableName};

    use super::*;

    fn ns() -> SchemaId {
        SchemaId::new("public")
    }

    #[test]
    fn parse_alter_create_scalar_index() {
        let stmt = parse_alter_table_scalar_index(
            "ALTER TABLE app.messages CREATE INDEX idx_conv (conversation_id);",
            &ns(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(stmt.namespace_id, SchemaId::new("app"));
        assert_eq!(stmt.table_name, TableName::new("messages"));
        match stmt.operation {
            ColumnOperation::CreateScalarIndex {
                name,
                columns,
                unique,
                if_not_exists,
            } => {
                assert_eq!(name, "idx_conv");
                assert_eq!(columns, vec!["conversation_id".to_string()]);
                assert!(!unique);
                assert!(!if_not_exists);
            },
            other => panic!("expected CreateScalarIndex, got {other:?}"),
        }
    }

    #[test]
    fn parse_create_index_on_unique_if_not_exists() {
        let stmt = parse_create_index_on(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_email ON users (email, status)",
            &ns(),
        )
        .unwrap();
        match stmt.operation {
            ColumnOperation::CreateScalarIndex {
                name,
                columns,
                unique,
                if_not_exists,
            } => {
                assert_eq!(name, "idx_email");
                assert_eq!(columns, vec!["email".to_string(), "status".to_string()]);
                assert!(unique);
                assert!(if_not_exists);
            },
            other => panic!("expected CreateScalarIndex, got {other:?}"),
        }
    }

    #[test]
    fn parentheses_required_for_scalar_alter() {
        assert!(parse_alter_table_scalar_index("ALTER TABLE docs CREATE INDEX embedding", &ns())
            .unwrap()
            .is_none());
    }

    #[test]
    fn mixing_using_metric_errors() {
        let err = parse_alter_table_scalar_index(
            "ALTER TABLE t CREATE INDEX idx (embedding) USING COSINE",
            &ns(),
        )
        .unwrap_err();
        assert!(err.contains("USING"));
    }

    #[test]
    fn parse_drop_index_if_exists_uses_current_schema() {
        let stmt = parse_drop_index("DROP INDEX IF EXISTS idx_users2_email;", &ns()).unwrap();
        assert_eq!(stmt.namespace_id, ns());
        match stmt.operation {
            ColumnOperation::DropStandaloneIndex { indexes, if_exists } => {
                assert!(if_exists);
                assert_eq!(indexes.len(), 1);
                assert_eq!(indexes[0].name, "idx_users2_email");
                assert_eq!(indexes[0].namespace_id, ns());
            },
            other => panic!("expected DropStandaloneIndex, got {other:?}"),
        }
    }

    #[test]
    fn parse_drop_index_schema_list_and_concurrently() {
        let stmt =
            parse_drop_index("DROP INDEX CONCURRENTLY IF EXISTS app.idx_a, idx_b RESTRICT", &ns())
                .unwrap();
        match stmt.operation {
            ColumnOperation::DropStandaloneIndex { indexes, if_exists } => {
                assert!(if_exists);
                assert_eq!(indexes[0].namespace_id, SchemaId::new("app"));
                assert_eq!(indexes[0].name, "idx_a");
                assert_eq!(indexes[1].namespace_id, ns());
                assert_eq!(indexes[1].name, "idx_b");
            },
            other => panic!("expected DropStandaloneIndex, got {other:?}"),
        }
    }

    #[test]
    fn parse_create_index_btree_concurrently_keeps_scalar_columns() {
        let stmt = parse_create_index_on(
            "CREATE UNIQUE INDEX CONCURRENTLY IF NOT EXISTS idx_email ON app.users USING btree \
             (email DESC, status)",
            &ns(),
        )
        .unwrap();
        assert_eq!(stmt.namespace_id, SchemaId::new("app"));
        assert_eq!(stmt.table_name, TableName::new("users"));
        match stmt.operation {
            ColumnOperation::CreateScalarIndex {
                name,
                columns,
                unique,
                if_not_exists,
            } => {
                assert_eq!(name, "idx_email");
                assert_eq!(columns, vec!["email".to_string(), "status".to_string()]);
                assert!(unique);
                assert!(if_not_exists);
            },
            other => panic!("expected CreateScalarIndex, got {other:?}"),
        }
    }

    #[test]
    fn parse_create_index_gin_is_rejected() {
        let err =
            parse_create_index_on("CREATE INDEX idx_message ON posts USING gin (message)", &ns())
                .unwrap_err();
        assert!(err.to_ascii_lowercase().contains("gin"), "{err}");
    }

    #[test]
    fn parse_alter_index_rename_if_exists() {
        let stmt = parse_alter_index(
            "ALTER INDEX IF EXISTS app.idx_email RENAME TO idx_email_lookup",
            &ns(),
        )
        .unwrap();
        match stmt.operation {
            ColumnOperation::RenameStandaloneIndex {
                index,
                new_name,
                if_exists,
            } => {
                assert!(if_exists);
                assert_eq!(index.namespace_id, SchemaId::new("app"));
                assert_eq!(index.name, "idx_email");
                assert_eq!(new_name, "idx_email_lookup");
            },
            other => panic!("expected RenameStandaloneIndex, got {other:?}"),
        }
    }

    #[test]
    fn parse_drop_index_on_table_is_direct_drop() {
        let stmt = parse_drop_index("DROP INDEX IF EXISTS idx_email ON app.users", &ns()).unwrap();
        assert_eq!(stmt.namespace_id, SchemaId::new("app"));
        assert_eq!(stmt.table_name, TableName::new("users"));
        match stmt.operation {
            ColumnOperation::DropIndex { name, if_exists } => {
                assert_eq!(name, "idx_email");
                assert!(if_exists);
            },
            other => panic!("expected DropIndex, got {other:?}"),
        }
    }
}
