//! DROP DOMAIN parser for PostgreSQL 18 domain types.

use kalamdb_commons::models::SchemaId;

use crate::ddl::{
    create_type::{split_qualified_ident, split_top_level},
    parsing::take_keyword_ci,
    DdlResult,
};

/// One domain target in `DROP DOMAIN`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedDomainName {
    pub namespace_id: SchemaId,
    pub name:         String,
}

/// Parsed `DROP DOMAIN [IF EXISTS] name [, ...] [CASCADE|RESTRICT]` statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropDomainStatement {
    pub namespace_id: SchemaId,
    pub name:         String,
    pub targets:      Vec<QualifiedDomainName>,
    pub if_exists:    bool,
    pub cascade:      bool,
}

impl DropDomainStatement {
    pub fn parse(sql: &str, default_namespace: &SchemaId) -> DdlResult<Self> {
        let mut rest = sql.trim().trim_end_matches(';');
        if !take_keyword_ci(&mut rest, "DROP DOMAIN") {
            return Err("Expected DROP DOMAIN statement".to_string());
        }
        let if_exists = take_keyword_ci(&mut rest, "IF EXISTS");
        let rest = rest.trim_start();
        if rest.is_empty() {
            return Err("Expected domain name in DROP DOMAIN".to_string());
        }

        let parts = split_top_level(rest, ',');
        let mut targets = Vec::with_capacity(parts.len());
        let mut cascade = false;

        for (index, part) in parts.iter().enumerate() {
            let (qual, after) = split_qualified_ident(part.trim())?;
            let mut leftover = after.trim_start();
            if index + 1 == parts.len() {
                if take_keyword_ci(&mut leftover, "CASCADE") {
                    cascade = true;
                } else {
                    let _ = take_keyword_ci(&mut leftover, "RESTRICT");
                }
            }
            if !leftover.trim().is_empty() {
                return Err(format!(
                    "Expected CASCADE, RESTRICT, or end of statement after DROP DOMAIN, got '{}'",
                    leftover.trim()
                ));
            }
            targets.push(QualifiedDomainName {
                namespace_id: qual.namespace_or(default_namespace),
                name:         qual.name,
            });
        }

        let first = targets
            .first()
            .cloned()
            .ok_or_else(|| "Expected domain name in DROP DOMAIN".to_string())?;

        Ok(Self {
            namespace_id: first.namespace_id,
            name: first.name,
            targets,
            if_exists,
            cascade,
        })
    }
}
