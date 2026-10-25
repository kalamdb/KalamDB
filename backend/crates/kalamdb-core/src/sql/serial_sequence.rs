//! Process-local `serial_id` counters and the names `SETVAL` uses to reach them.
//!
//! PostgreSQL resets a serial after an explicit primary key with
//! `pg_get_serial_sequence` plus `SETVAL`. KalamDB has no sequence catalog. A `serial_id`
//! column is registered here when its table enters the schema cache, and `SETVAL` moves that
//! table's counter so the next generated id is `max + 1`.

use std::sync::{
    atomic::{AtomicI32, Ordering},
    OnceLock,
};

use dashmap::DashMap;
use kalamdb_commons::{models::schemas::TableDefinition, schemas::serial_sequence_name, TableId};

use crate::error::KalamDbError;

struct SerialColumn {
    table_id: TableId,
    sequence_name: String,
}

fn counters() -> &'static DashMap<TableId, AtomicI32> {
    static COUNTERS: OnceLock<DashMap<TableId, AtomicI32>> = OnceLock::new();
    COUNTERS.get_or_init(DashMap::new)
}

fn by_column() -> &'static DashMap<String, SerialColumn> {
    static BY_COLUMN: OnceLock<DashMap<String, SerialColumn>> = OnceLock::new();
    BY_COLUMN.get_or_init(DashMap::new)
}

fn by_sequence() -> &'static DashMap<String, TableId> {
    static BY_SEQUENCE: OnceLock<DashMap<String, TableId>> = OnceLock::new();
    BY_SEQUENCE.get_or_init(DashMap::new)
}

fn column_key(namespace: &str, table: &str, column: &str) -> String {
    format!(
        "{}\0{}\0{}",
        namespace.to_ascii_lowercase(),
        table.to_ascii_lowercase(),
        column.to_ascii_lowercase()
    )
}

/// Next `serial_id` for `table_id`. The stored atomic is the value this call returns.
pub(crate) fn next_serial_id(table_id: &TableId) -> Result<i32, KalamDbError> {
    let counter = counters().entry(table_id.clone()).or_insert_with(|| AtomicI32::new(1));
    let id = counter.fetch_add(1, Ordering::Relaxed);
    if id <= 0 {
        return Err(KalamDbError::InvalidOperation(
            "serial_id() exceeded the int4 range".to_string(),
        ));
    }
    Ok(id)
}

/// Record every `serial_id` column on `table`, replacing any previous registration.
pub(crate) fn remember_serial_columns(table: &TableDefinition) {
    let table_id = table.table_id();
    forget_serial_columns(&table_id);
    let namespace = table.namespace_id.as_str();
    let table_name = table.table_name.as_str();
    for column in &table.columns {
        if !column.default_value.is_serial_sequence() {
            continue;
        }
        let sequence_name = serial_sequence_name(namespace, table_name, &column.column_name);
        by_column().insert(
            column_key(namespace, table_name, &column.column_name),
            SerialColumn {
                table_id: table_id.clone(),
                sequence_name: sequence_name.clone(),
            },
        );
        by_sequence().insert(sequence_name, table_id.clone());
    }
}

/// Drop sequence names for a table that left the schema cache.
pub(crate) fn forget_serial_columns(table_id: &TableId) {
    let stale: Vec<String> = by_column()
        .iter()
        .filter(|entry| &entry.value().table_id == table_id)
        .map(|entry| entry.key().clone())
        .collect();
    for key in stale {
        if let Some((_, column)) = by_column().remove(&key) {
            by_sequence().remove(&column.sequence_name);
        }
    }
}

/// Sequence name for a column, or `None` when that column is not `serial_id`.
pub(crate) fn serial_sequence_for(
    namespace: Option<&str>,
    table: &str,
    column: &str,
) -> Option<String> {
    if let Some(namespace) = namespace.filter(|namespace| !namespace.is_empty()) {
        if let Some(found) = by_column().get(&column_key(namespace, table, column)) {
            return Some(found.sequence_name.clone());
        }
        return None;
    }
    let suffix = format!("\0{}\0{}", table.to_ascii_lowercase(), column.to_ascii_lowercase());
    let mut found = None;
    for entry in by_column().iter() {
        if entry.key().ends_with(&suffix) {
            if found.is_some() {
                return None;
            }
            found = Some(entry.value().sequence_name.clone());
        }
    }
    found
}

/// Resolve `pg_get_serial_sequence(table_text, column_text)`.
///
/// `table_text` may be a quoted identifier (`"articles"`) or `schema.table`. An unqualified
/// name uses `default_schema`.
pub(crate) fn serial_sequence_for_regclass(
    default_schema: Option<&str>,
    table_text: &str,
    column_text: &str,
) -> Option<String> {
    let (schema, table) = parse_regclass_name(table_text)?;
    let column = unquote_ident(column_text.trim());
    if table.is_empty() || column.is_empty() {
        return None;
    }
    let schema = schema.or_else(|| {
        default_schema
            .map(str::trim)
            .filter(|schema| !schema.is_empty())
            .map(str::to_string)
    });
    serial_sequence_for(schema.as_deref(), &table, &column)
}

/// Set the counter so the next `serial_id` follows PostgreSQL `setval(seq, value, is_called)`.
///
/// `is_called` means the next generated id is `value + 1`. Directus uses that two-argument form.
pub(crate) fn set_serial_sequence(
    sequence_name: &str,
    last_value: i64,
    is_called: bool,
) -> Result<(), KalamDbError> {
    let Some(table_id) = by_sequence().get(sequence_name).map(|entry| entry.clone()) else {
        return Err(KalamDbError::InvalidOperation(format!(
            "serial sequence {sequence_name} does not exist"
        )));
    };
    let next = if is_called {
        last_value.checked_add(1)
    } else {
        Some(last_value)
    };
    let Some(next) = next else {
        return Err(KalamDbError::InvalidOperation(
            "serial value is outside the int4 range".to_string(),
        ));
    };
    let Ok(next) = i32::try_from(next) else {
        return Err(KalamDbError::InvalidOperation(
            "serial value is outside the int4 range".to_string(),
        ));
    };
    if next <= 0 {
        return Err(KalamDbError::InvalidOperation("serial value must be positive".to_string()));
    }
    counters().insert(table_id, AtomicI32::new(next));
    Ok(())
}

fn parse_regclass_name(input: &str) -> Option<(Option<String>, String)> {
    let parts = split_qualified_name(input.trim());
    match parts.as_slice() {
        [table] if !table.is_empty() => Some((None, table.clone())),
        [schema, table] if !schema.is_empty() && !table.is_empty() => {
            Some((Some(schema.clone()), table.clone()))
        },
        _ => None,
    }
}

fn split_qualified_name(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut part_was_quoted = false;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            if in_quotes && chars.peek() == Some(&'"') {
                current.push('"');
                chars.next();
            } else {
                in_quotes = !in_quotes;
                part_was_quoted = true;
            }
            continue;
        }
        if ch == '.' && !in_quotes {
            parts.push(finish_ident(std::mem::take(&mut current), part_was_quoted));
            part_was_quoted = false;
            continue;
        }
        current.push(ch);
    }
    if !current.is_empty() || !parts.is_empty() {
        parts.push(finish_ident(current, part_was_quoted));
    }
    parts
}

fn finish_ident(value: String, was_quoted: bool) -> String {
    if was_quoted {
        value
    } else {
        value.to_ascii_lowercase()
    }
}

fn unquote_ident(input: &str) -> String {
    let parts = split_qualified_name(input);
    parts.last().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::{
        models::{
            datatypes::KalamDataType,
            schemas::{ColumnDefinition, TableOptions, TableType},
        },
        schemas::ColumnDefault,
        SchemaId, TableName,
    };

    use super::*;

    fn articles() -> TableDefinition {
        let id = ColumnDefinition::new(
            1,
            "id",
            1,
            KalamDataType::Int,
            false,
            true,
            false,
            ColumnDefault::function("serial_id", vec![]),
            None,
        );
        let title = ColumnDefinition::new(
            2,
            "title",
            2,
            KalamDataType::Text,
            true,
            false,
            false,
            ColumnDefault::none(),
            None,
        );
        TableDefinition::new(
            SchemaId::new("directus"),
            TableName::new("articles"),
            TableType::User,
            vec![id, title],
            TableOptions::user(),
            None,
        )
        .expect("articles")
    }

    #[test]
    fn setval_advances_the_next_serial_past_an_explicit_key() {
        let table = articles();
        remember_serial_columns(&table);
        let sequence = serial_sequence_for_regclass(Some("directus"), "\"articles\"", "id")
            .expect("quoted table name");
        assert_eq!(sequence, "directus.articles_id_seq");
        set_serial_sequence(&sequence, 101_111, true).expect("setval");
        assert_eq!(next_serial_id(&table.table_id()).expect("next"), 101_112);
        forget_serial_columns(&table.table_id());
        assert!(serial_sequence_for(Some("directus"), "articles", "id").is_none());
    }

    #[test]
    fn snowflake_default_is_not_a_serial_sequence() {
        let id = ColumnDefinition::new(
            1,
            "id",
            1,
            KalamDataType::BigInt,
            false,
            true,
            false,
            ColumnDefault::function("snowflake_id", vec![]),
            None,
        );
        let table = TableDefinition::new(
            SchemaId::new("directus"),
            TableName::new("big_articles"),
            TableType::User,
            vec![id],
            TableOptions::user(),
            None,
        )
        .expect("big_articles");
        remember_serial_columns(&table);
        assert!(serial_sequence_for(Some("directus"), "big_articles", "id").is_none());
    }
}
