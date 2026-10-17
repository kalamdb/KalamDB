//! ALTER TYPE composite and enum operations.

use kalamdb_commons::models::SchemaId;

use crate::ddl::{
    create_domain::take_collation_name,
    create_type::{
        parse_type_reference, split_qualified_ident, take_ident, validate_enum_label, TypeReference,
    },
    parsing::{parse_sql_string_prefix, take_keyword_ci},
    DdlResult,
};

/// Position of a newly added enum label relative to an existing label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnumValueNeighbor {
    Before(String),
    After(String),
}

/// ALTER TYPE operations supported in V1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlterTypeOperation {
    AddAttribute {
        field:    String,
        type_ref: TypeReference,
    },
    DropAttribute {
        field:   String,
        cascade: bool,
    },
    RenameAttribute {
        from: String,
        to:   String,
    },
    AlterAttributeType {
        field:    String,
        type_ref: TypeReference,
    },
    AddValue {
        label:         String,
        if_not_exists: bool,
        neighbor:      Option<EnumValueNeighbor>,
    },
    RenameValue {
        from: String,
        to:   String,
    },
    SetSchema {
        schema: SchemaId,
    },
    RenameType {
        new_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlterTypeStatement {
    pub namespace_id: SchemaId,
    pub name:         String,
    pub operation:    AlterTypeOperation,
}

impl AlterTypeStatement {
    pub fn parse(sql: &str, default_namespace: &SchemaId) -> DdlResult<Self> {
        let mut rest = sql.trim().trim_end_matches(';');
        if !take_keyword_ci(&mut rest, "ALTER TYPE") {
            return Err("Expected ALTER TYPE statement".to_string());
        }
        let (qual, after) = split_qualified_ident(rest)?;
        let namespace_id = qual.namespace_or(default_namespace);
        let mut after = after.trim_start();

        let operation = if take_keyword_ci(&mut after, "ADD VALUE") {
            parse_add_value(after)?
        } else if take_keyword_ci(&mut after, "RENAME VALUE") {
            parse_rename_value(after)?
        } else if take_keyword_ci(&mut after, "ADD ATTRIBUTE") {
            let mut rest = after.trim_start();
            if take_keyword_ci(&mut rest, "IF NOT EXISTS") {
                return Err("ALTER TYPE ... ADD ATTRIBUTE IF NOT EXISTS is not supported in \
                            PostgreSQL 18"
                    .to_string());
            }
            let (field, rest) = take_ident(rest)?;
            let (type_ref, leftover) = parse_type_reference(rest.trim_start())?;
            let mut leftover = leftover.trim_start();
            if take_keyword_ci(&mut leftover, "COLLATE") {
                let (_coll, next) = take_collation_name(leftover)?;
                leftover = next.trim_start();
            }
            let _ = take_keyword_ci(&mut leftover, "CASCADE")
                || take_keyword_ci(&mut leftover, "RESTRICT");
            if !leftover.trim().is_empty() {
                let mut probe = leftover;
                if take_keyword_ci(&mut probe, "NOT NULL")
                    || take_keyword_ci(&mut probe, "NULL")
                    || take_keyword_ci(&mut probe, "NONEMPTY")
                    || take_keyword_ci(&mut probe, "DEFAULT")
                    || take_keyword_ci(&mut probe, "CHECK")
                {
                    return Err(format!(
                        "Composite type attribute '{field}' cannot declare NOT NULL, DEFAULT, \
                         CHECK, or NONEMPTY constraints in PostgreSQL 18; use CREATE DOMAIN for \
                         reusable value constraints"
                    ));
                }
                return Err(format!(
                    "Unexpected tokens after ADD ATTRIBUTE: '{}'",
                    leftover.trim()
                ));
            }
            AlterTypeOperation::AddAttribute { field, type_ref }
        } else if take_keyword_ci(&mut after, "DROP ATTRIBUTE") {
            let mut rest = after.trim_start();
            let _if_exists = take_keyword_ci(&mut rest, "IF EXISTS");
            let (field, rest) = take_ident(rest)?;
            let mut leftover = rest.trim_start();
            let cascade = take_keyword_ci(&mut leftover, "CASCADE");
            if !cascade {
                let _ = take_keyword_ci(&mut leftover, "RESTRICT");
            }
            if !leftover.trim().is_empty() {
                return Err(format!(
                    "Unexpected tokens after DROP ATTRIBUTE: '{}'",
                    leftover.trim()
                ));
            }
            AlterTypeOperation::DropAttribute { field, cascade }
        } else if take_keyword_ci(&mut after, "RENAME TO") {
            let (new_name, leftover) = take_ident(after.trim_start())?;
            if !leftover.trim().is_empty() {
                return Err("Unexpected tokens after RENAME TO".to_string());
            }
            if new_name.is_empty() {
                return Err("Expected type name after RENAME TO".to_string());
            }
            AlterTypeOperation::RenameType { new_name }
        } else if take_keyword_ci(&mut after, "RENAME ATTRIBUTE") {
            let (from, rest) = take_ident(after.trim_start())?;
            let mut rest = rest.trim_start();
            if !take_keyword_ci(&mut rest, "TO") {
                return Err("Expected TO in RENAME ATTRIBUTE".to_string());
            }
            let (to, leftover) = take_ident(rest.trim_start())?;
            let mut leftover = leftover.trim_start();
            let _ = take_keyword_ci(&mut leftover, "CASCADE")
                || take_keyword_ci(&mut leftover, "RESTRICT");
            if !leftover.trim().is_empty() {
                return Err(format!(
                    "Unexpected tokens after RENAME ATTRIBUTE: '{}'",
                    leftover.trim()
                ));
            }
            AlterTypeOperation::RenameAttribute { from, to }
        } else if take_keyword_ci(&mut after, "ALTER ATTRIBUTE") {
            let (field, rest) = take_ident(after.trim_start())?;
            let mut rest = rest.trim_start();
            let _ = take_keyword_ci(&mut rest, "SET DATA");
            if !take_keyword_ci(&mut rest, "TYPE") {
                return Err("Expected TYPE in ALTER ATTRIBUTE".to_string());
            }
            let (type_ref, leftover) = parse_type_reference(rest.trim_start())?;
            let mut leftover = leftover.trim_start();
            if take_keyword_ci(&mut leftover, "COLLATE") {
                let (_coll, next) = take_collation_name(leftover)?;
                leftover = next.trim_start();
            }
            let _ = take_keyword_ci(&mut leftover, "CASCADE")
                || take_keyword_ci(&mut leftover, "RESTRICT");
            if !leftover.trim().is_empty() {
                return Err(format!(
                    "Unexpected tokens after ALTER ATTRIBUTE TYPE: '{}'",
                    leftover.trim()
                ));
            }
            AlterTypeOperation::AlterAttributeType { field, type_ref }
        } else if take_keyword_ci(&mut after, "SET SCHEMA") {
            let (schema_name, schema_quoted, leftover) =
                crate::ddl::create_type::take_ident_with_quote(after.trim_start())?;
            if !leftover.trim().is_empty() {
                return Err(format!("Unexpected tokens after SET SCHEMA: '{}'", leftover.trim()));
            }
            let schema = if schema_quoted {
                SchemaId::try_new(schema_name).map_err(|error| error.to_string())?
            } else {
                SchemaId::try_parse_reference(schema_name).map_err(|error| error.to_string())?
            };
            AlterTypeOperation::SetSchema { schema }
        } else {
            return Err("Expected ADD VALUE, RENAME VALUE, ADD/DROP/RENAME/ALTER ATTRIBUTE, \
                        RENAME TO, or SET SCHEMA"
                .to_string());
        };

        Ok(Self {
            namespace_id,
            name: qual.name,
            operation,
        })
    }
}

fn parse_add_value(after: &str) -> DdlResult<AlterTypeOperation> {
    let mut rest = after.trim_start();
    let if_not_exists = take_keyword_ci(&mut rest, "IF NOT EXISTS");
    let (label, leftover) = parse_sql_string_prefix(rest)?;
    validate_enum_label(&label)?;
    let mut leftover = leftover.trim_start();
    let neighbor = if take_keyword_ci(&mut leftover, "BEFORE") {
        let (neighbor, rest) = parse_sql_string_prefix(leftover)?;
        leftover = rest.trim_start();
        Some(EnumValueNeighbor::Before(neighbor))
    } else if take_keyword_ci(&mut leftover, "AFTER") {
        let (neighbor, rest) = parse_sql_string_prefix(leftover)?;
        leftover = rest.trim_start();
        Some(EnumValueNeighbor::After(neighbor))
    } else {
        None
    };
    if !leftover.is_empty() {
        return Err(format!("Unexpected tokens after ADD VALUE '{}'", leftover.trim()));
    }
    Ok(AlterTypeOperation::AddValue {
        label,
        if_not_exists,
        neighbor,
    })
}

fn parse_rename_value(after: &str) -> DdlResult<AlterTypeOperation> {
    let rest = after.trim_start();
    let (from, leftover) = parse_sql_string_prefix(rest)?;
    validate_enum_label(&from)?;
    let mut leftover = leftover.trim_start();
    if !take_keyword_ci(&mut leftover, "TO") {
        return Err("Expected TO in RENAME VALUE".to_string());
    }
    let (to, rest) = parse_sql_string_prefix(leftover.trim_start())?;
    validate_enum_label(&to)?;
    if !rest.trim().is_empty() {
        return Err(format!("Unexpected tokens after RENAME VALUE '{}'", rest.trim()));
    }
    Ok(AlterTypeOperation::RenameValue { from, to })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns() -> SchemaId {
        SchemaId::new("app")
    }

    #[test]
    fn parse_add_attribute() {
        let stmt = AlterTypeStatement::parse(
            "ALTER TYPE app.address ADD ATTRIBUTE postal_code TEXT",
            &ns(),
        )
        .unwrap();
        match stmt.operation {
            AlterTypeOperation::AddAttribute { field, .. } => {
                assert_eq!(field, "postal_code");
            },
            _ => panic!("expected add"),
        }
    }

    #[test]
    fn parse_add_value_append() {
        let stmt =
            AlterTypeStatement::parse("ALTER TYPE app.status ADD VALUE 'archived'", &ns()).unwrap();
        match stmt.operation {
            AlterTypeOperation::AddValue {
                label,
                if_not_exists,
                neighbor,
            } => {
                assert_eq!(label, "archived");
                assert!(!if_not_exists);
                assert!(neighbor.is_none());
            },
            _ => panic!("expected add value"),
        }
    }

    #[test]
    fn parse_add_value_if_not_exists_before() {
        let stmt = AlterTypeStatement::parse(
            "ALTER TYPE app.status ADD VALUE IF NOT EXISTS 'pending' BEFORE 'active'",
            &ns(),
        )
        .unwrap();
        match stmt.operation {
            AlterTypeOperation::AddValue {
                label,
                if_not_exists,
                neighbor,
            } => {
                assert_eq!(label, "pending");
                assert!(if_not_exists);
                assert_eq!(neighbor, Some(EnumValueNeighbor::Before("active".into())));
            },
            _ => panic!("expected add value"),
        }
    }

    #[test]
    fn parse_add_value_after_preserves_label_case() {
        let stmt = AlterTypeStatement::parse(
            "ALTER TYPE app.status ADD VALUE 'Beta' AFTER 'Alpha'",
            &ns(),
        )
        .unwrap();
        match stmt.operation {
            AlterTypeOperation::AddValue {
                label, neighbor, ..
            } => {
                assert_eq!(label, "Beta");
                assert_eq!(neighbor, Some(EnumValueNeighbor::After("Alpha".into())));
            },
            _ => panic!("expected add value"),
        }
    }

    #[test]
    fn parse_rename_type() {
        let stmt =
            AlterTypeStatement::parse("ALTER TYPE app.address RENAME TO user_address", &ns())
                .unwrap();
        match stmt.operation {
            AlterTypeOperation::RenameType { new_name } => {
                assert_eq!(new_name, "user_address");
                assert_eq!(stmt.name, "address");
            },
            _ => panic!("expected rename type"),
        }
    }

    #[test]
    fn allow_empty_and_colon_add_value_and_reject_over_63_bytes() {
        let stmt = AlterTypeStatement::parse("ALTER TYPE app.status ADD VALUE ''", &ns()).unwrap();
        assert!(matches!(
            stmt.operation,
            AlterTypeOperation::AddValue { ref label, .. } if label.is_empty()
        ));

        let over = format!("ALTER TYPE app.status ADD VALUE '{}'", "x".repeat(64));
        let err = AlterTypeStatement::parse(&over, &ns()).unwrap_err();
        assert!(err.contains("63 bytes"), "{err}");
    }
}
