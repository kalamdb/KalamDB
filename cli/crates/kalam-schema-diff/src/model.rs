use std::collections::{BTreeMap, BTreeSet};

use crate::sql::unquote;

#[derive(Debug, Clone, Default)]
pub(crate) struct Schema {
    pub(crate) namespaces: BTreeSet<String>,
    pub(crate) tables: BTreeMap<String, Table>,
    pub(crate) topics: BTreeMap<String, Topic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum TableKind {
    User,
    Shared,
    Stream,
}

impl TableKind {
    pub(crate) fn from_str(value: &str) -> Option<Self> {
        match unquote(value).to_ascii_uppercase().as_str() {
            "USER" => Some(Self::User),
            "SHARED" => Some(Self::Shared),
            "STREAM" => Some(Self::Stream),
            _ => None,
        }
    }

    pub(crate) fn as_create_prefix(self) -> &'static str {
        match self {
            Self::User => "USER ",
            Self::Shared => "SHARED ",
            Self::Stream => "STREAM ",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Table {
    pub(crate) key: String,
    pub(crate) name_sql: String,
    pub(crate) kind: Option<TableKind>,
    pub(crate) column_order: Vec<String>,
    pub(crate) columns: BTreeMap<String, Column>,
    pub(crate) constraints: Vec<String>,
    pub(crate) options: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Column {
    pub(crate) key: String,
    pub(crate) name_sql: String,
    pub(crate) type_sql: String,
    pub(crate) type_key: String,
    pub(crate) not_null: bool,
    pub(crate) default_sql: Option<String>,
    pub(crate) primary_key: bool,
    pub(crate) extra_options: Vec<String>,
    pub(crate) create_sql: String,
}

impl Column {
    pub(crate) fn semantic_signature(&self) -> String {
        format!(
            "type={};not_null={};default={};pk={};extra={}",
            self.type_key,
            self.not_null,
            self.default_sql.clone().unwrap_or_default(),
            self.primary_key,
            self.extra_options.join("|"),
        )
    }

    pub(crate) fn modify_fragment(&self) -> String {
        let mut out = format!("{} {}", self.name_sql, self.type_sql);

        if self.not_null {
            out.push_str(" NOT NULL");
        } else {
            out.push_str(" NULL");
        }

        if let Some(default_sql) = &self.default_sql {
            out.push_str(" DEFAULT ");
            out.push_str(default_sql);
        }

        out
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Topic {
    pub(crate) key: String,
    pub(crate) name_sql: String,
    pub(crate) if_not_exists: bool,
    pub(crate) partitions: Option<u32>,
    pub(crate) retention: TopicRetention,
    pub(crate) sources: BTreeMap<String, TopicSource>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TopicRetention {
    pub(crate) retention_seconds: Option<String>,
    pub(crate) retention_max_bytes: Option<String>,
}

impl TopicRetention {
    pub(crate) fn is_empty(&self) -> bool {
        self.retention_seconds.is_none() && self.retention_max_bytes.is_none()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TopicSource {
    pub(crate) key: String,
    pub(crate) table_sql: String,
    pub(crate) table_key: String,
    pub(crate) operation: TopicOperation,
    pub(crate) filter_expr: Option<String>,
    pub(crate) payload_mode: TopicPayloadMode,
    pub(crate) payload_explicit: bool,
}

impl TopicSource {
    pub(crate) fn refresh_key(&mut self) {
        self.key = topic_source_key(
            &self.table_key,
            self.operation,
            self.filter_expr.as_deref(),
            self.payload_mode,
        );
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PendingTopicSource {
    pub(crate) topic_key: String,
    pub(crate) topic_name_sql: String,
    pub(crate) source: TopicSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum TopicOperation {
    Insert,
    Update,
    Delete,
}

impl TopicOperation {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_uppercase().as_str() {
            "INSERT" => Ok(Self::Insert),
            "UPDATE" => Ok(Self::Update),
            "DELETE" => Ok(Self::Delete),
            _ => Err(format!(
                "invalid topic source operation {value}. Expected INSERT, UPDATE, or DELETE"
            )),
        }
    }

    pub(crate) fn as_sql(self) -> &'static str {
        match self {
            Self::Insert => "INSERT",
            Self::Update => "UPDATE",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum TopicPayloadMode {
    Key,
    Full,
    Diff,
}

impl TopicPayloadMode {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match unquote(value).to_ascii_lowercase().as_str() {
            "key" => Ok(Self::Key),
            "full" => Ok(Self::Full),
            "diff" => Ok(Self::Diff),
            _ => Err(format!("invalid topic payload mode {value}. Expected key, full, or diff")),
        }
    }

    pub(crate) fn as_sql(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Full => "full",
            Self::Diff => "diff",
        }
    }
}

pub(crate) fn topic_source_key(
    table_key: &str,
    operation: TopicOperation,
    filter_expr: Option<&str>,
    payload_mode: TopicPayloadMode,
) -> String {
    format!(
        "table={};op={};filter={};payload={}",
        table_key,
        operation.as_sql(),
        filter_expr.unwrap_or_default(),
        payload_mode.as_sql(),
    )
}
