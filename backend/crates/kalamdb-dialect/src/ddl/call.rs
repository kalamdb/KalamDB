//! CALL procedure parser.

use kalamdb_commons::models::{NamespaceId, RoutineCall, RoutineId};

use crate::ddl::{
    column_default::parse_call_argument_sql, create_type::split_qualified_ident, DdlResult,
};

/// Parsed `CALL schema.name(args...)`. Same `RoutineCall` as column `DEFAULT fn(...)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallStatement {
    pub call: RoutineCall,
}

impl CallStatement {
    pub fn parse(sql: &str, default_namespace: &NamespaceId) -> DdlResult<Self> {
        let trimmed = sql.trim().trim_end_matches(';');
        if trimmed.len() < 4 || !trimmed[..4].eq_ignore_ascii_case("CALL") {
            return Err("Expected CALL statement".to_string());
        }
        let rest = trimmed[4..].trim_start();
        let (qual, after) = split_qualified_ident(rest)?;
        let after = after.trim_start();
        if !after.starts_with('(') {
            return Err("Expected argument list after procedure name".to_string());
        }
        let close = crate::ddl::create_type::matching_paren(after)
            .ok_or_else(|| "Unterminated CALL argument list".to_string())?;
        let leftover = strip_gui_limit_offset(after[close + 1..].trim())?;
        if !leftover.is_empty() {
            return Err(format!("Unexpected tokens after CALL arguments: '{leftover}'"));
        }
        let args_body = &after[1..close];
        let mut arguments = Vec::new();
        if !args_body.trim().is_empty() {
            for part in crate::ddl::create_type::split_top_level(args_body, ',') {
                arguments.push(parse_call_argument_sql(part.trim())?);
            }
        }
        let namespace_id = qual.namespace_or(default_namespace);
        Ok(Self {
            call: RoutineCall::new(
                RoutineId::from_parts(Some(&namespace_id), &qual.name),
                arguments,
            ),
        })
    }
}

/// GUI clients such as Tabularis wrap every statement with `LIMIT n OFFSET m`.
/// `CALL` returns at most one result row, so the paging clause is ignored.
fn strip_gui_limit_offset(rest: &str) -> DdlResult<&str> {
    let rest = consume_limit_or_offset(rest.trim(), "LIMIT")?;
    let rest = consume_limit_or_offset(rest.trim(), "OFFSET")?;
    let rest = consume_limit_or_offset(rest.trim(), "LIMIT")?;
    Ok(rest.trim())
}

fn consume_limit_or_offset<'a>(rest: &'a str, keyword: &str) -> DdlResult<&'a str> {
    if rest.len() < keyword.len() || !rest[..keyword.len()].eq_ignore_ascii_case(keyword) {
        return Ok(rest);
    }
    if rest.len() > keyword.len() {
        let next = rest.as_bytes()[keyword.len()];
        if next.is_ascii_alphabetic() || next == b'_' {
            return Ok(rest);
        }
    }
    let after_keyword = rest[keyword.len()..].trim_start();
    let digits = after_keyword
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(after_keyword.len());
    if digits == 0 {
        return Err(format!("Expected integer after {keyword}"));
    }
    Ok(&after_keyword[digits..])
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::{CallArgument, KalamDataType};

    use super::*;

    #[test]
    fn parse_call_positional_and_placeholder() {
        let ns = NamespaceId::new("app");
        let stmt = CallStatement::parse("CALL api.echo('hi', $1, 7, true, NULL)", &ns).unwrap();
        assert_eq!(stmt.call.routine_id.as_str(), "api.echo");
        assert_eq!(stmt.call.routine_id.namespace_id(), Some(NamespaceId::new("api")));
        assert_eq!(
            stmt.call.arguments,
            vec![
                CallArgument::text("hi"),
                CallArgument::Placeholder(1),
                CallArgument::bigint(7),
                CallArgument::boolean(true),
                CallArgument::Null,
            ]
        );
    }

    #[test]
    fn parse_call_uses_default_namespace() {
        let ns = NamespaceId::new("chat");
        let stmt = CallStatement::parse("CALL ping()", &ns).unwrap();
        assert_eq!(stmt.call.routine_id.as_str(), "chat.ping");
        assert!(stmt.call.arguments.is_empty());
    }

    #[test]
    fn parse_call_ignores_gui_limit_offset_suffix() {
        let ns = NamespaceId::new("app");
        let stmt = CallStatement::parse(
            r#"CALL "kobj_fnlint_mtyd04d2_22tq_0"."health"() LIMIT 501 OFFSET 0"#,
            &ns,
        )
        .expect("Tabularis appends LIMIT/OFFSET to CALL");
        assert_eq!(stmt.call.routine_id.as_str(), "kobj_fnlint_mtyd04d2_22tq_0.health");
        assert!(stmt.call.arguments.is_empty());

        CallStatement::parse("CALL api.echo('hi') LIMIT 501", &ns).unwrap();
        CallStatement::parse("CALL api.echo('hi') OFFSET 0", &ns).unwrap();
    }

    #[test]
    fn parse_call_rejects_non_limit_trailing_tokens() {
        let ns = NamespaceId::new("app");
        let err = CallStatement::parse("CALL api.echo('hi') RETURNING x", &ns).unwrap_err();
        assert!(err.contains("Unexpected tokens after CALL arguments"));
    }

    #[test]
    fn parse_call_accepts_catalog_typed_arguments() {
        let ns = NamespaceId::new("app");
        let stmt = CallStatement::parse(
            "CALL api.echo(CAST('{\"ok\":true}' AS JSON), UUID \
             '550e8400-e29b-41d4-a716-446655440000', X'ff00')",
            &ns,
        )
        .unwrap();
        assert_eq!(
            stmt.call.arguments,
            vec![
                CallArgument::json(serde_json::json!({"ok": true})),
                CallArgument::uuid("550e8400-e29b-41d4-a716-446655440000"),
                CallArgument::typed(KalamDataType::Bytes, serde_json::json!([255, 0]),),
            ]
        );
    }
}
