//! CREATE PROCEDURE / DROP PROCEDURE parsers.

use kalamdb_commons::models::{NamespaceId, RoutineId, RoutineSecurityMode};

use crate::ddl::{
    create_type::{
        matching_paren, parse_type_reference, split_qualified_ident, split_top_level, take_ident,
        take_type_attributes, TypeReference,
    },
    parsing::{parse_optional_comment, parse_sql_string_prefix, strip_keyword_ci, take_keyword_ci},
    DdlResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcedureParameter {
    pub name:     String,
    pub type_ref: TypeReference,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateProcedureStatement {
    pub routine_id:   RoutineId,
    pub namespace_id: NamespaceId,
    pub name:         String,
    pub or_replace:   bool,
    pub parameters:   Vec<ProcedureParameter>,
    pub return_type:  Option<TypeReference>,
    pub language:     Option<String>,
    pub security:     RoutineSecurityMode,
    pub body:         Option<String>,
    pub comment:      Option<String>,
}

impl CreateProcedureStatement {
    pub fn parse(sql: &str, default_namespace: &NamespaceId) -> DdlResult<Self> {
        let mut rest = sql.trim().trim_end_matches(';');
        let or_replace = take_keyword_ci(&mut rest, "CREATE OR REPLACE PROCEDURE");
        if !or_replace && !take_keyword_ci(&mut rest, "CREATE PROCEDURE") {
            return Err("Expected CREATE PROCEDURE statement".to_string());
        }
        let (qual, after) = split_qualified_ident(rest)?;
        let namespace_id = qual.namespace_or(default_namespace);
        let procedure_name = qual.name.to_ascii_lowercase();
        let after = after.trim_start();
        if !after.starts_with('(') {
            return Err("Expected parameter list after procedure name".to_string());
        }
        let close = matching_paren(after)
            .ok_or_else(|| "Unterminated procedure parameter list".to_string())?;
        let params_body = &after[1..close];
        let mut parameters = Vec::new();
        if !params_body.trim().is_empty() {
            for part in split_top_level(params_body, ',') {
                let part = part.trim();
                let (name, rest) = take_ident(part)?;
                let (mut type_ref, leftover) = parse_type_reference(rest.trim_start())?;
                take_type_attributes(&mut type_ref, leftover);
                parameters.push(ProcedureParameter { name, type_ref });
            }
        }

        let mut rest = after[close + 1..].trim_start();
        let mut return_type = None;
        let mut language = None;
        let mut security = RoutineSecurityMode::Invoker;
        let mut body = None;
        let mut comment = None;

        loop {
            if rest.is_empty() {
                break;
            }
            if take_keyword_ci(&mut rest, "RETURNS") {
                let after_returns = strip_keyword_ci(rest, "ROW TYPE").unwrap_or(rest);
                let (ty, leftover) = parse_type_reference(after_returns)?;
                return_type = Some(ty);
                rest = leftover.trim_start();
                continue;
            }
            if take_keyword_ci(&mut rest, "LANGUAGE") {
                let (lang, leftover) = take_ident(rest)?;
                language = Some(lang.to_ascii_uppercase());
                rest = leftover.trim_start();
                continue;
            }
            if take_keyword_ci(&mut rest, "SECURITY INVOKER") {
                security = RoutineSecurityMode::Invoker;
                continue;
            }
            if take_keyword_ci(&mut rest, "SECURITY DEFINER") {
                security = RoutineSecurityMode::Definer;
                continue;
            }
            let (parsed, leftover) = parse_optional_comment(rest)?;
            if parsed.is_some() {
                comment = parsed;
                rest = leftover.trim_start();
                continue;
            }
            if take_keyword_ci(&mut rest, "AS") {
                if looks_like_source_file_mapping(rest) {
                    return Err("CREATE PROCEDURE source-file mapping (AS 'path', 'export') is \
                                not supported; implement the procedure in the functions project \
                                or use LANGUAGE with an inline body"
                        .to_string());
                }
                let (parsed_body, leftover) = parse_procedure_body(rest)?;
                body = Some(parsed_body);
                rest = leftover.trim_start();
                continue;
            }
            return Err(format!("Unexpected procedure clause starting at '{rest}'"));
        }

        match (&language, &body) {
            (Some(_), None) => {
                return Err("LANGUAGE requires an AS $$ ... $$ (or string) body; omit LANGUAGE \
                            for project-backed procedures"
                    .to_string());
            },
            (None, Some(_)) => {
                return Err("inline procedure body requires a LANGUAGE clause (JAVASCRIPT or \
                            TYPESCRIPT)"
                    .to_string());
            },
            (None, None) | (Some(_), Some(_)) => {},
        }

        Ok(Self {
            routine_id: RoutineId::from_parts(Some(&namespace_id), &procedure_name),
            namespace_id,
            name: procedure_name,
            or_replace,
            parameters,
            return_type,
            language,
            security,
            body,
            comment,
        })
    }
}

fn looks_like_source_file_mapping(input: &str) -> bool {
    let input = input.trim();
    if !input.starts_with('\'') {
        return false;
    }
    let bytes = input.as_bytes();
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                i += 2;
                continue;
            }
            return input[i + 1..].trim_start().starts_with(',');
        }
        i += 1;
    }
    false
}

fn parse_procedure_body(input: &str) -> DdlResult<(String, &str)> {
    let input = input.trim();
    if input.starts_with("$$") {
        let rest = &input[2..];
        let end = rest
            .find("$$")
            .ok_or_else(|| "Unterminated dollar-quoted procedure body".to_string())?;
        return Ok((rest[..end].to_string(), &rest[end + 2..]));
    }
    if input.starts_with('\'') {
        return parse_sql_string_prefix(input);
    }
    Err("Procedure body must be dollar-quoted ($$ ... $$) or a string literal".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropProcedureStatement {
    pub routine_id: RoutineId,
    pub if_exists:  bool,
}

impl DropProcedureStatement {
    pub fn parse(sql: &str, default_namespace: &NamespaceId) -> DdlResult<Self> {
        let mut rest = sql.trim().trim_end_matches(';');
        if !take_keyword_ci(&mut rest, "DROP PROCEDURE") {
            return Err("Expected DROP PROCEDURE statement".to_string());
        }
        let if_exists = take_keyword_ci(&mut rest, "IF EXISTS");
        let (qual, _) = split_qualified_ident(rest)?;
        let namespace_id = qual.namespace_or(default_namespace);
        Ok(Self {
            routine_id: RoutineId::from_parts(Some(&namespace_id), &qual.name),
            if_exists,
        })
    }
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::models::NamespaceId;

    use super::*;

    #[test]
    fn parse_procedure_signature() {
        let stmt = CreateProcedureStatement::parse(
            "CREATE PROCEDURE chat.get_user(user_id TEXT NOT NULL)
             RETURNS ROW TYPE chat.users
             LANGUAGE SQL
             SECURITY INVOKER
             AS $$ SELECT * FROM chat.users WHERE id = user_id; $$",
            &NamespaceId::new("app"),
        )
        .unwrap();
        assert_eq!(stmt.routine_id.as_str(), "chat.get_user");
        assert_eq!(stmt.security, RoutineSecurityMode::Invoker);
        assert_eq!(stmt.language.as_deref(), Some("SQL"));
        assert!(stmt.body.unwrap().contains("SELECT"));
        assert_eq!(stmt.return_type.unwrap().name, "users");
    }

    #[test]
    fn parse_inline_javascript_dollar_quoted_body() {
        let stmt = CreateProcedureStatement::parse(
            "CREATE PROCEDURE api.health() RETURNS TEXT LANGUAGE JAVASCRIPT AS $$ return \"ok\"; \
             $$",
            &NamespaceId::new("app"),
        )
        .unwrap();
        assert_eq!(stmt.language.as_deref(), Some("JAVASCRIPT"));
        assert_eq!(stmt.body.as_deref(), Some(" return \"ok\"; "));
    }

    #[test]
    fn parse_inline_typescript_dollar_quoted_body() {
        let stmt = CreateProcedureStatement::parse(
            "CREATE PROCEDURE api.greeting(name TEXT) RETURNS TEXT LANGUAGE TYPESCRIPT AS $$\n    \
             return `Hello ${input.name}`;\n$$",
            &NamespaceId::new("app"),
        )
        .unwrap();
        assert_eq!(stmt.language.as_deref(), Some("TYPESCRIPT"));
        assert!(stmt.body.unwrap().contains("Hello"));
    }

    #[test]
    fn parse_bodyless_project_backed_procedure() {
        let stmt = CreateProcedureStatement::parse(
            "CREATE PROCEDURE api.create_order(request api.create_order_request)
             RETURNS api.create_order_result
             SECURITY DEFINER",
            &NamespaceId::new("app"),
        )
        .unwrap();
        assert_eq!(stmt.routine_id.as_str(), "api.create_order");
        assert_eq!(stmt.security, RoutineSecurityMode::Definer);
        assert!(stmt.language.is_none());
        assert!(stmt.body.is_none());
    }

    #[test]
    fn parse_procedure_comment_before_body() {
        let stmt = CreateProcedureStatement::parse(
            "CREATE PROCEDURE api.health() RETURNS TEXT COMMENT 'Liveness probe' LANGUAGE \
             JAVASCRIPT AS $$ return \"ok\"; $$",
            &NamespaceId::new("app"),
        )
        .unwrap();
        assert_eq!(stmt.comment.as_deref(), Some("Liveness probe"));
        assert_eq!(stmt.language.as_deref(), Some("JAVASCRIPT"));
    }

    #[test]
    fn parse_procedure_comment_after_body() {
        let stmt = CreateProcedureStatement::parse(
            "CREATE PROCEDURE api.health() RETURNS TEXT LANGUAGE JAVASCRIPT AS $$ return \"ok\"; \
             $$ COMMENT 'Liveness probe'",
            &NamespaceId::new("app"),
        )
        .unwrap();
        assert_eq!(stmt.comment.as_deref(), Some("Liveness probe"));
    }

    #[test]
    fn parse_bodyless_procedure_comment() {
        let stmt = CreateProcedureStatement::parse(
            "CREATE PROCEDURE api.create_order(request api.create_order_request)
             RETURNS api.create_order_result
             COMMENT 'Place an order'
             SECURITY DEFINER",
            &NamespaceId::new("app"),
        )
        .unwrap();
        assert_eq!(stmt.comment.as_deref(), Some("Place an order"));
        assert!(stmt.body.is_none());
    }

    #[test]
    fn reject_source_file_mapping() {
        let err = CreateProcedureStatement::parse(
            "CREATE PROCEDURE api.create_order(request api.create_order_request)
             RETURNS api.create_order_result
             AS 'src/api/orders.ts', 'createOrder'",
            &NamespaceId::new("app"),
        )
        .unwrap_err();
        assert!(err.contains("source-file mapping"), "{err}");
    }

    #[test]
    fn language_requires_body() {
        let err = CreateProcedureStatement::parse(
            "CREATE PROCEDURE api.health() RETURNS TEXT LANGUAGE JAVASCRIPT",
            &NamespaceId::new("app"),
        )
        .unwrap_err();
        assert!(err.contains("LANGUAGE requires"), "{err}");
    }

    #[test]
    fn body_requires_language() {
        let err = CreateProcedureStatement::parse(
            "CREATE PROCEDURE api.health() RETURNS TEXT AS $$ return 'ok'; $$",
            &NamespaceId::new("app"),
        )
        .unwrap_err();
        assert!(err.contains("LANGUAGE"), "{err}");
    }
}
