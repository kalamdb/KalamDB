//! COMMENT ON TYPE / DOMAIN / FUNCTION / PROCEDURE parser.

use kalamdb_commons::models::{RoutineId, RoutineKind, SchemaId};
use sqlparser::ast::{CommentObject, ObjectName, ObjectNamePart, Statement};

use crate::{
    ddl::{
        create_type::{matching_paren, split_qualified_ident},
        parsing::{parse_sql_string_prefix, take_keyword_ci},
        DdlResult,
    },
    parser::KalamParser,
};

/// Catalog object targeted by `COMMENT ON`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentOnTarget {
    Type {
        namespace_id: SchemaId,
        name: String,
    },
    Procedure(RoutineId),
    Function(RoutineId),
}

/// Parsed `COMMENT ON TYPE|DOMAIN|FUNCTION|PROCEDURE ... IS ...` statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentOnStatement {
    pub target: CommentOnTarget,
    pub comment: Option<String>,
    pub if_exists: bool,
}

impl CommentOnStatement {
    pub fn parse(sql: &str, default_namespace: &SchemaId) -> DdlResult<Self> {
        if let Some(routine_comment) = try_parse_routine_comment(sql, default_namespace)? {
            return Ok(routine_comment);
        }

        let mut statements = KalamParser::parse(sql)
            .map_err(|error| format!("Failed to parse COMMENT ON statement: {error}"))?;
        if statements.len() != 1 {
            return Err("Expected exactly one COMMENT ON statement".to_string());
        }
        match statements.remove(0) {
            Statement::Comment {
                object_type,
                object_name,
                comment,
                if_exists,
            } => {
                let (namespace_id, name) = resolve_object_name(&object_name, default_namespace)?;
                let target =
                    match object_type {
                        CommentObject::Type | CommentObject::Domain => {
                            CommentOnTarget::Type { namespace_id, name }
                        },
                        CommentObject::Procedure => CommentOnTarget::Procedure(
                            RoutineId::from_parts(Some(&namespace_id), &name.to_ascii_lowercase()),
                        ),
                        CommentObject::Function => CommentOnTarget::Function(
                            RoutineId::from_parts(Some(&namespace_id), &name.to_ascii_lowercase()),
                        ),
                        other => {
                            return Err(format!(
                                "COMMENT ON {other} is not supported; use COMMENT ON TYPE, \
                                 COMMENT ON DOMAIN, COMMENT ON FUNCTION, or COMMENT ON PROCEDURE"
                            ));
                        },
                    };
                Ok(Self {
                    target,
                    comment,
                    if_exists,
                })
            },
            other => Err(format!("Expected COMMENT ON statement, got {other}")),
        }
    }
}

fn try_parse_routine_comment(
    sql: &str,
    default_namespace: &SchemaId,
) -> DdlResult<Option<CommentOnStatement>> {
    let mut rest = sql.trim().trim_end_matches(';');
    let kind = if take_keyword_ci(&mut rest, "COMMENT ON FUNCTION") {
        RoutineKind::Function
    } else if take_keyword_ci(&mut rest, "COMMENT ON PROCEDURE") {
        RoutineKind::Procedure
    } else {
        return Ok(None);
    };
    let if_exists = take_keyword_ci(&mut rest, "IF EXISTS");
    let (qual, after_name) = split_qualified_ident(rest)?;
    let mut after = after_name.trim_start();
    if after.starts_with('(') {
        let close = matching_paren(after)
            .ok_or_else(|| "Unterminated argument list in COMMENT ON routine".to_string())?;
        after = after[close + 1..].trim_start();
    }
    if !take_keyword_ci(&mut after, "IS") {
        return Err("Expected IS after routine name in COMMENT ON".to_string());
    }
    let comment = if take_keyword_ci(&mut after, "NULL") {
        if !after.trim().is_empty() {
            return Err(format!(
                "Unexpected tokens after IS NULL in COMMENT ON: '{}'",
                after.trim()
            ));
        }
        None
    } else {
        let (text, leftover) = parse_sql_string_prefix(after.trim_start())?;
        if !leftover.trim().is_empty() {
            return Err(format!(
                "Unexpected tokens after comment string in COMMENT ON: '{}'",
                leftover.trim()
            ));
        }
        Some(text)
    };
    let namespace_id = qual.namespace_or(default_namespace);
    let routine_id = RoutineId::from_parts(Some(&namespace_id), &qual.name);
    let target = match kind {
        RoutineKind::Function => CommentOnTarget::Function(routine_id),
        RoutineKind::Procedure => CommentOnTarget::Procedure(routine_id),
    };
    Ok(Some(CommentOnStatement {
        target,
        comment,
        if_exists,
    }))
}

fn resolve_object_name(
    name: &ObjectName,
    default_namespace: &SchemaId,
) -> DdlResult<(SchemaId, String)> {
    let parts: Vec<String> = name
        .0
        .iter()
        .map(|part| match part {
            ObjectNamePart::Identifier(ident) => {
                if ident.quote_style.is_some() {
                    Ok(ident.value.clone())
                } else {
                    Ok(ident.value.to_ascii_lowercase())
                }
            },
            ObjectNamePart::Function(_) => {
                Err("COMMENT ON does not accept function-call object names".to_string())
            },
        })
        .collect::<DdlResult<_>>()?;
    match parts.as_slice() {
        [object] => Ok((default_namespace.clone(), object.clone())),
        [namespace, object] => Ok((SchemaId::new(namespace.as_str()), object.clone())),
        _ => Err("COMMENT ON expects <name> or <schema>.<name>".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns() -> SchemaId {
        SchemaId::new("app")
    }

    #[test]
    fn parse_comment_on_type() {
        let stmt = CommentOnStatement::parse(
            "COMMENT ON TYPE chat.address IS 'Postal address for a user'",
            &ns(),
        )
        .unwrap();
        match stmt.target {
            CommentOnTarget::Type { namespace_id, name } => {
                assert_eq!(namespace_id.as_str(), "chat");
                assert_eq!(name, "address");
            },
            other => panic!("expected type, got {other:?}"),
        }
        assert_eq!(stmt.comment.as_deref(), Some("Postal address for a user"));
        assert!(!stmt.if_exists);
    }

    #[test]
    fn parse_comment_on_procedure_clears_with_null() {
        let stmt =
            CommentOnStatement::parse("COMMENT ON PROCEDURE chat.send_message IS NULL", &ns())
                .unwrap();
        match stmt.target {
            CommentOnTarget::Procedure(routine_id) => {
                assert_eq!(routine_id.as_str(), "chat.send_message")
            },
            other => panic!("expected procedure, got {other:?}"),
        }
        assert!(stmt.comment.is_none());
    }

    #[test]
    fn parse_comment_on_function_with_args() {
        let stmt = CommentOnStatement::parse(
            "COMMENT ON FUNCTION chat.format_name(TEXT, TEXT) IS 'Format user display name'",
            &ns(),
        )
        .unwrap();
        match stmt.target {
            CommentOnTarget::Function(routine_id) => {
                assert_eq!(routine_id.as_str(), "chat.format_name")
            },
            other => panic!("expected function, got {other:?}"),
        }
        assert_eq!(stmt.comment.as_deref(), Some("Format user display name"));
    }

    #[test]
    fn parse_unqualified_procedure_uses_default_namespace() {
        let stmt = CommentOnStatement::parse("COMMENT ON PROCEDURE ping IS 'health check'", &ns())
            .unwrap();
        match stmt.target {
            CommentOnTarget::Procedure(routine_id) => {
                assert_eq!(routine_id.as_str(), "app.ping")
            },
            other => panic!("expected procedure, got {other:?}"),
        }
    }

    #[test]
    fn reject_comment_on_table() {
        let err =
            CommentOnStatement::parse("COMMENT ON TABLE chat.users IS 'rows'", &ns()).unwrap_err();
        assert!(err.contains("COMMENT ON TABLE"), "{err}");
        assert!(err.contains("TYPE"), "{err}");
    }
}
