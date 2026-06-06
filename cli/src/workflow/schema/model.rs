//! Language-neutral schema model for generation and migration diffing.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaSnapshot {
    pub origin: SchemaOrigin,
    pub tables: BTreeMap<String, TableDefinition>,
    pub captured_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SchemaOrigin {
    File,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableDefinition {
    pub name: String,
    pub columns: Vec<ColumnDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnDefinition {
    pub name: String,
    pub sql_type: String,
    pub nullable: bool,
    pub primary_key: bool,
}

impl SchemaSnapshot {
    pub fn empty(origin: SchemaOrigin) -> Self {
        Self {
            origin,
            tables: BTreeMap::new(),
            captured_at: Utc::now(),
        }
    }

    pub fn table_names(&self) -> Vec<&str> {
        self.tables.keys().map(String::as_str).collect()
    }
}

/// Supported generated language targets in the first release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LanguageTarget {
    TypeScript,
    Dart,
}

impl LanguageTarget {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "typescript" | "ts" => Some(Self::TypeScript),
            "dart" => Some(Self::Dart),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::Dart => "dart",
        }
    }
}

pub fn parse_language_list(values: &[String]) -> Vec<LanguageTarget> {
    values.iter().filter_map(|value| LanguageTarget::parse(value)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_language_aliases() {
        assert_eq!(LanguageTarget::parse("ts"), Some(LanguageTarget::TypeScript));
        assert_eq!(LanguageTarget::parse("dart"), Some(LanguageTarget::Dart));
    }
}
