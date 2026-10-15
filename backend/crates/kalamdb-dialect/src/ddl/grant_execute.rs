//! GRANT / REVOKE EXECUTE ON FUNCTION / PROCEDURE / ALL FUNCTIONS|PROCEDURES IN SCHEMA parsers.

use kalamdb_commons::models::{RoutineId, RoutineKind, SchemaId};

use crate::ddl::{
    create_type::{matching_paren, split_qualified_ident, take_ident},
    parsing::take_keyword_ci,
    DdlResult,
};

/// Principal that can be granted EXECUTE on a routine.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExecuteGrantee {
    Public,
    User,
    Service,
    Anonymous,
    Role(String),
}

impl ExecuteGrantee {
    pub fn parse(value: &str) -> DdlResult<Self> {
        let (ident, leftover) = take_ident(value)?;
        if !leftover.trim().is_empty() {
            return Err("Unexpected tokens after EXECUTE grantee".to_string());
        }
        Ok(Self::from_ident(&ident))
    }

    pub fn from_ident(ident: &str) -> Self {
        match ident.to_ascii_uppercase().as_str() {
            "PUBLIC" => Self::Public,
            "USER" => Self::User,
            "SERVICE" => Self::Service,
            "ANONYMOUS" => Self::Anonymous,
            _ => Self::Role(ident.to_ascii_lowercase()),
        }
    }

    pub fn as_sql(&self) -> String {
        match self {
            Self::Public => "PUBLIC".to_string(),
            Self::User => "user".to_string(),
            Self::Service => "service".to_string(),
            Self::Anonymous => "anonymous".to_string(),
            Self::Role(name) => name.clone(),
        }
    }
}

/// Target of a `GRANT EXECUTE` or `REVOKE EXECUTE` statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecuteTarget {
    Routine {
        routine_id:    RoutineId,
        expected_kind: RoutineKind,
    },
    AllInSchema {
        namespace_id:  SchemaId,
        expected_kind: RoutineKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantExecuteStatement {
    pub routine_id: RoutineId,
    pub target:     ExecuteTarget,
    pub grantee:    ExecuteGrantee,
}

impl GrantExecuteStatement {
    pub fn parse(sql: &str, default_namespace: &SchemaId) -> DdlResult<Self> {
        parse_execute_acl(sql, default_namespace, true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokeExecuteStatement {
    pub routine_id: RoutineId,
    pub target:     ExecuteTarget,
    pub grantee:    ExecuteGrantee,
}

impl RevokeExecuteStatement {
    pub fn parse(sql: &str, default_namespace: &SchemaId) -> DdlResult<Self> {
        let grant = parse_execute_acl(sql, default_namespace, false)?;
        Ok(Self {
            routine_id: grant.routine_id,
            target:     grant.target,
            grantee:    grant.grantee,
        })
    }
}

fn parse_execute_acl(
    sql: &str,
    default_namespace: &SchemaId,
    is_grant: bool,
) -> DdlResult<GrantExecuteStatement> {
    let mut rest = sql.trim().trim_end_matches(';');
    let verb = if is_grant {
        "GRANT EXECUTE ON"
    } else {
        "REVOKE EXECUTE ON"
    };
    if !take_keyword_ci(&mut rest, verb) {
        return Err(format!("Expected {verb}"));
    }

    let connector = if is_grant { "TO" } else { "FROM" };

    if take_keyword_ci(&mut rest, "ALL FUNCTIONS IN SCHEMA") {
        let (schema_ident, after) = take_ident(rest)?;
        let mut after = after.trim_start();
        if !take_keyword_ci(&mut after, connector) {
            return Err(format!("Expected {connector} after schema name"));
        }
        let grantee = ExecuteGrantee::parse(after)?;
        let namespace_id = SchemaId::new(schema_ident);
        let routine_id = RoutineId::from_parts(Some(&namespace_id), "*");
        return Ok(GrantExecuteStatement {
            routine_id,
            target: ExecuteTarget::AllInSchema {
                namespace_id,
                expected_kind: RoutineKind::Function,
            },
            grantee,
        });
    }

    if take_keyword_ci(&mut rest, "ALL PROCEDURES IN SCHEMA") {
        let (schema_ident, after) = take_ident(rest)?;
        let mut after = after.trim_start();
        if !take_keyword_ci(&mut after, connector) {
            return Err(format!("Expected {connector} after schema name"));
        }
        let grantee = ExecuteGrantee::parse(after)?;
        let namespace_id = SchemaId::new(schema_ident);
        let routine_id = RoutineId::from_parts(Some(&namespace_id), "*");
        return Ok(GrantExecuteStatement {
            routine_id,
            target: ExecuteTarget::AllInSchema {
                namespace_id,
                expected_kind: RoutineKind::Procedure,
            },
            grantee,
        });
    }

    let expected_kind = if take_keyword_ci(&mut rest, "PROCEDURE") {
        RoutineKind::Procedure
    } else if take_keyword_ci(&mut rest, "FUNCTION") {
        RoutineKind::Function
    } else {
        return Err(format!(
            "Expected FUNCTION, PROCEDURE, ALL FUNCTIONS IN SCHEMA, or ALL PROCEDURES IN SCHEMA \
             after {verb}"
        ));
    };

    let (qual, after) = split_qualified_ident(rest)?;
    let mut after = after.trim_start();
    if after.starts_with('(') {
        let close = matching_paren(after)
            .ok_or_else(|| "Unterminated argument list in GRANT/REVOKE EXECUTE".to_string())?;
        after = after[close + 1..].trim_start();
    }
    if !take_keyword_ci(&mut after, connector) {
        return Err(format!(
            "Expected {connector} after {} name",
            expected_kind.as_str().to_ascii_lowercase()
        ));
    }
    let grantee = ExecuteGrantee::parse(after.trim_start())?;
    let namespace_id = qual.namespace_or(default_namespace);
    let routine_id = RoutineId::from_parts(Some(&namespace_id), &qual.name);
    Ok(GrantExecuteStatement {
        routine_id: routine_id.clone(),
        target: ExecuteTarget::Routine {
            routine_id,
            expected_kind,
        },
        grantee,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_grant_and_revoke() {
        let ns = SchemaId::new("app");
        let grant = GrantExecuteStatement::parse(
            "GRANT EXECUTE ON PROCEDURE api.create_order TO user",
            &ns,
        )
        .unwrap();
        assert_eq!(grant.routine_id.as_str(), "api.create_order");
        assert_eq!(grant.grantee, ExecuteGrantee::User);

        let grant_fn = GrantExecuteStatement::parse(
            "GRANT EXECUTE ON FUNCTION api.health(TEXT) TO anonymous",
            &ns,
        )
        .unwrap();
        assert_eq!(grant_fn.routine_id.as_str(), "api.health");
        assert_eq!(grant_fn.grantee, ExecuteGrantee::Anonymous);

        let all_fns = GrantExecuteStatement::parse(
            "GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA api TO service",
            &ns,
        )
        .unwrap();
        assert_eq!(
            all_fns.target,
            ExecuteTarget::AllInSchema {
                namespace_id:  SchemaId::new("api"),
                expected_kind: RoutineKind::Function,
            }
        );

        let revoke = RevokeExecuteStatement::parse(
            "REVOKE EXECUTE ON PROCEDURE api.create_order FROM PUBLIC",
            &ns,
        )
        .unwrap();
        assert_eq!(revoke.grantee, ExecuteGrantee::Public);
    }
}
