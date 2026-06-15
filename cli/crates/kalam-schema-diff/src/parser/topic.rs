use std::collections::BTreeMap;

use crate::{
    diff::SchemaDiffError,
    model::{
        topic_source_key, PendingTopicSource, Schema, Table, Topic, TopicOperation,
        TopicPayloadMode, TopicRetention, TopicSource,
    },
    sql::{
        clean_identifier_token, eq_ci, extract_with_options, find_trailing_with_span,
        normalize_object_key, normalize_sql_fragment, skip_ws, strip_trailing_with_options,
        unquote, word_spans,
    },
};

pub(super) fn is_create_topic(sql: &str) -> bool {
    let words = word_spans(sql);

    words.len() >= 2 && eq_ci(words[0].text, "CREATE") && eq_ci(words[1].text, "TOPIC")
}

pub(super) fn is_alter_topic_add_source(sql: &str) -> bool {
    let words = word_spans(sql);

    words.len() >= 5
        && eq_ci(words[0].text, "ALTER")
        && eq_ci(words[1].text, "TOPIC")
        && eq_ci(words[3].text, "ADD")
        && eq_ci(words[4].text, "SOURCE")
}

pub(super) fn parse_create_topic(path: &str, sql: &str) -> Result<Topic, SchemaDiffError> {
    let definition_sql = strip_trailing_with_options(sql);
    let words = word_spans(&definition_sql);

    if words.len() < 3 || !is_create_topic(&definition_sql) {
        return Err(topic_parse_error(path, "topic name required after CREATE TOPIC"));
    }

    let mut if_not_exists = false;
    let mut topic_index = 2;

    if words.get(2).is_some_and(|word| eq_ci(word.text, "IF")) {
        if words.len() < 6 || !eq_ci(words[3].text, "NOT") || !eq_ci(words[4].text, "EXISTS") {
            return Err(topic_parse_error(path, "expected IF NOT EXISTS after CREATE TOPIC"));
        }

        if_not_exists = true;
        topic_index = 5;
    }

    let topic_word = words
        .get(topic_index)
        .ok_or_else(|| topic_parse_error(path, "topic name required after CREATE TOPIC"))?;
    let name_sql = clean_identifier_token(topic_word.text);
    let key = normalize_object_key(&name_sql);
    let mut index = topic_index + 1;
    let mut partitions = None;

    if let Some(word) = words.get(index) {
        if !eq_ci(word.text, "PARTITIONS") {
            return Err(topic_parse_error(
                path,
                &format!("unexpected token {} in CREATE TOPIC", word.text),
            ));
        }

        let count_word = words
            .get(index + 1)
            .ok_or_else(|| topic_parse_error(path, "partition count required after PARTITIONS"))?;
        let count = count_word
            .text
            .parse::<u32>()
            .map_err(|_| topic_parse_error(path, "partition count must be a positive integer"))?;

        partitions = Some(count);
        index += 2;
    }

    if let Some(word) = words.get(index) {
        return Err(topic_parse_error(
            path,
            &format!("unexpected token {} in CREATE TOPIC", word.text),
        ));
    }

    Ok(Topic {
        key,
        name_sql,
        if_not_exists,
        partitions,
        retention: parse_topic_retention(path, sql)?,
        sources: BTreeMap::new(),
    })
}

pub(super) fn parse_alter_topic_add_source(
    path: &str,
    sql: &str,
) -> Result<PendingTopicSource, SchemaDiffError> {
    let words = word_spans(sql);

    if words.len() < 8 || !is_alter_topic_add_source(sql) {
        return Err(topic_parse_error(
            path,
            "expected ALTER TOPIC <topic> ADD SOURCE <table> ON <INSERT|UPDATE|DELETE>",
        ));
    }

    let topic_name_sql = clean_identifier_token(words[2].text);
    let topic_key = normalize_object_key(&topic_name_sql);
    let table_sql = clean_identifier_token(words[5].text);
    let table_key = normalize_object_key(&table_sql);

    if !eq_ci(words[6].text, "ON") {
        return Err(topic_parse_error(path, "expected ON after ALTER TOPIC ADD SOURCE table"));
    }

    let operation = TopicOperation::parse(words[7].text)
        .map_err(|message| topic_parse_error(path, &message))?;

    if let Some(word) = words.get(8) {
        if !eq_ci(word.text, "WHERE") && !eq_ci(word.text, "WITH") {
            return Err(topic_parse_error(
                path,
                &format!("unexpected token {} in ALTER TOPIC ADD SOURCE", word.text),
            ));
        }
    }

    let with_span = find_trailing_with_span(sql);
    let filter_expr = words
        .iter()
        .skip(8)
        .find(|word| eq_ci(word.text, "WHERE"))
        .map(|word| {
            let filter_start = skip_ws(sql, word.end);
            let filter_end = with_span.map(|(start, _end)| start).unwrap_or(sql.len());
            let filter = sql[filter_start..filter_end].trim().trim_end_matches(';').trim();

            if filter.is_empty() {
                Err(topic_parse_error(path, "WHERE clause must include a filter expression"))
            } else {
                Ok(normalize_sql_fragment(filter))
            }
        })
        .transpose()?;
    let (payload_mode, payload_explicit) = parse_topic_payload(path, sql)?;
    let key = topic_source_key(&table_key, operation, filter_expr.as_deref(), payload_mode);

    Ok(PendingTopicSource {
        topic_key,
        topic_name_sql,
        source: TopicSource {
            key,
            table_sql,
            table_key,
            operation,
            filter_expr,
            payload_mode,
            payload_explicit,
        },
    })
}

pub(super) fn parse_drop_topic(path: &str, sql: &str) -> Result<Option<String>, SchemaDiffError> {
    let words = word_spans(sql);

    if words.len() < 2 || !eq_ci(words[0].text, "DROP") || !eq_ci(words[1].text, "TOPIC") {
        return Ok(None);
    }

    let topic_word = words
        .get(2)
        .ok_or_else(|| topic_parse_error(path, "topic name required after DROP TOPIC"))?;

    Ok(Some(normalize_object_key(topic_word.text)))
}

fn parse_topic_retention(path: &str, sql: &str) -> Result<TopicRetention, SchemaDiffError> {
    let Some((_start, _end)) = find_trailing_with_span(sql) else {
        return Ok(TopicRetention::default());
    };

    let options = extract_with_options(sql);
    let mut retention = TopicRetention::default();

    for (key, value) in options {
        let parsed_value = parse_topic_retention_value(path, &value)?;

        match key.as_str() {
            "RETENTION_SECONDS" => retention.retention_seconds = Some(parsed_value),
            "RETENTION_MAX_BYTES" => retention.retention_max_bytes = Some(parsed_value),
            _ => {
                return Err(topic_parse_error(
                    path,
                    &format!("unknown CREATE TOPIC WITH option {}", key),
                ));
            },
        }
    }

    if retention.is_empty() {
        return Err(topic_parse_error(
            path,
            "CREATE TOPIC WITH must include retention_seconds or retention_max_bytes",
        ));
    }

    Ok(retention)
}

fn parse_topic_payload(path: &str, sql: &str) -> Result<(TopicPayloadMode, bool), SchemaDiffError> {
    let Some((_start, _end)) = find_trailing_with_span(sql) else {
        return Ok((TopicPayloadMode::Full, false));
    };

    let options = extract_with_options(sql);
    let mut payload_mode = None;

    for (key, value) in options {
        match key.as_str() {
            "PAYLOAD" => {
                payload_mode = Some(
                    TopicPayloadMode::parse(&value)
                        .map_err(|message| topic_parse_error(path, &message))?,
                );
            },
            _ => {
                return Err(topic_parse_error(
                    path,
                    &format!("unknown ALTER TOPIC ADD SOURCE WITH option {}", key),
                ));
            },
        }
    }

    payload_mode
        .map(|mode| (mode, true))
        .ok_or_else(|| topic_parse_error(path, "ALTER TOPIC ADD SOURCE WITH must include payload"))
}

fn parse_topic_retention_value(path: &str, value: &str) -> Result<String, SchemaDiffError> {
    let trimmed = unquote(value.trim().trim_end_matches(';'));

    if trimmed.eq_ignore_ascii_case("NULL") {
        return Ok("NULL".to_string());
    }

    let parsed = trimmed
        .parse::<i64>()
        .map_err(|_| topic_parse_error(path, "topic retention values must be integers or NULL"))?;

    if parsed < 0 {
        return Err(topic_parse_error(
            path,
            "topic retention values must be non-negative integers or NULL",
        ));
    }

    Ok(parsed.to_string())
}

pub(super) fn attach_topic_sources(
    path: &str,
    schema: &mut Schema,
    pending_topic_sources: Vec<PendingTopicSource>,
) -> Result<(), SchemaDiffError> {
    for mut pending in pending_topic_sources {
        if !schema.topics.contains_key(&pending.topic_key) {
            return Err(topic_parse_error(
                path,
                &format!(
                    "topic source references {}, but CREATE TOPIC {} is not defined in schema.sql",
                    pending.topic_name_sql, pending.topic_name_sql
                ),
            ));
        }

        resolve_topic_source_table(path, &schema.tables, &mut pending.source)?;

        let topic = schema.topics.get_mut(&pending.topic_key).expect("topic existence checked");

        if topic.sources.contains_key(&pending.source.key) {
            return Err(topic_parse_error(
                path,
                &format!(
                    "duplicate source table {} for topic {}",
                    pending.source.table_sql, pending.topic_name_sql
                ),
            ));
        }

        topic.sources.insert(pending.source.key.clone(), pending.source);
    }

    Ok(())
}

fn resolve_topic_source_table(
    path: &str,
    tables: &BTreeMap<String, Table>,
    source: &mut TopicSource,
) -> Result<(), SchemaDiffError> {
    if tables.contains_key(&source.table_key) {
        return Ok(());
    }

    if !source.table_key.contains('.') {
        let default_key = format!("default.{}", source.table_key);

        if tables.contains_key(&default_key) {
            source.table_key = default_key;
            source.refresh_key();
            return Ok(());
        }

        let qualified_matches = tables
            .keys()
            .filter(|key| key.rsplit('.').next() == Some(source.table_key.as_str()))
            .cloned()
            .collect::<Vec<_>>();

        if !qualified_matches.is_empty() {
            return Err(topic_parse_error(
                path,
                &format!(
                    "topic source table {} is not defined in schema.sql; found {}. Use the qualified table name in ALTER TOPIC ADD SOURCE.",
                    source.table_sql,
                    qualified_matches.join(", ")
                ),
            ));
        }
    }

    Err(topic_parse_error(
        path,
        &format!("topic source table {} must be defined in schema.sql", source.table_sql),
    ))
}

fn topic_parse_error(path: &str, message: &str) -> SchemaDiffError {
    SchemaDiffError::Parse {
        message: format!("{path}: {message}"),
    }
}
