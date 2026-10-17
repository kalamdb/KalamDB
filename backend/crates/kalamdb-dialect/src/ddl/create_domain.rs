//! CREATE DOMAIN parser for PostgreSQL 18 domain types.

use std::collections::HashSet;

use kalamdb_commons::models::SchemaId;

use crate::ddl::{
    create_type::{
        matching_paren, parse_type_reference, split_qualified_ident, take_ident, TypeReference,
    },
    parsing::take_keyword_ci,
    DdlResult,
};

/// One `CHECK (...)` constraint attached to a domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainCheckClause {
    pub name:       Option<String>,
    pub expression: String,
    pub validated:  bool,
}

/// Parsed `CREATE DOMAIN` statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateDomainStatement {
    pub namespace_id: SchemaId,
    pub name:         String,
    pub base_type:    TypeReference,
    pub collation:    Option<String>,
    pub default_expr: Option<String>,
    pub not_null:     bool,
    pub checks:       Vec<DomainCheckClause>,
}

impl CreateDomainStatement {
    pub fn parse(sql: &str, default_namespace: &SchemaId) -> DdlResult<Self> {
        let mut rest = sql.trim().trim_end_matches(';');
        if take_keyword_ci(&mut rest, "CREATE OR REPLACE DOMAIN") {
            return Err("CREATE OR REPLACE DOMAIN is not supported in PostgreSQL 18; use ALTER \
                        DOMAIN or DROP DOMAIN instead"
                .to_string());
        }
        if !take_keyword_ci(&mut rest, "CREATE DOMAIN") {
            return Err("Expected CREATE DOMAIN statement".to_string());
        }
        if take_keyword_ci(&mut rest, "IF NOT EXISTS") {
            return Err("CREATE DOMAIN IF NOT EXISTS is not supported in PostgreSQL 18".to_string());
        }

        let (qual, after_name) = split_qualified_ident(rest)?;
        let namespace_id = qual.namespace_or(default_namespace);
        let name = qual.name;
        if name.is_empty() {
            return Err("Domain name cannot be empty".to_string());
        }

        let mut rest = after_name.trim_start();
        let _ = take_keyword_ci(&mut rest, "AS");
        rest = rest.trim_start();
        if rest.is_empty() {
            return Err("CREATE DOMAIN requires an underlying data type".to_string());
        }

        let (base_type, after_type) = parse_type_reference(rest)?;
        let mut rest = after_type.trim_start();

        let mut collation = None;
        let mut default_expr = None;
        let mut seen_not_null = false;
        let mut seen_null = false;
        let mut checks = Vec::new();
        let mut constraint_names = HashSet::new();

        while !rest.is_empty() {
            if take_keyword_ci(&mut rest, "COLLATE") {
                if collation.is_some() {
                    return Err("Duplicate COLLATE clause in CREATE DOMAIN".to_string());
                }
                let (coll, next) = take_collation_name(rest)?;
                collation = Some(coll);
                rest = next.trim_start();
                continue;
            }

            if take_keyword_ci(&mut rest, "DEFAULT") {
                if default_expr.is_some() {
                    return Err(
                        "Multiple DEFAULT clauses are not allowed in CREATE DOMAIN".to_string()
                    );
                }
                let (expr, next) = take_default_expression(rest)?;
                if expr.is_empty() {
                    return Err("DEFAULT expression cannot be empty in CREATE DOMAIN".to_string());
                }
                default_expr = Some(expr);
                rest = next.trim_start();
                continue;
            }

            let mut constraint_name = None;
            if take_keyword_ci(&mut rest, "CONSTRAINT") {
                let (cname, after_cname) = take_ident(rest)?;
                if cname.is_empty() {
                    return Err("Constraint name cannot be empty".to_string());
                }
                if !constraint_names.insert(cname.clone()) {
                    return Err(format!("Duplicate constraint name '{cname}' on domain '{name}'"));
                }
                constraint_name = Some(cname);
                rest = after_cname.trim_start();
            }

            if take_keyword_ci(&mut rest, "NOT NULL") {
                if seen_null {
                    return Err(
                        "Conflicting NULL and NOT NULL constraints in CREATE DOMAIN".to_string()
                    );
                }
                seen_not_null = true;
                rest = rest.trim_start();
                continue;
            }

            if take_keyword_ci(&mut rest, "NULL") {
                if seen_not_null {
                    return Err(
                        "Conflicting NULL and NOT NULL constraints in CREATE DOMAIN".to_string()
                    );
                }
                seen_null = true;
                rest = rest.trim_start();
                continue;
            }

            if take_keyword_ci(&mut rest, "CHECK") {
                rest = rest.trim_start();
                if !rest.starts_with('(') {
                    return Err("Expected '(' after CHECK in domain constraint".to_string());
                }
                let close = matching_paren(rest)
                    .ok_or_else(|| "Unterminated CHECK (...) in CREATE DOMAIN".to_string())?;
                let expr = rest[1..close].trim().to_string();
                if expr.is_empty() {
                    return Err("CHECK constraint expression cannot be empty".to_string());
                }
                rest = rest[close + 1..].trim_start();
                let not_valid = take_keyword_ci(&mut rest, "NOT VALID");
                checks.push(DomainCheckClause {
                    name:       constraint_name,
                    expression: expr,
                    validated:  !not_valid,
                });
                continue;
            }

            let unexpected = rest.split_whitespace().next().unwrap_or(rest);
            return Err(format!("Unexpected token '{unexpected}' in CREATE DOMAIN"));
        }

        Ok(Self {
            namespace_id,
            name,
            base_type,
            collation,
            default_expr,
            not_null: seen_not_null,
            checks,
        })
    }
}

pub(crate) fn take_collation_name(input: &str) -> DdlResult<(String, &str)> {
    let trimmed = input.trim_start();
    if trimmed.is_empty() {
        return Err("Expected collation name after COLLATE".to_string());
    }
    let (qual, rest) = split_qualified_ident(trimmed)?;
    let display = match qual.namespace_id {
        Some(ns) => format!("{}.{}", ns.as_str(), qual.name),
        None => qual.name,
    };
    Ok((display, rest))
}

pub(crate) fn take_default_expression(input: &str) -> DdlResult<(String, &str)> {
    let trimmed = input.trim_start();
    if trimmed.is_empty() {
        return Err("Expected expression after DEFAULT".to_string());
    }
    let mut depth = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = trimmed.char_indices().peekable();
    let mut end = trimmed.len();

    while let Some((idx, ch)) = chars.next() {
        if in_single {
            if ch == '\'' {
                if chars.peek().map(|(_, next)| *next) == Some('\'') {
                    chars.next();
                } else {
                    in_single = false;
                }
            }
            continue;
        }
        if in_double {
            if ch == '"' {
                if chars.peek().map(|(_, next)| *next) == Some('"') {
                    chars.next();
                } else {
                    in_double = false;
                }
            }
            continue;
        }
        match ch {
            '\'' => in_single = true,
            '"' => in_double = true,
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return Err("Unexpected ')' in DEFAULT expression".to_string());
                }
                depth -= 1;
            },
            _ if depth == 0 && ch.is_whitespace() => {
                let tail = trimmed[idx..].trim_start();
                let mut probe = tail;
                if take_keyword_ci(&mut probe, "CONSTRAINT")
                    || take_keyword_ci(&mut probe, "NOT NULL")
                    || take_keyword_ci(&mut probe, "NULL")
                    || take_keyword_ci(&mut probe, "CHECK")
                    || take_keyword_ci(&mut probe, "COLLATE")
                {
                    end = idx;
                    break;
                }
            },
            _ => {},
        }
    }

    if in_single || in_double || depth != 0 {
        return Err("Unterminated DEFAULT expression in domain definition".to_string());
    }

    Ok((trimmed[..end].trim().to_string(), &trimmed[end..]))
}
