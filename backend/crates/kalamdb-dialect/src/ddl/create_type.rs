//! CREATE TYPE / DROP TYPE parsers for Functions V1 contract types.

use kalamdb_commons::{
    models::{NamespaceId, TypeId},
    KalamDataType,
};

use crate::ddl::DdlResult;

/// SQL type reference used in composite fields and signatures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeReference {
    pub namespace_id: Option<NamespaceId>,
    pub name:         String,
    /// Parsed builtin, including `DECIMAL(p,s)` / `EMBEDDING(n)` parameters.
    pub data_type:    Option<KalamDataType>,
    pub is_array:     bool,
    pub not_null:     bool,
    pub nonempty:     bool,
}

impl TypeReference {
    pub fn type_id(&self) -> TypeId {
        TypeId::from_parts(self.namespace_id.as_ref(), &self.name)
    }

    /// Built-in catalog type, if this reference is not a named `CREATE TYPE`.
    pub fn builtin_data_type(&self) -> Option<KalamDataType> {
        self.data_type
    }

    /// Named type identity, or `None` for builtins such as `TEXT` / `UUID`.
    pub fn resolved_type_id(&self, current: &NamespaceId) -> Option<TypeId> {
        if self.builtin_data_type().is_some() {
            None
        } else {
            Some(TypeId::from_parts(
                Some(self.namespace_id.as_ref().unwrap_or(current)),
                &self.name,
            ))
        }
    }

    pub fn resolved_type_name(&self, current: &NamespaceId) -> String {
        if let Some(data_type) = self.builtin_data_type() {
            data_type.sql_name()
        } else {
            self.resolved_type_id(current).expect("named types have a type id").to_string()
        }
    }
}

/// One field of `CREATE TYPE name AS (...)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositeTypeField {
    pub name:     String,
    pub type_ref: TypeReference,
}

/// Body of CREATE TYPE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateTypeBody {
    Composite {
        fields: Vec<CompositeTypeField>,
    },
    Enum {
        labels: Vec<String>,
    },
    FromTable {
        table_namespace_id: Option<NamespaceId>,
        table_name:         String,
    },
}

/// CREATE TYPE statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTypeStatement {
    pub type_id:       TypeId,
    pub namespace_id:  NamespaceId,
    pub name:          String,
    pub if_not_exists: bool,
    pub body:          CreateTypeBody,
    pub comment:       Option<String>,
}

const UNION_RESERVED: &str = "UNION types are reserved and not supported in V1";
const INTERFACE_RESERVED: &str = "INTERFACE types are reserved and not supported in V1";

impl CreateTypeStatement {
    pub fn parse(sql: &str, default_namespace: &NamespaceId) -> DdlResult<Self> {
        let trimmed = sql.trim().trim_end_matches(';');
        let upper = trimmed.to_ascii_uppercase();
        if !upper.starts_with("CREATE TYPE") {
            return Err("Expected CREATE TYPE statement".to_string());
        }
        let mut rest = trimmed["CREATE TYPE".len()..].trim_start();
        let if_not_exists = starts_with_ignore_case(rest, "IF NOT EXISTS");
        if if_not_exists {
            rest = rest["IF NOT EXISTS".len()..].trim_start();
        }

        let (qual, after_name) = split_qualified_ident(rest)?;
        let (namespace_id, name) = (qual.namespace_or(default_namespace), qual.name);
        let rest = after_name.trim_start();
        let rest_upper = rest.to_ascii_uppercase();

        if rest_upper.starts_with("AS UNION") {
            return Err(UNION_RESERVED.to_string());
        }
        if rest_upper.starts_with("AS INTERFACE") {
            return Err(INTERFACE_RESERVED.to_string());
        }

        let (body, after_body) = if rest_upper.starts_with("AS ENUM") {
            let after = rest["AS ENUM".len()..].trim_start();
            let (labels, leftover) = parse_enum_labels(after)?;
            (CreateTypeBody::Enum { labels }, leftover)
        } else if rest_upper.starts_with("FROM TABLE") {
            let after = rest["FROM TABLE".len()..].trim_start();
            let (table, leftover) = split_qualified_ident(after)?;
            (
                CreateTypeBody::FromTable {
                    table_namespace_id: table.namespace_id,
                    table_name:         table.name,
                },
                leftover,
            )
        } else if rest_upper.starts_with("AS") {
            let after = rest[2..].trim_start();
            if !after.starts_with('(') {
                return Err("Expected AS ( ... ) composite definition".to_string());
            }
            let (fields, leftover) = parse_composite_fields(after)?;
            (CreateTypeBody::Composite { fields }, leftover)
        } else {
            return Err("Expected AS (...), AS ENUM (...), or FROM TABLE".to_string());
        };

        let (comment, leftover) = parse_optional_comment(after_body)?;
        if !leftover.trim().is_empty() {
            return Err(format!("Unexpected tokens after type definition '{}'", leftover.trim()));
        }

        Ok(Self {
            type_id: TypeId::from_parts(Some(&namespace_id), &name),
            namespace_id,
            name,
            if_not_exists,
            body,
            comment,
        })
    }
}

/// DROP TYPE [IF EXISTS] name [CASCADE|RESTRICT]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropTypeStatement {
    pub type_id:   TypeId,
    pub if_exists: bool,
    pub cascade:   bool,
}

impl DropTypeStatement {
    pub fn parse(sql: &str, default_namespace: &NamespaceId) -> DdlResult<Self> {
        let trimmed = sql.trim().trim_end_matches(';');
        let upper = trimmed.to_ascii_uppercase();
        if !upper.starts_with("DROP TYPE") {
            return Err("Expected DROP TYPE statement".to_string());
        }
        let mut rest = trimmed["DROP TYPE".len()..].trim_start();
        let if_exists = starts_with_ignore_case(rest, "IF EXISTS");
        if if_exists {
            rest = rest["IF EXISTS".len()..].trim_start();
        }
        let (qual, after) = split_qualified_ident(rest)?;
        let leftover = after.trim();
        let leftover_upper = leftover.to_ascii_uppercase();
        let cascade = leftover_upper == "CASCADE";
        if !leftover.is_empty() && leftover_upper != "CASCADE" && leftover_upper != "RESTRICT" {
            return Err(
                "Expected CASCADE, RESTRICT, or end of statement after DROP TYPE".to_string()
            );
        }
        let namespace_id = qual.namespace_or(default_namespace);
        Ok(Self {
            type_id: TypeId::from_parts(Some(&namespace_id), &qual.name),
            if_exists,
            cascade,
        })
    }
}

#[derive(Debug)]
pub(crate) struct QualifiedIdent {
    pub namespace_id: Option<NamespaceId>,
    pub name:         String,
}

impl QualifiedIdent {
    pub(crate) fn namespace_or(&self, default: &NamespaceId) -> NamespaceId {
        self.namespace_id.clone().unwrap_or_else(|| default.clone())
    }
}

fn starts_with_ignore_case(haystack: &str, prefix: &str) -> bool {
    haystack.len() >= prefix.len() && haystack[..prefix.len()].eq_ignore_ascii_case(prefix)
}

pub(crate) fn split_qualified_ident(input: &str) -> DdlResult<(QualifiedIdent, &str)> {
    let (first, rest) = take_ident(input)?;
    let rest = rest.trim_start();
    if rest.starts_with('.') {
        let (second, rest2) = take_ident(rest[1..].trim_start())?;
        Ok((
            QualifiedIdent {
                namespace_id: Some(parse_namespace_id(&first)?),
                name:         second,
            },
            rest2,
        ))
    } else {
        Ok((
            QualifiedIdent {
                namespace_id: None,
                name:         first,
            },
            rest,
        ))
    }
}

pub(crate) fn take_ident(input: &str) -> DdlResult<(String, &str)> {
    let input = input.trim_start();
    if input.is_empty() {
        return Err("Expected identifier".to_string());
    }
    if input.starts_with('"') {
        let end = input[1..]
            .find('"')
            .ok_or_else(|| "Unterminated quoted identifier".to_string())?;
        let name = input[1..1 + end].to_string();
        return Ok((name, &input[2 + end..]));
    }
    let mut chars = input.char_indices();
    let Some((_, first)) = chars.next() else {
        return Err("Expected identifier".to_string());
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(format!("Invalid identifier starting with '{first}'"));
    }
    let mut end = 1;
    for (idx, ch) in chars {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    Ok((input[..end].to_string(), &input[end..]))
}

fn parse_enum_labels(input: &str) -> DdlResult<(Vec<String>, &str)> {
    let (body, leftover) = split_parens(input)?;
    if body.trim().is_empty() {
        return Err("ENUM type requires at least one label".to_string());
    }
    let mut labels = Vec::new();
    for part in split_top_level(body, ',') {
        labels.push(parse_sql_string(part.trim())?);
    }
    Ok((labels, leftover))
}

fn parse_composite_fields(input: &str) -> DdlResult<(Vec<CompositeTypeField>, &str)> {
    let (body, leftover) = split_parens(input)?;
    if body.trim().is_empty() {
        return Err("composite type requires at least one field".to_string());
    }
    let mut fields = Vec::new();
    for part in split_top_level(body, ',') {
        fields.push(parse_field(part.trim())?);
    }
    Ok((fields, leftover))
}

fn parse_field(input: &str) -> DdlResult<CompositeTypeField> {
    let (name, rest) = take_ident(input)?;
    let rest = rest.trim_start();
    if rest.is_empty() {
        return Err(format!("Missing type for field '{name}'"));
    }
    let (mut type_ref, rest) = parse_type_reference(rest)?;
    let mut leftover = rest.trim_start().to_ascii_uppercase();
    leftover = leftover.replace("NOT  NULL", "NOT NULL");
    let tokens: Vec<&str> = leftover.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "NOT" if tokens.get(i + 1) == Some(&"NULL") => {
                type_ref.not_null = true;
                i += 2;
            },
            "NULL" => {
                type_ref.not_null = false;
                i += 1;
            },
            "NONEMPTY" => {
                type_ref.nonempty = true;
                i += 1;
            },
            other => {
                return Err(format!("Unexpected field attribute '{other}'"));
            },
        }
    }
    if type_ref.nonempty && !type_ref.not_null {
        return Err("NONEMPTY requires NOT NULL".to_string());
    }
    Ok(CompositeTypeField { name, type_ref })
}

pub(crate) fn parse_type_reference(input: &str) -> DdlResult<(TypeReference, &str)> {
    let (qual, mut rest) = split_qualified_ident(input)?;
    rest = rest.trim_start();
    let mut params = None;
    if rest.starts_with('(') {
        let close =
            matching_paren(rest).ok_or_else(|| "Unterminated type parameter list".to_string())?;
        params = Some(&rest[1..close]);
        rest = rest[close + 1..].trim_start();
    }
    let mut is_array = false;
    if rest.starts_with("[]") {
        is_array = true;
        rest = rest[2..].trim_start();
    }
    let data_type = if qual.namespace_id.is_none() {
        match params {
            Some(params) if !params.is_empty() => {
                KalamDataType::from_sql_name(&format!("{}({params})", qual.name))
            },
            _ => KalamDataType::from_sql_name(&qual.name),
        }
    } else {
        None
    };
    Ok((
        TypeReference {
            namespace_id: qual.namespace_id,
            name: qual.name,
            data_type,
            is_array,
            not_null: false,
            nonempty: false,
        },
        rest,
    ))
}

fn parse_namespace_id(name: &str) -> DdlResult<NamespaceId> {
    NamespaceId::try_parse_reference(name).map_err(|error| error.to_string())
}

fn split_parens(input: &str) -> DdlResult<(&str, &str)> {
    let input = input.trim();
    if !input.starts_with('(') {
        return Err("Expected '('".to_string());
    }
    let close = matching_paren(input).ok_or_else(|| "Unterminated '('".to_string())?;
    Ok((&input[1..close], &input[close + 1..]))
}

/// Parse `COMMENT 'text'` or `COMMENT = 'text'` if present.
pub(crate) fn parse_optional_comment(input: &str) -> DdlResult<(Option<String>, &str)> {
    let rest = input.trim_start();
    if rest.len() < 7 || !rest[..7].eq_ignore_ascii_case("COMMENT") {
        return Ok((None, rest));
    }
    let after_kw = &rest[7..];
    if after_kw.starts_with(|ch: char| ch.is_ascii_alphanumeric() || ch == '_') {
        return Ok((None, rest));
    }
    let mut after = after_kw.trim_start();
    if after.starts_with('=') {
        after = after[1..].trim_start();
    }
    let (value, leftover) = parse_sql_string_prefix(after)?;
    Ok((Some(value), leftover))
}

pub(crate) fn parse_sql_string_prefix(input: &str) -> DdlResult<(String, &str)> {
    let input = input.trim_start();
    let mut chars = input.char_indices();
    let Some((_, '\'')) = chars.next() else {
        return Err(format!("Expected string literal, got '{input}'"));
    };
    let mut out = String::new();
    while let Some((idx, ch)) = chars.next() {
        if ch == '\'' {
            if chars.as_str().starts_with('\'') {
                chars.next();
                out.push('\'');
                continue;
            }
            return Ok((out, &input[idx + 1..]));
        }
        out.push(ch);
    }
    Err("Unterminated string literal".to_string())
}

pub(crate) fn matching_paren(input: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut chars = input.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if in_str {
            if ch == '\'' {
                if chars.peek().map(|(_, n)| *n) == Some('\'') {
                    chars.next();
                } else {
                    in_str = false;
                }
            }
            continue;
        }
        match ch {
            '\'' => in_str = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            },
            _ => {},
        }
    }
    None
}

pub(crate) fn split_top_level(input: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut chars = input.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if in_str {
            if ch == '\'' {
                if chars.peek().map(|(_, n)| *n) == Some('\'') {
                    chars.next();
                } else {
                    in_str = false;
                }
            }
            continue;
        }
        match ch {
            '\'' => in_str = true,
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c == sep && depth == 0 => {
                parts.push(&input[start..idx]);
                start = idx + c.len_utf8();
            },
            _ => {},
        }
    }
    parts.push(&input[start..]);
    parts
}

pub(crate) fn parse_sql_string(input: &str) -> DdlResult<String> {
    let input = input.trim();
    if input.len() >= 2 && input.starts_with('\'') && input.ends_with('\'') {
        return Ok(input[1..input.len() - 1].replace("''", "'"));
    }
    Err(format!("Expected string literal, got '{input}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns() -> NamespaceId {
        NamespaceId::new("app")
    }

    #[test]
    fn parse_composite_type() {
        let stmt = CreateTypeStatement::parse(
            "CREATE TYPE chat.recipient_result AS (
                user_id TEXT NOT NULL,
                delivered BOOLEAN NOT NULL
            )",
            &ns(),
        )
        .unwrap();
        assert_eq!(stmt.type_id.as_str(), "chat.recipient_result");
        match stmt.body {
            CreateTypeBody::Composite { fields } => {
                assert_eq!(fields.len(), 2);
                assert!(fields[0].type_ref.not_null);
                assert!(!fields[0].type_ref.is_array);
            },
            _ => panic!("expected composite"),
        }
    }

    #[test]
    fn parse_enum_type() {
        let stmt = CreateTypeStatement::parse(
            "CREATE TYPE chat.message_status AS ENUM ('sent', 'delivered', 'read')",
            &ns(),
        )
        .unwrap();
        match stmt.body {
            CreateTypeBody::Enum { labels } => {
                assert_eq!(labels, vec!["sent", "delivered", "read"]);
            },
            _ => panic!("expected enum"),
        }
    }

    #[test]
    fn parse_from_table() {
        let stmt = CreateTypeStatement::parse("CREATE TYPE chat.user FROM TABLE chat.users", &ns())
            .unwrap();
        match stmt.body {
            CreateTypeBody::FromTable {
                table_namespace_id,
                table_name,
            } => {
                assert_eq!(table_namespace_id.as_ref().map(NamespaceId::as_str), Some("chat"));
                assert_eq!(table_name, "users");
            },
            _ => panic!("expected from table"),
        }
    }

    #[test]
    fn reject_union() {
        let err = CreateTypeStatement::parse("CREATE TYPE t AS UNION (a INT)", &ns()).unwrap_err();
        assert_eq!(err, UNION_RESERVED);
    }

    #[test]
    fn reject_interface() {
        let err = CreateTypeStatement::parse("CREATE TYPE t AS INTERFACE { id TEXT }", &ns())
            .unwrap_err();
        assert_eq!(err, INTERFACE_RESERVED);
    }

    #[test]
    fn nonempty_requires_not_null() {
        let err =
            CreateTypeStatement::parse("CREATE TYPE t AS (note TEXT NONEMPTY)", &ns()).unwrap_err();
        assert!(err.contains("NONEMPTY"));
    }

    #[test]
    fn array_and_nonempty() {
        let stmt = CreateTypeStatement::parse(
            "CREATE TYPE chat.send_message_result AS (
                recipients chat.recipient_result[] NOT NULL NONEMPTY
            )",
            &ns(),
        )
        .unwrap();
        match stmt.body {
            CreateTypeBody::Composite { fields } => {
                assert!(fields[0].type_ref.is_array);
                assert!(fields[0].type_ref.not_null);
                assert!(fields[0].type_ref.nonempty);
                assert_eq!(
                    fields[0].type_ref.namespace_id.as_ref().map(NamespaceId::as_str),
                    Some("chat")
                );
            },
            _ => panic!("expected composite"),
        }
    }

    #[test]
    fn parse_composite_type_comment() {
        let stmt = CreateTypeStatement::parse(
            "CREATE TYPE chat.address AS (city TEXT NOT NULL) COMMENT 'Postal address'",
            &ns(),
        )
        .unwrap();
        assert_eq!(stmt.comment.as_deref(), Some("Postal address"));
    }

    #[test]
    fn parse_enum_comment_with_equals() {
        let stmt = CreateTypeStatement::parse(
            "CREATE TYPE chat.status AS ENUM ('active', 'blocked') COMMENT = 'Lifecycle state'",
            &ns(),
        )
        .unwrap();
        assert_eq!(stmt.comment.as_deref(), Some("Lifecycle state"));
    }

    #[test]
    fn parse_from_table_comment() {
        let stmt = CreateTypeStatement::parse(
            "CREATE TYPE chat.user FROM TABLE chat.users COMMENT 'Singular row alias'",
            &ns(),
        )
        .unwrap();
        assert_eq!(stmt.comment.as_deref(), Some("Singular row alias"));
    }

    #[test]
    fn parameterized_builtins_keep_kalam_data_type() {
        let stmt = CreateTypeStatement::parse(
            "CREATE TYPE chat.payload AS (
                amount DECIMAL(10, 2) NOT NULL,
                vector EMBEDDING(384) NOT NULL
            )",
            &ns(),
        )
        .unwrap();
        match stmt.body {
            CreateTypeBody::Composite { fields } => {
                assert_eq!(
                    fields[0].type_ref.data_type,
                    Some(KalamDataType::Decimal {
                        precision: 10,
                        scale:     2,
                    })
                );
                assert_eq!(fields[1].type_ref.data_type, Some(KalamDataType::Embedding(384)));
            },
            _ => panic!("expected composite"),
        }
    }
}
