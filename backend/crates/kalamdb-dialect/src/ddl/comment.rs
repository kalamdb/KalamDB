//! COMMENT ON TYPE / COMMENT ON PROCEDURE via sqlparser.

use kalamdb_commons::models::{NamespaceId, RoutineId, TypeId};
use sqlparser::{
    ast::{CommentObject, ObjectName, ObjectNamePart, Statement},
    dialect::PostgreSqlDialect,
};

use crate::{ddl::DdlResult, parser::utils::parse_sql_statements};

/// Catalog object targeted by `COMMENT ON`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentOnTarget {
    Type(TypeId),
    Procedure(RoutineId),
}

/// Parsed `COMMENT ON TYPE|PROCEDURE ... IS ...` statement.
///
/// sqlparser 0.62 already understands PostgreSQL `COMMENT ON` (see
/// `PostgreSqlDialect::supports_comment_on`). KalamDB persists comments only
/// for types and procedures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentOnStatement {
    pub target:    CommentOnTarget,
    pub comment:   Option<String>,
    pub if_exists: bool,
}

impl CommentOnStatement {
    pub fn parse(sql: &str, default_namespace: &NamespaceId) -> DdlResult<Self> {
        let dialect = PostgreSqlDialect {};
        let mut statements = parse_sql_statements(sql, &dialect)
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
                let target = match object_type {
                    CommentObject::Type => {
                        CommentOnTarget::Type(TypeId::from_parts(Some(&namespace_id), &name))
                    },
                    CommentObject::Procedure => CommentOnTarget::Procedure(RoutineId::from_parts(
                        Some(&namespace_id),
                        &name.to_ascii_lowercase(),
                    )),
                    other => {
                        return Err(format!(
                            "COMMENT ON {other} is not supported; use COMMENT ON TYPE or COMMENT \
                             ON PROCEDURE"
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

fn resolve_object_name(
    name: &ObjectName,
    default_namespace: &NamespaceId,
) -> DdlResult<(NamespaceId, String)> {
    let parts: Vec<&str> = name
        .0
        .iter()
        .map(|part| match part {
            ObjectNamePart::Identifier(ident) => Ok(ident.value.as_str()),
            ObjectNamePart::Function(_) => {
                Err("COMMENT ON does not accept function-call object names".to_string())
            },
        })
        .collect::<DdlResult<_>>()?;
    match parts.as_slice() {
        [object] => Ok((default_namespace.clone(), (*object).to_string())),
        [namespace, object] => Ok((NamespaceId::new(*namespace), (*object).to_string())),
        _ => Err("COMMENT ON expects <name> or <schema>.<name>".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns() -> NamespaceId {
        NamespaceId::new("app")
    }

    #[test]
    fn parse_comment_on_type() {
        let stmt = CommentOnStatement::parse(
            "COMMENT ON TYPE chat.address IS 'Postal address for a user'",
            &ns(),
        )
        .unwrap();
        match stmt.target {
            CommentOnTarget::Type(type_id) => assert_eq!(type_id.as_str(), "chat.address"),
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
