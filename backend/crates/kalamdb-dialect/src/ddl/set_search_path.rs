//! SET search_path session clause (canonicalizes with USE).

use kalamdb_commons::models::NamespaceId;

use crate::ddl::DdlResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetSearchPathStatement {
    pub schemas: Vec<NamespaceId>,
}

impl SetSearchPathStatement {
    pub fn parse(sql: &str) -> DdlResult<Self> {
        let trimmed = sql.trim().trim_end_matches(';').trim();
        let upper = trimmed.to_ascii_uppercase();
        if upper == "RESET SEARCH_PATH" {
            return Ok(Self {
                schemas: vec![NamespaceId::new("default")],
            });
        }

        let rest = strip_set_session_prefix(trimmed)?;
        let rest_upper = rest.to_ascii_uppercase();
        if !rest_upper.starts_with("SEARCH_PATH") {
            return Err("Expected SET search_path statement".to_string());
        }
        let rest = rest["SEARCH_PATH".len()..].trim_start();
        let rest = if rest.to_ascii_uppercase().starts_with("TO ") {
            rest[3..].trim_start()
        } else if rest.starts_with('=') {
            rest[1..].trim_start()
        } else {
            return Err("Expected TO or = after SET search_path".to_string());
        };
        if rest.to_ascii_uppercase() == "DEFAULT" {
            return Ok(Self {
                schemas: vec![NamespaceId::new("default")],
            });
        }
        let mut schemas = Vec::new();
        for part in rest.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let name = parse_search_path_entry(part)?;
            schemas.push(NamespaceId::new(name));
        }
        if schemas.is_empty() {
            return Err("SET search_path requires at least one schema".to_string());
        }
        Ok(Self { schemas })
    }
}

fn strip_set_session_prefix(sql: &str) -> DdlResult<&str> {
    let upper = sql.to_ascii_uppercase();
    if !upper.starts_with("SET") {
        return Err("Expected SET search_path statement".to_string());
    }
    let rest = sql[3..].trim_start();
    let rest_upper = rest.to_ascii_uppercase();
    if rest_upper.starts_with("SESSION ") {
        Ok(rest[7..].trim_start())
    } else if rest_upper.starts_with("LOCAL ") {
        Ok(rest[5..].trim_start())
    } else {
        Ok(rest)
    }
}

fn parse_search_path_entry(part: &str) -> DdlResult<String> {
    let trimmed = part.trim();
    if trimmed.eq_ignore_ascii_case("$user") || trimmed.eq_ignore_ascii_case("\"$user\"") {
        return Ok("$user".to_string());
    }
    if trimmed.eq_ignore_ascii_case("DEFAULT") {
        return Ok("default".to_string());
    }
    if trimmed.len() >= 2 {
        let bytes = trimmed.as_bytes();
        if bytes[0] == b'\'' && bytes[trimmed.len() - 1] == b'\'' {
            let name = &trimmed[1..trimmed.len() - 1];
            if name.is_empty() {
                return Err("SET search_path requires at least one schema".to_string());
            }
            return Ok(name.to_string());
        }
    }
    let (name, leftover) = crate::ddl::create_type::take_ident(trimmed)?;
    if !leftover.trim().is_empty() {
        return Err("Unexpected tokens in search_path list".to_string());
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_search_path_to() {
        let stmt = SetSearchPathStatement::parse("SET search_path TO chat").unwrap();
        assert_eq!(stmt.schemas[0].as_str(), "chat");
    }

    #[test]
    fn parse_search_path_eq() {
        let stmt = SetSearchPathStatement::parse("SET search_path = chat, public").unwrap();
        assert_eq!(stmt.schemas.len(), 2);
    }

    #[test]
    fn parse_search_path_session_and_reset() {
        let session = SetSearchPathStatement::parse("SET SESSION search_path TO chat").unwrap();
        assert_eq!(session.schemas[0].as_str(), "chat");

        let reset = SetSearchPathStatement::parse("RESET search_path").unwrap();
        assert_eq!(reset.schemas[0].as_str(), "default");

        let default = SetSearchPathStatement::parse("SET search_path TO DEFAULT").unwrap();
        assert_eq!(default.schemas[0].as_str(), "default");
    }

    #[test]
    fn parse_search_path_user_alias() {
        let stmt = SetSearchPathStatement::parse(r#"SET search_path TO "$user", public"#).unwrap();
        assert_eq!(stmt.schemas[0].as_str(), "$user");
        assert_eq!(stmt.schemas[1].as_str(), "public");
    }
}
