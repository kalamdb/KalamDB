//! ALTER DOMAIN parser for PostgreSQL 18 domain types.

use kalamdb_commons::models::SchemaId;

use crate::ddl::{
    create_domain::{take_default_expression, DomainCheckClause},
    create_type::{matching_paren, split_qualified_ident, take_ident},
    parsing::take_keyword_ci,
    DdlResult,
};

/// Supported PostgreSQL 18 `ALTER DOMAIN` operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlterDomainOperation {
    SetDefault {
        expr: String,
    },
    DropDefault,
    SetNotNull,
    DropNotNull,
    AddCheck {
        clause: DomainCheckClause,
    },
    AddNotNull {
        constraint_name: Option<String>,
        validated:       bool,
    },
    DropConstraint {
        name:      String,
        if_exists: bool,
        cascade:   bool,
    },
    RenameConstraint {
        from: String,
        to:   String,
    },
    ValidateConstraint {
        name: String,
    },
    OwnerTo {
        new_owner: String,
    },
    RenameTo {
        new_name: String,
    },
    SetSchema {
        schema: SchemaId,
    },
}

/// Parsed `ALTER DOMAIN` statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlterDomainStatement {
    pub namespace_id: SchemaId,
    pub name:         String,
    pub operation:    AlterDomainOperation,
}

impl AlterDomainStatement {
    pub fn parse(sql: &str, default_namespace: &SchemaId) -> DdlResult<Self> {
        let mut rest = sql.trim().trim_end_matches(';');
        if !take_keyword_ci(&mut rest, "ALTER DOMAIN") {
            return Err("Expected ALTER DOMAIN statement".to_string());
        }
        let (qual, after_name) = split_qualified_ident(rest)?;
        let namespace_id = qual.namespace_or(default_namespace);
        let name = qual.name;
        let mut rest = after_name.trim_start();

        let operation = if take_keyword_ci(&mut rest, "SET DEFAULT") {
            let (expr, leftover) = take_default_expression(rest)?;
            if !leftover.trim().is_empty() {
                return Err(format!("Unexpected tokens after SET DEFAULT: '{}'", leftover.trim()));
            }
            AlterDomainOperation::SetDefault { expr }
        } else if take_keyword_ci(&mut rest, "DROP DEFAULT") {
            if !rest.trim().is_empty() {
                return Err(format!("Unexpected tokens after DROP DEFAULT: '{}'", rest.trim()));
            }
            AlterDomainOperation::DropDefault
        } else if take_keyword_ci(&mut rest, "SET NOT NULL") {
            if !rest.trim().is_empty() {
                return Err(format!("Unexpected tokens after SET NOT NULL: '{}'", rest.trim()));
            }
            AlterDomainOperation::SetNotNull
        } else if take_keyword_ci(&mut rest, "DROP NOT NULL") {
            if !rest.trim().is_empty() {
                return Err(format!("Unexpected tokens after DROP NOT NULL: '{}'", rest.trim()));
            }
            AlterDomainOperation::DropNotNull
        } else if take_keyword_ci(&mut rest, "ADD") {
            let mut constraint_name = None;
            if take_keyword_ci(&mut rest, "CONSTRAINT") {
                let (cname, after_cname) = take_ident(rest)?;
                constraint_name = Some(cname);
                rest = after_cname.trim_start();
            }
            if take_keyword_ci(&mut rest, "NOT NULL") {
                let not_valid = take_keyword_ci(&mut rest, "NOT VALID");
                if !rest.trim().is_empty() {
                    return Err(format!("Unexpected tokens after ADD NOT NULL: '{}'", rest.trim()));
                }
                AlterDomainOperation::AddNotNull {
                    constraint_name,
                    validated: !not_valid,
                }
            } else if take_keyword_ci(&mut rest, "CHECK") {
                rest = rest.trim_start();
                if !rest.starts_with('(') {
                    return Err("Expected '(' after CHECK in ALTER DOMAIN ADD".to_string());
                }
                let close = matching_paren(rest)
                    .ok_or_else(|| "Unterminated CHECK (...) in ALTER DOMAIN".to_string())?;
                let expr = rest[1..close].trim().to_string();
                if expr.is_empty() {
                    return Err("CHECK constraint expression cannot be empty".to_string());
                }
                rest = rest[close + 1..].trim_start();
                let not_valid = take_keyword_ci(&mut rest, "NOT VALID");
                if !rest.trim().is_empty() {
                    return Err(format!("Unexpected tokens after ADD CHECK: '{}'", rest.trim()));
                }
                AlterDomainOperation::AddCheck {
                    clause: DomainCheckClause {
                        name:       constraint_name,
                        expression: expr,
                        validated:  !not_valid,
                    },
                }
            } else {
                return Err("Expected CHECK (...) or NOT NULL in ALTER DOMAIN ADD".to_string());
            }
        } else if take_keyword_ci(&mut rest, "DROP CONSTRAINT") {
            let if_exists = take_keyword_ci(&mut rest, "IF EXISTS");
            let (cname, after_cname) = take_ident(rest)?;
            let mut leftover = after_cname.trim_start();
            let cascade = take_keyword_ci(&mut leftover, "CASCADE");
            if !cascade {
                let _ = take_keyword_ci(&mut leftover, "RESTRICT");
            }
            if !leftover.trim().is_empty() {
                return Err(format!(
                    "Unexpected tokens after DROP CONSTRAINT: '{}'",
                    leftover.trim()
                ));
            }
            AlterDomainOperation::DropConstraint {
                name: cname,
                if_exists,
                cascade,
            }
        } else if take_keyword_ci(&mut rest, "RENAME CONSTRAINT") {
            let (from, after_from) = take_ident(rest)?;
            let mut after_from = after_from.trim_start();
            if !take_keyword_ci(&mut after_from, "TO") {
                return Err("Expected TO in ALTER DOMAIN RENAME CONSTRAINT".to_string());
            }
            let (to, leftover) = take_ident(after_from)?;
            if !leftover.trim().is_empty() {
                return Err(format!(
                    "Unexpected tokens after RENAME CONSTRAINT: '{}'",
                    leftover.trim()
                ));
            }
            AlterDomainOperation::RenameConstraint { from, to }
        } else if take_keyword_ci(&mut rest, "VALIDATE CONSTRAINT") {
            let (cname, leftover) = take_ident(rest)?;
            if !leftover.trim().is_empty() {
                return Err(format!(
                    "Unexpected tokens after VALIDATE CONSTRAINT: '{}'",
                    leftover.trim()
                ));
            }
            AlterDomainOperation::ValidateConstraint { name: cname }
        } else if take_keyword_ci(&mut rest, "OWNER TO") {
            let (owner, leftover) = take_ident(rest)?;
            if !leftover.trim().is_empty() {
                return Err(format!("Unexpected tokens after OWNER TO: '{}'", leftover.trim()));
            }
            AlterDomainOperation::OwnerTo { new_owner: owner }
        } else if take_keyword_ci(&mut rest, "RENAME TO") {
            let (new_name, leftover) = take_ident(rest)?;
            if !leftover.trim().is_empty() {
                return Err(format!("Unexpected tokens after RENAME TO: '{}'", leftover.trim()));
            }
            AlterDomainOperation::RenameTo { new_name }
        } else if take_keyword_ci(&mut rest, "SET SCHEMA") {
            let (schema_name, leftover) = take_ident(rest)?;
            if !leftover.trim().is_empty() {
                return Err(format!("Unexpected tokens after SET SCHEMA: '{}'", leftover.trim()));
            }
            let schema =
                SchemaId::try_parse_reference(&schema_name).map_err(|error| error.to_string())?;
            AlterDomainOperation::SetSchema { schema }
        } else {
            return Err("Expected SET/DROP DEFAULT, SET/DROP NOT NULL, ADD/DROP/RENAME/VALIDATE \
                        CONSTRAINT, OWNER TO, RENAME TO, or SET SCHEMA in ALTER DOMAIN"
                .to_string());
        };

        Ok(Self {
            namespace_id,
            name,
            operation,
        })
    }
}
