//! ALTER TABLE statement parser
//!
//! Parses SQL statements like:
//! - ALTER TABLE messages ADD COLUMN age INT
//! - ALTER TABLE messages DROP COLUMN age
//! - ALTER TABLE messages MODIFY COLUMN age BIGINT

use kalamdb_commons::{
    models::{datatypes::KalamDataType, NamespaceId, StorageId, TableAccess, TableId, TableName},
    schemas::{policy::FlushPolicy, ColumnDefault, TableCompression},
};
use kalamdb_system::VectorMetric;
use once_cell::sync::Lazy;
use regex::{Captures, Regex};
use sqlparser::{
    ast::{
        AlterColumnOperation, AlterTableOperation, ColumnDef, ColumnOption, ColumnOptionDef,
        DropBehavior, Expr, Ident, ObjectName, SqlOption, Statement, Value,
    },
    dialect::GenericDialect,
};

use crate::{
    compatibility::map_sql_type_to_kalam,
    ddl::{parsing::parse_table_reference, DdlResult},
    parser::utils::parse_sql_statements,
};

/// Column alteration operation
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnOperation {
    /// Add a new column
    Add {
        column_name: String,
        data_type: KalamDataType,
        nullable: bool,
        default_value: Option<ColumnDefault>,
        if_not_exists: bool,
    },
    /// Drop an existing column
    Drop { column_name: String },
    /// Modify an existing column's data type
    Modify {
        column_name: String,
        new_data_type: KalamDataType,
        nullable: Option<bool>,
    },
    /// Set or drop nullable state on an existing column.
    SetNullable { column_name: String, nullable: bool },
    /// Set a column default expression.
    SetDefault {
        column_name: String,
        default_value: ColumnDefault,
    },
    /// Drop a column default expression.
    DropDefault { column_name: String },
    /// Rename an existing column (metadata only)
    Rename {
        old_column_name: String,
        new_column_name: String,
    },
    /// Set access level (SHARED tables only)
    SetAccessLevel { access_level: TableAccess },
    /// Set one or more table-level options.
    SetTableOptions { updates: TablePropertyUpdates },
    /// Create or enable a vector index for an embedding column.
    CreateVectorIndex {
        column_name: String,
        metric: VectorMetric,
    },
    /// Disable a vector index for an embedding column.
    DropVectorIndex { column_name: String },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TablePropertyUpdates {
    pub storage_id: Option<StorageId>,
    pub use_user_storage: Option<bool>,
    pub flush_policy: Option<Option<FlushPolicy>>,
    pub ttl_seconds: Option<u64>,
    pub access_level: Option<TableAccess>,
    pub compression: Option<TableCompression>,
    pub eviction_strategy: Option<String>,
    pub max_stream_size_bytes: Option<u64>,
}

impl TablePropertyUpdates {
    fn is_empty(&self) -> bool {
        self.storage_id.is_none()
            && self.use_user_storage.is_none()
            && self.flush_policy.is_none()
            && self.ttl_seconds.is_none()
            && self.access_level.is_none()
            && self.compression.is_none()
            && self.eviction_strategy.is_none()
            && self.max_stream_size_bytes.is_none()
    }

    fn is_access_level_only(&self) -> bool {
        self.access_level.is_some()
            && self.storage_id.is_none()
            && self.use_user_storage.is_none()
            && self.flush_policy.is_none()
            && self.ttl_seconds.is_none()
            && self.compression.is_none()
            && self.eviction_strategy.is_none()
            && self.max_stream_size_bytes.is_none()
    }
}

static SET_ACCESS_LEVEL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)SET\s+ACCESS\s+LEVEL\s+(PUBLIC|PRIVATE|RESTRICTED)").unwrap());
static RE_STORAGE_ID: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap());
static ALTER_CREATE_VECTOR_INDEX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^\s*ALTER\s+TABLE\s+([a-zA-Z_][\w]*(?:\.[a-zA-Z_][\w]*)?)\s+CREATE\s+(?:VECTOR\s+)?INDEX\s+([a-zA-Z_][\w]*)\s*(?:USING\s+(COSINE|L2|DOT))?\s*;?\s*$",
    )
    .unwrap()
});
static ALTER_DROP_VECTOR_INDEX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^\s*ALTER\s+TABLE\s+([a-zA-Z_][\w]*(?:\.[a-zA-Z_][\w]*)?)\s+DROP\s+(?:VECTOR\s+)?INDEX\s+([a-zA-Z_][\w]*)\s*;?\s*$",
    )
    .unwrap()
});

/// ALTER TABLE statement
#[derive(Debug, Clone, PartialEq)]
pub struct AlterTableStatement {
    /// Table name to alter
    pub table_name: TableName,

    /// Namespace ID (defaults to current namespace)
    pub namespace_id: NamespaceId,

    /// Column operation to perform
    pub operation: ColumnOperation,
}

impl AlterTableStatement {
    /// Parse an ALTER TABLE statement from SQL (sqlparser-backed)
    pub fn parse(sql: &str, current_namespace: &NamespaceId) -> DdlResult<Self> {
        if let Some(stmt) = parse_vector_index_operation(sql, current_namespace)? {
            return Ok(stmt);
        }

        let normalized_sql = normalize_alter_sql(sql);
        let dialect = GenericDialect {};
        let mut statements =
            parse_sql_statements(&normalized_sql, &dialect).map_err(|e| e.to_string())?;

        if statements.len() != 1 {
            return Err("Expected exactly one ALTER TABLE statement".to_string());
        }

        let statement = statements.remove(0);
        let Statement::AlterTable(sqlparser::ast::AlterTable {
            name, operations, ..
        }) = statement
        else {
            return Err("Expected ALTER TABLE statement".to_string());
        };

        if operations.len() != 1 {
            return Err("Only one ALTER TABLE operation is supported per statement".to_string());
        }

        let (namespace_id, table_name) = resolve_table_reference(name, current_namespace)?;
        let operation = convert_operation(&operations[0])?;

        Ok(Self {
            table_name,
            namespace_id,
            operation,
        })
    }
}

fn parse_vector_index_operation(
    sql: &str,
    current_namespace: &NamespaceId,
) -> DdlResult<Option<AlterTableStatement>> {
    if let Some(caps) = ALTER_CREATE_VECTOR_INDEX_RE.captures(sql) {
        let table_ref = caps
            .get(1)
            .map(|m| m.as_str())
            .ok_or_else(|| "Missing table reference in CREATE INDEX".to_string())?;
        let column_name = caps
            .get(2)
            .map(|m| m.as_str().to_string())
            .ok_or_else(|| "Missing column name in CREATE INDEX".to_string())?;
        let metric = caps
            .get(3)
            .map(|m| m.as_str().to_uppercase())
            .map(|m| match m.as_str() {
                "COSINE" => Ok(VectorMetric::Cosine),
                "L2" => Ok(VectorMetric::L2),
                "DOT" => Ok(VectorMetric::Dot),
                _ => Err(format!("Unsupported vector index metric '{}'", m)),
            })
            .transpose()?
            .unwrap_or(VectorMetric::Cosine);
        let (namespace_id, table_name) =
            resolve_table_reference_from_str(table_ref, current_namespace)?;
        return Ok(Some(AlterTableStatement {
            table_name,
            namespace_id,
            operation: ColumnOperation::CreateVectorIndex {
                column_name,
                metric,
            },
        }));
    }

    if let Some(caps) = ALTER_DROP_VECTOR_INDEX_RE.captures(sql) {
        let table_ref = caps
            .get(1)
            .map(|m| m.as_str())
            .ok_or_else(|| "Missing table reference in DROP INDEX".to_string())?;
        let column_name = caps
            .get(2)
            .map(|m| m.as_str().to_string())
            .ok_or_else(|| "Missing column name in DROP INDEX".to_string())?;
        let (namespace_id, table_name) =
            resolve_table_reference_from_str(table_ref, current_namespace)?;
        return Ok(Some(AlterTableStatement {
            table_name,
            namespace_id,
            operation: ColumnOperation::DropVectorIndex { column_name },
        }));
    }

    Ok(None)
}

fn resolve_table_reference_from_str(
    table_ref: &str,
    current_namespace: &NamespaceId,
) -> DdlResult<(NamespaceId, TableName)> {
    let (namespace, table_name) = parse_table_reference(table_ref)?;
    let table_id = TableId::try_from_strings(
        namespace.as_deref().unwrap_or(current_namespace.as_str()),
        &table_name,
    )?;
    Ok(table_id.into_parts())
}

fn normalize_alter_sql(sql: &str) -> String {
    let trimmed = sql.trim().trim_end_matches(';');
    SET_ACCESS_LEVEL_RE
        .replace(trimmed, |caps: &Captures| {
            format!("SET TBLPROPERTIES (ACCESS_LEVEL = '{}')", caps[1].to_uppercase())
        })
        .into_owned()
}

fn resolve_table_reference(
    name: ObjectName,
    current_namespace: &NamespaceId,
) -> DdlResult<(NamespaceId, TableName)> {
    let parts = name.0;
    match parts.len() {
        1 => {
            let table_ident = parts[0]
                .as_ident()
                .ok_or_else(|| "Function-based table references are not supported".to_string())?;
            Ok((current_namespace.clone(), TableName::from(table_ident.value.as_str())))
        },
        2 => {
            let namespace_ident = parts[0].as_ident().ok_or_else(|| {
                "Function-based namespace references are not supported".to_string()
            })?;
            let table_ident = parts[1]
                .as_ident()
                .ok_or_else(|| "Function-based table references are not supported".to_string())?;
            Ok((
                NamespaceId::from(namespace_ident.value.as_str()),
                TableName::from(table_ident.value.as_str()),
            ))
        },
        _ => Err("Invalid table reference. Use 'table' or 'namespace.table'".to_string()),
    }
}

fn convert_operation(operation: &AlterTableOperation) -> DdlResult<ColumnOperation> {
    match operation {
        AlterTableOperation::AddColumn {
            column_def,
            column_position,
            if_not_exists,
            ..
        } => {
            if column_position.is_some() {
                return Err("Column position modifiers (FIRST/AFTER) are not supported".to_string());
            }
            build_add_column_operation(column_def, *if_not_exists)
        },
        AlterTableOperation::DropColumn {
            column_names,
            drop_behavior,
            ..
        } => build_drop_column_operation(column_names, drop_behavior),
        AlterTableOperation::ModifyColumn {
            col_name,
            data_type,
            options,
            column_position,
        } => {
            if column_position.is_some() {
                return Err("Column position modifiers (FIRST/AFTER) are not supported".to_string());
            }
            build_modify_column_operation(col_name, data_type, options)
        },
        AlterTableOperation::AlterColumn { column_name, op } => {
            build_alter_column_operation(column_name, op)
        },
        AlterTableOperation::RenameColumn {
            old_column_name,
            new_column_name,
        } => Ok(ColumnOperation::Rename {
            old_column_name: old_column_name.value.clone(),
            new_column_name: new_column_name.value.clone(),
        }),
        AlterTableOperation::SetTblProperties { table_properties } => {
            build_set_table_properties_operation(table_properties)
        },
        _ => Err("Unsupported ALTER TABLE operation".to_string()),
    }
}

fn build_add_column_operation(
    column_def: &ColumnDef,
    if_not_exists: bool,
) -> DdlResult<ColumnOperation> {
    let default_nullable = true;
    let column_name = column_def.name.value.clone();
    let data_type = map_sql_type_to_kalam(&column_def.data_type)?;
    let (nullable, default_value) = extract_column_options(&column_def.options, default_nullable);

    Ok(ColumnOperation::Add {
        column_name,
        data_type,
        nullable,
        default_value,
        if_not_exists,
    })
}

fn build_drop_column_operation(
    column_names: &[Ident],
    drop_behavior: &Option<DropBehavior>,
) -> DdlResult<ColumnOperation> {
    if column_names.len() != 1 {
        return Err("ALTER TABLE only supports dropping one column at a time".to_string());
    }
    if drop_behavior.is_some() {
        return Err("DROP COLUMN CASCADE/RESTRICT is not supported".to_string());
    }
    Ok(ColumnOperation::Drop {
        column_name: column_names[0].value.clone(),
    })
}

fn build_modify_column_operation(
    column_name: &Ident,
    data_type: &sqlparser::ast::DataType,
    options: &[ColumnOption],
) -> DdlResult<ColumnOperation> {
    // Validate the requested type using the shared CREATE TABLE conversion logic.
    let new_data_type = map_sql_type_to_kalam(data_type)?;
    let mut nullable: Option<bool> = None;
    for option in options {
        match option {
            ColumnOption::NotNull => nullable = Some(false),
            ColumnOption::Null => nullable = Some(true),
            _ => {},
        }
    }

    Ok(ColumnOperation::Modify {
        column_name: column_name.value.clone(),
        new_data_type,
        nullable,
    })
}

fn build_alter_column_operation(
    column_name: &Ident,
    operation: &AlterColumnOperation,
) -> DdlResult<ColumnOperation> {
    match operation {
        AlterColumnOperation::SetNotNull => Ok(ColumnOperation::SetNullable {
            column_name: column_name.value.clone(),
            nullable: false,
        }),
        AlterColumnOperation::DropNotNull => Ok(ColumnOperation::SetNullable {
            column_name: column_name.value.clone(),
            nullable: true,
        }),
        AlterColumnOperation::SetDefault { value } => Ok(ColumnOperation::SetDefault {
            column_name: column_name.value.clone(),
            default_value: expr_to_column_default(value),
        }),
        AlterColumnOperation::DropDefault => Ok(ColumnOperation::DropDefault {
            column_name: column_name.value.clone(),
        }),
        AlterColumnOperation::SetDataType { .. } | AlterColumnOperation::AddGenerated { .. } => {
            Err("Unsupported ALTER COLUMN operation".to_string())
        },
    }
}

fn build_set_table_properties_operation(
    table_properties: &[SqlOption],
) -> DdlResult<ColumnOperation> {
    let updates = extract_table_property_updates(table_properties)?;
    if updates.is_empty() {
        return Err("At least one table property is required".to_string());
    }

    if updates.is_access_level_only() {
        if let Some(access_level) = updates.access_level {
            return Ok(ColumnOperation::SetAccessLevel { access_level });
        }
    }

    Ok(ColumnOperation::SetTableOptions { updates })
}

fn extract_table_property_updates(
    table_properties: &[SqlOption],
) -> DdlResult<TablePropertyUpdates> {
    let mut updates = TablePropertyUpdates::default();

    for option in table_properties {
        let SqlOption::KeyValue { key, value } = option else {
            return Err("Only key/value table properties are supported".to_string());
        };

        match key.value.to_uppercase().as_str() {
            "ACCESS_LEVEL" => {
                updates.access_level = Some(parse_access_level_value(value)?);
            },
            "STORAGE_ID" => {
                let storage_id = expr_to_literal(value);
                if !RE_STORAGE_ID.is_match(&storage_id) {
                    return Err(format!(
                        "Invalid STORAGE_ID '{}'. Only alphanumeric, underscore, and hyphen allowed.",
                        storage_id
                    ));
                }
                updates.storage_id = Some(StorageId::from(storage_id));
            },
            "USE_USER_STORAGE" => {
                updates.use_user_storage = Some(parse_bool_property(value)?);
            },
            "FLUSH_POLICY" => {
                updates.flush_policy = Some(parse_optional_flush_policy(value)?);
            },
            "TTL_SECONDS" => {
                updates.ttl_seconds = Some(parse_u64_property(value, "TTL_SECONDS")?);
            },
            "COMPRESSION" => {
                updates.compression = Some(parse_compression_property(value)?);
            },
            "EVICTION_STRATEGY" => {
                updates.eviction_strategy = Some(parse_eviction_strategy_property(value)?);
            },
            "MAX_STREAM_SIZE_BYTES" => {
                updates.max_stream_size_bytes =
                    Some(parse_u64_property(value, "MAX_STREAM_SIZE_BYTES")?);
            },
            other => return Err(format!("Unknown table property '{}'", other)),
        }
    }

    Ok(updates)
}

fn extract_column_options(
    options: &[ColumnOptionDef],
    default_nullable: bool,
) -> (bool, Option<ColumnDefault>) {
    let mut nullable = default_nullable;
    let mut default_value = None;

    for option in options {
        match &option.option {
            ColumnOption::NotNull => nullable = false,
            ColumnOption::Null => nullable = true,
            ColumnOption::Default(expr) => {
                default_value = Some(expr_to_column_default(expr));
            },
            _ => {},
        }
    }

    (nullable, default_value)
}

fn expr_to_literal(expr: &Expr) -> String {
    match expr {
        Expr::Value(value) => value_to_string(&value.value),
        _ => expr.to_string(),
    }
}

fn expr_to_column_default(expr: &Expr) -> ColumnDefault {
    match expr {
        Expr::Function(func) => {
            let name = func.name.to_string().to_uppercase();
            match name.as_str() {
                "NOW" | "CURRENT_TIMESTAMP" | "SNOWFLAKE_ID" | "UUID_V7" | "ULID"
                | "CURRENT_USER" => ColumnDefault::function(&name, vec![]),
                _ => ColumnDefault::literal(serde_json::Value::String(func.to_string())),
            }
        },
        Expr::Value(value) => match &value.value {
            Value::Number(number, _) => {
                if let Ok(int_value) = number.parse::<i64>() {
                    ColumnDefault::literal(serde_json::Value::Number(int_value.into()))
                } else if let Ok(float_value) = number.parse::<f64>() {
                    ColumnDefault::literal(serde_json::json!(float_value))
                } else {
                    ColumnDefault::literal(serde_json::Value::String(number.clone()))
                }
            },
            Value::SingleQuotedString(string_value)
            | Value::DoubleQuotedString(string_value)
            | Value::TripleSingleQuotedString(string_value)
            | Value::TripleDoubleQuotedString(string_value)
            | Value::SingleQuotedByteStringLiteral(string_value)
            | Value::DoubleQuotedByteStringLiteral(string_value)
            | Value::TripleSingleQuotedByteStringLiteral(string_value)
            | Value::TripleDoubleQuotedByteStringLiteral(string_value)
            | Value::SingleQuotedRawStringLiteral(string_value)
            | Value::DoubleQuotedRawStringLiteral(string_value)
            | Value::TripleSingleQuotedRawStringLiteral(string_value)
            | Value::TripleDoubleQuotedRawStringLiteral(string_value)
            | Value::EscapedStringLiteral(string_value)
            | Value::UnicodeStringLiteral(string_value)
            | Value::NationalStringLiteral(string_value)
            | Value::HexStringLiteral(string_value) => {
                ColumnDefault::literal(serde_json::Value::String(string_value.clone()))
            },
            Value::DollarQuotedString(string_value) => {
                ColumnDefault::literal(serde_json::Value::String(string_value.value.clone()))
            },
            Value::QuoteDelimitedStringLiteral(string_value)
            | Value::NationalQuoteDelimitedStringLiteral(string_value) => {
                ColumnDefault::literal(serde_json::Value::String(string_value.value.clone()))
            },
            Value::Boolean(boolean_value) => {
                ColumnDefault::literal(serde_json::Value::Bool(*boolean_value))
            },
            Value::Null => ColumnDefault::literal(serde_json::Value::Null),
            Value::Placeholder(value) => {
                ColumnDefault::literal(serde_json::Value::String(value.clone()))
            },
        },
        Expr::Identifier(identifier) => {
            let normalized = identifier.value.to_uppercase();
            match normalized.as_str() {
                "CURRENT_TIMESTAMP" => ColumnDefault::function("NOW", vec![]),
                "CURRENT_USER" => ColumnDefault::function("CURRENT_USER", vec![]),
                "NULL" => ColumnDefault::literal(serde_json::Value::Null),
                _ => ColumnDefault::literal(serde_json::Value::String(identifier.value.clone())),
            }
        },
        _ => {
            let literal = expr.to_string();
            let normalized = literal.to_uppercase();
            if normalized == "NULL" {
                ColumnDefault::literal(serde_json::Value::Null)
            } else if normalized == "CURRENT_TIMESTAMP" || normalized == "NOW()" {
                ColumnDefault::function("NOW", vec![])
            } else {
                ColumnDefault::literal(serde_json::Value::String(
                    literal.trim_matches('\'').to_string(),
                ))
            }
        },
    }
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::Number(n, _) => n.clone(),
        Value::SingleQuotedString(s)
        | Value::DoubleQuotedString(s)
        | Value::TripleSingleQuotedString(s)
        | Value::TripleDoubleQuotedString(s)
        | Value::SingleQuotedByteStringLiteral(s)
        | Value::DoubleQuotedByteStringLiteral(s)
        | Value::TripleSingleQuotedByteStringLiteral(s)
        | Value::TripleDoubleQuotedByteStringLiteral(s)
        | Value::SingleQuotedRawStringLiteral(s)
        | Value::DoubleQuotedRawStringLiteral(s)
        | Value::TripleSingleQuotedRawStringLiteral(s)
        | Value::TripleDoubleQuotedRawStringLiteral(s)
        | Value::EscapedStringLiteral(s)
        | Value::UnicodeStringLiteral(s)
        | Value::NationalStringLiteral(s)
        | Value::HexStringLiteral(s) => s.clone(),
        Value::DollarQuotedString(s) => s.value.clone(),
        Value::Boolean(b) => b.to_string(),
        Value::Null => "NULL".to_string(),
        Value::Placeholder(p) => p.clone(),
        Value::QuoteDelimitedStringLiteral(s) | Value::NationalQuoteDelimitedStringLiteral(s) => {
            s.value.clone()
        },
    }
}

fn parse_access_level_value(value: &Expr) -> DdlResult<TableAccess> {
    let normalized = expr_to_literal(value).to_uppercase();
    match normalized.as_str() {
        "PUBLIC" => Ok(TableAccess::Public),
        "PRIVATE" => Ok(TableAccess::Private),
        "RESTRICTED" => Ok(TableAccess::Restricted),
        "DBA" => Ok(TableAccess::Dba),
        other => Err(format!(
            "Invalid ACCESS_LEVEL '{}'. Supported values: PUBLIC, PRIVATE, RESTRICTED, DBA",
            other
        )),
    }
}

fn parse_bool_property(value: &Expr) -> DdlResult<bool> {
    match expr_to_literal(value).to_uppercase().as_str() {
        "TRUE" => Ok(true),
        "FALSE" => Ok(false),
        other => Err(format!("Invalid boolean table property '{}'", other)),
    }
}

fn parse_optional_flush_policy(value: &Expr) -> DdlResult<Option<FlushPolicy>> {
    let literal = expr_to_literal(value);
    if literal.eq_ignore_ascii_case("NULL") || literal.eq_ignore_ascii_case("DEFAULT") {
        return Ok(None);
    }
    parse_flush_policy_literal(&literal).map(Some)
}

fn parse_flush_policy_literal(value: &str) -> DdlResult<FlushPolicy> {
    let mut row_limit = None;
    let mut interval_seconds = None;

    for part in value.split(',') {
        let mut pieces = part.splitn(2, ':');
        let key = pieces.next().unwrap_or_default().trim().to_ascii_lowercase();
        let entry = pieces.next().unwrap_or_default().trim();
        if key.is_empty() || entry.is_empty() {
            return Err(format!(
                "Invalid FLUSH_POLICY format '{}'. Expected rows:N, interval:N, or rows:N,interval:N",
                value
            ));
        }

        match key.as_str() {
            "rows" | "row_limit" => {
                row_limit =
                    Some(entry.parse::<u32>().map_err(|_| "Invalid row limit in FLUSH_POLICY")?);
            },
            "interval" | "interval_seconds" => {
                interval_seconds =
                    Some(entry.parse::<u32>().map_err(|_| "Invalid interval in FLUSH_POLICY")?);
            },
            other => return Err(format!("Unknown FLUSH_POLICY key '{}'", other)),
        }
    }

    match (row_limit, interval_seconds) {
        (Some(rows), Some(interval)) => FlushPolicy::combined(rows, interval),
        (Some(rows), None) => FlushPolicy::row_limit(rows),
        (None, Some(interval)) => FlushPolicy::time_interval(interval),
        (None, None) => Err("FLUSH_POLICY must specify 'rows' or 'interval'".to_string()),
    }
}

fn parse_u64_property(value: &Expr, property_name: &str) -> DdlResult<u64> {
    expr_to_literal(value).parse().map_err(|_| format!("Invalid {}", property_name))
}

fn parse_compression_property(value: &Expr) -> DdlResult<TableCompression> {
    expr_to_literal(value).parse()
}

fn parse_eviction_strategy_property(value: &Expr) -> DdlResult<String> {
    let normalized = expr_to_literal(value).trim().to_ascii_lowercase();
    match normalized.as_str() {
        "time_based" | "size_based" | "hybrid" => Ok(normalized),
        _ => Err(format!(
            "Invalid EVICTION_STRATEGY '{}'. Supported: time_based, size_based, hybrid",
            normalized
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_namespace() -> NamespaceId {
        NamespaceId::new("test_app")
    }

    #[test]
    fn test_parse_add_column() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE messages ADD COLUMN age INT",
            &test_namespace(),
        )
        .unwrap();

        assert_eq!(stmt.table_name.as_str(), "messages");

        match stmt.operation {
            ColumnOperation::Add {
                column_name,
                data_type,
                nullable,
                default_value,
                if_not_exists,
            } => {
                assert_eq!(column_name, "age");
                assert_eq!(data_type, KalamDataType::Int);
                assert!(nullable);
                assert_eq!(default_value, None);
                assert!(!if_not_exists);
            },
            _ => panic!("Expected Add operation"),
        }
    }

    #[test]
    fn test_parse_add_column_if_not_exists() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE messages ADD COLUMN IF NOT EXISTS age INT",
            &test_namespace(),
        )
        .unwrap();

        match stmt.operation {
            ColumnOperation::Add { if_not_exists, .. } => {
                assert!(if_not_exists);
            },
            _ => panic!("Expected Add operation"),
        }
    }

    #[test]
    fn test_parse_add_column_not_null() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE messages ADD COLUMN age INT NOT NULL",
            &test_namespace(),
        )
        .unwrap();

        match stmt.operation {
            ColumnOperation::Add { nullable, .. } => {
                assert!(!nullable);
            },
            _ => panic!("Expected Add operation"),
        }
    }

    #[test]
    fn test_parse_add_column_with_default() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE messages ADD COLUMN age INT DEFAULT 0",
            &test_namespace(),
        )
        .unwrap();

        match stmt.operation {
            ColumnOperation::Add {
                column_name,
                default_value,
                ..
            } => {
                assert_eq!(column_name, "age");
                assert_eq!(default_value, Some(ColumnDefault::literal(serde_json::json!(0))));
            },
            _ => panic!("Expected Add operation"),
        }
    }

    #[test]
    fn test_parse_alter_column_set_not_null() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE messages ALTER COLUMN age SET NOT NULL",
            &test_namespace(),
        )
        .unwrap();

        match stmt.operation {
            ColumnOperation::SetNullable {
                column_name,
                nullable,
            } => {
                assert_eq!(column_name, "age");
                assert!(!nullable);
            },
            _ => panic!("Expected SetNullable operation"),
        }
    }

    #[test]
    fn test_parse_alter_column_drop_not_null() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE messages ALTER COLUMN age DROP NOT NULL",
            &test_namespace(),
        )
        .unwrap();

        match stmt.operation {
            ColumnOperation::SetNullable { nullable, .. } => {
                assert!(nullable);
            },
            _ => panic!("Expected SetNullable operation"),
        }
    }

    #[test]
    fn test_parse_alter_column_set_default() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE messages ALTER COLUMN created_at SET DEFAULT NOW()",
            &test_namespace(),
        )
        .unwrap();

        match stmt.operation {
            ColumnOperation::SetDefault {
                column_name,
                default_value,
            } => {
                assert_eq!(column_name, "created_at");
                assert_eq!(default_value, ColumnDefault::function("NOW", vec![]));
            },
            _ => panic!("Expected SetDefault operation"),
        }
    }

    #[test]
    fn test_parse_alter_column_drop_default() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE messages ALTER COLUMN created_at DROP DEFAULT",
            &test_namespace(),
        )
        .unwrap();

        match stmt.operation {
            ColumnOperation::DropDefault { column_name } => {
                assert_eq!(column_name, "created_at");
            },
            _ => panic!("Expected DropDefault operation"),
        }
    }

    #[test]
    fn test_parse_drop_column() {
        let stmt =
            AlterTableStatement::parse("ALTER TABLE messages DROP COLUMN age", &test_namespace())
                .unwrap();

        assert_eq!(stmt.table_name.as_str(), "messages");

        match stmt.operation {
            ColumnOperation::Drop { column_name } => {
                assert_eq!(column_name, "age");
            },
            _ => panic!("Expected Drop operation"),
        }
    }

    #[test]
    fn test_parse_drop_column_shorthand() {
        let stmt =
            AlterTableStatement::parse("ALTER TABLE messages DROP age", &test_namespace()).unwrap();

        match stmt.operation {
            ColumnOperation::Drop { column_name } => {
                assert_eq!(column_name, "age");
            },
            _ => panic!("Expected Drop operation"),
        }
    }

    #[test]
    fn test_parse_modify_column() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE messages MODIFY COLUMN age BIGINT",
            &test_namespace(),
        )
        .unwrap();

        assert_eq!(stmt.table_name.as_str(), "messages");

        match stmt.operation {
            ColumnOperation::Modify {
                column_name,
                new_data_type,
                nullable,
            } => {
                assert_eq!(column_name, "age");
                assert_eq!(new_data_type, KalamDataType::BigInt);
                assert_eq!(nullable, None);
            },
            _ => panic!("Expected Modify operation"),
        }
    }

    #[test]
    fn test_parse_modify_column_with_nullable() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE messages MODIFY COLUMN age BIGINT NOT NULL",
            &test_namespace(),
        )
        .unwrap();

        match stmt.operation {
            ColumnOperation::Modify { nullable, .. } => {
                assert_eq!(nullable, Some(false));
            },
            _ => panic!("Expected Modify operation"),
        }
    }

    #[test]
    fn test_parse_alter_table_add_column() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE messages ADD COLUMN age INT",
            &test_namespace(),
        )
        .unwrap();

        assert_eq!(stmt.table_name.as_str(), "messages");
    }

    #[test]
    fn test_parse_alter_table_drop_column() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE conversations DROP COLUMN old_field",
            &test_namespace(),
        )
        .unwrap();

        assert_eq!(stmt.table_name.as_str(), "conversations");
    }

    #[test]
    fn test_parse_invalid_statement() {
        let result = AlterTableStatement::parse("SELECT * FROM messages", &test_namespace());
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_missing_column_name() {
        let result =
            AlterTableStatement::parse("ALTER TABLE messages ADD COLUMN", &test_namespace());
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_missing_operation() {
        let result = AlterTableStatement::parse("ALTER TABLE messages", &test_namespace());
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_set_access_level_public() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE analytics SET ACCESS LEVEL public",
            &test_namespace(),
        )
        .unwrap();

        assert_eq!(stmt.table_name.as_str(), "analytics");

        match stmt.operation {
            ColumnOperation::SetAccessLevel { access_level } => {
                assert_eq!(access_level, TableAccess::Public);
            },
            _ => panic!("Expected SetAccessLevel operation"),
        }
    }

    #[test]
    fn test_parse_set_access_level_private() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE reports SET ACCESS LEVEL private",
            &test_namespace(),
        )
        .unwrap();

        match stmt.operation {
            ColumnOperation::SetAccessLevel { access_level } => {
                assert_eq!(access_level, TableAccess::Private);
            },
            _ => panic!("Expected SetAccessLevel operation"),
        }
    }

    #[test]
    fn test_parse_set_access_level_restricted() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE sensitive SET ACCESS LEVEL restricted",
            &test_namespace(),
        )
        .unwrap();

        match stmt.operation {
            ColumnOperation::SetAccessLevel { access_level } => {
                assert_eq!(access_level, TableAccess::Restricted);
            },
            _ => panic!("Expected SetAccessLevel operation"),
        }
    }

    #[test]
    fn test_parse_set_access_level_invalid() {
        let result = AlterTableStatement::parse(
            "ALTER TABLE test SET ACCESS LEVEL invalid",
            &test_namespace(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_set_tblproperties_user_options() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE profiles SET TBLPROPERTIES (STORAGE_ID='local-ssd', USE_USER_STORAGE=true, FLUSH_POLICY='rows:500', COMPRESSION='zstd')",
            &test_namespace(),
        )
        .unwrap();

        match stmt.operation {
            ColumnOperation::SetTableOptions { updates } => {
                assert_eq!(updates.storage_id.unwrap().as_str(), "local-ssd");
                assert_eq!(updates.use_user_storage, Some(true));
                assert!(matches!(updates.flush_policy, Some(Some(FlushPolicy::RowLimit { .. }))));
                assert_eq!(updates.compression, Some(TableCompression::Zstd));
            },
            _ => panic!("Expected SetTableOptions operation"),
        }
    }

    #[test]
    fn test_parse_set_tblproperties_stream_options() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE events SET TBLPROPERTIES (TTL_SECONDS=7200, EVICTION_STRATEGY='hybrid', MAX_STREAM_SIZE_BYTES=1048576)",
            &test_namespace(),
        )
        .unwrap();

        match stmt.operation {
            ColumnOperation::SetTableOptions { updates } => {
                assert_eq!(updates.ttl_seconds, Some(7200));
                assert_eq!(updates.eviction_strategy.as_deref(), Some("hybrid"));
                assert_eq!(updates.max_stream_size_bytes, Some(1_048_576));
                assert_eq!(updates.compression, None);
            },
            _ => panic!("Expected SetTableOptions operation"),
        }
    }

    #[test]
    fn test_parse_set_tblproperties_none_compression() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE profiles SET TBLPROPERTIES (COMPRESSION='none')",
            &test_namespace(),
        )
        .unwrap();

        match stmt.operation {
            ColumnOperation::SetTableOptions { updates } => {
                assert_eq!(updates.compression, Some(TableCompression::None));
            },
            _ => panic!("Expected SetTableOptions operation"),
        }
    }

    #[test]
    fn test_parse_set_tblproperties_rejects_unsupported_compression() {
        let sql = "ALTER TABLE events SET TBLPROPERTIES (COMPRESSION='lz4')";
        let err = AlterTableStatement::parse(sql, &test_namespace()).unwrap_err();
        assert!(err.contains("Supported: none, snappy, zstd"));
    }

    #[test]
    fn test_parse_set_access_level_missing_keyword() {
        let result =
            AlterTableStatement::parse("ALTER TABLE test SET LEVEL public", &test_namespace());
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_create_vector_index_default_metric() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE docs CREATE INDEX embedding",
            &test_namespace(),
        )
        .unwrap();

        match stmt.operation {
            ColumnOperation::CreateVectorIndex {
                column_name,
                metric,
            } => {
                assert_eq!(column_name, "embedding");
                assert_eq!(metric, VectorMetric::Cosine);
            },
            _ => panic!("Expected CreateVectorIndex operation"),
        }
    }

    #[test]
    fn test_parse_create_vector_index_with_metric_and_namespace() {
        let stmt = AlterTableStatement::parse(
            "ALTER TABLE app.docs CREATE VECTOR INDEX emb USING L2",
            &test_namespace(),
        )
        .unwrap();
        assert_eq!(stmt.namespace_id, NamespaceId::new("app"));
        assert_eq!(stmt.table_name, TableName::new("docs"));
        match stmt.operation {
            ColumnOperation::CreateVectorIndex {
                column_name,
                metric,
            } => {
                assert_eq!(column_name, "emb");
                assert_eq!(metric, VectorMetric::L2);
            },
            _ => panic!("Expected CreateVectorIndex operation"),
        }
    }

    #[test]
    fn test_parse_add_vector_index_rejected() {
        let result =
            AlterTableStatement::parse("ALTER TABLE docs ADD INDEX embedding", &test_namespace());
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_drop_vector_index() {
        let stmt =
            AlterTableStatement::parse("ALTER TABLE docs DROP INDEX embedding", &test_namespace())
                .unwrap();
        match stmt.operation {
            ColumnOperation::DropVectorIndex { column_name } => {
                assert_eq!(column_name, "embedding");
            },
            _ => panic!("Expected DropVectorIndex operation"),
        }
    }
}
