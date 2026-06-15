mod table;
mod topic;

use sqlparser::{ast::Statement, dialect::PostgreSqlDialect, parser::Parser};

use crate::{
    diff::SchemaDiffError,
    model::Schema,
    parser::{
        table::{
            extract_kalam_table_kind, parse_create_namespace, remove_kalam_table_kind,
            table_from_create,
        },
        topic::{
            attach_topic_sources, is_alter_topic_add_source, is_create_topic,
            parse_alter_topic_add_source, parse_create_topic, parse_drop_topic,
        },
    },
    sql::{
        extract_with_options, normalize_object_key, split_sql_statements,
        strip_trailing_with_options, trim_leading_sql_comments,
    },
};

pub(crate) fn parse_schema(path: &str, sql: &str) -> Result<Schema, SchemaDiffError> {
    let dialect = PostgreSqlDialect {};
    let mut schema = Schema::default();
    let mut pending_topic_sources = Vec::new();

    for raw_stmt in split_sql_statements(sql) {
        let raw_stmt = raw_stmt.trim();

        if raw_stmt.is_empty() {
            continue;
        }

        let custom_stmt = trim_leading_sql_comments(raw_stmt);

        if custom_stmt.is_empty() {
            continue;
        }

        if let Some(namespace) = parse_create_namespace(custom_stmt) {
            schema.namespaces.insert(normalize_object_key(&namespace));
            continue;
        }

        if is_create_topic(custom_stmt) {
            let topic = parse_create_topic(path, custom_stmt)?;

            if schema.topics.contains_key(&topic.key) {
                return Err(SchemaDiffError::Parse {
                    message: format!("{path}: duplicate topic definition for {}", topic.name_sql),
                });
            }

            schema.topics.insert(topic.key.clone(), topic);
            continue;
        }

        if is_alter_topic_add_source(custom_stmt) {
            pending_topic_sources.push(parse_alter_topic_add_source(path, custom_stmt)?);
            continue;
        }

        if let Some(topic_key) = parse_drop_topic(path, custom_stmt)? {
            schema.topics.remove(&topic_key);
            continue;
        }

        let kind_from_prefix = extract_kalam_table_kind(raw_stmt);
        let with_options = extract_with_options(raw_stmt);
        let parseable_stmt = strip_trailing_with_options(&remove_kalam_table_kind(raw_stmt));

        let parsed = Parser::parse_sql(&dialect, &parseable_stmt).map_err(|source| {
            SchemaDiffError::Parse {
                message: format!(
                    "{path}: failed to parse statement:\n{raw_stmt}\nparser error: {source}"
                ),
            }
        })?;

        for stmt in parsed {
            match stmt {
                Statement::CreateTable(create_table) => {
                    let table =
                        table_from_create(create_table, kind_from_prefix, with_options.clone())
                            .map_err(|message| SchemaDiffError::Parse { message })?;

                    if schema.tables.contains_key(&table.key) {
                        return Err(SchemaDiffError::Parse {
                            message: format!(
                                "{path}: duplicate table definition for {}",
                                table.name_sql
                            ),
                        });
                    }

                    schema.tables.insert(table.key.clone(), table);
                },
                Statement::CreateSchema { schema_name, .. } => {
                    schema.namespaces.insert(normalize_object_key(&schema_name.to_string()));
                },
                _ => {},
            }
        }
    }

    attach_topic_sources(path, &mut schema, pending_topic_sources)?;

    Ok(schema)
}
