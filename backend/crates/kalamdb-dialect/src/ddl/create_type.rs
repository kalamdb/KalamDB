//! CREATE TYPE / DROP TYPE parsers for PostgreSQL 18 composite and enum types.

use std::collections::HashSet;

use kalamdb_commons::{models::SchemaId, KalamDataType};

use crate::ddl::{
    create_domain::take_collation_name,
    parsing::{parse_optional_comment, parse_sql_string, take_keyword_ci},
    DdlResult,
};

/// Maximum PostgreSQL identifier / enum label length in UTF-8 bytes (`NAMEDATALEN - 1`).
pub(crate) const PG_NAMEDATALEN_MAX_BYTES: usize = 63;

/// Maximum enum label length in UTF-8 bytes (`NAMEDATALEN - 1`).
pub(crate) const ENUM_LABEL_MAX_LEN: usize = PG_NAMEDATALEN_MAX_BYTES;

/// SQL type reference used in composite fields, domains, and routine signatures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeReference {
    pub namespace_id:       Option<SchemaId>,
    pub name:               String,
    /// Parsed builtin, including `DECIMAL(p,s)` / `EMBEDDING(n)` parameters.
    pub data_type:          Option<KalamDataType>,
    /// Parenthesized modifier text from the AST, such as `12,2` or `10`.
    ///
    /// Composite attributes and domains keep this. Function and procedure
    /// signatures discard it when computing the effective type.
    pub syntactic_modifier: Option<String>,
    pub is_array:           bool,
    pub not_null:           bool,
    pub nonempty:           bool,
}

impl TypeReference {
    /// Built-in catalog type, if this reference is not a named `CREATE TYPE`.
    pub fn builtin_data_type(&self) -> Option<KalamDataType> {
        self.data_type
    }

    /// Schema + unqualified name for live catalog lookup (not the TypeId).
    pub fn resolved_name(&self, current: &SchemaId) -> Option<(SchemaId, String)> {
        if self.builtin_data_type().is_some() {
            None
        } else {
            Some((self.namespace_id.clone().unwrap_or_else(|| current.clone()), self.name.clone()))
        }
    }

    /// Effective function or procedure argument type.
    ///
    /// PostgreSQL `CREATE FUNCTION` discards parenthesized type modifiers, so
    /// `numeric(12,2)` and `numeric(4,1)` are the same signature type. The
    /// parsed [`Self::syntactic_modifier`] stays on the original AST. Bare
    /// `numeric` still uses the existing decimal128 default; that is not
    /// unconstrained numeric.
    pub fn function_signature_type(&self) -> Self {
        let data_type = if self.data_type.is_some() && self.namespace_id.is_none() {
            KalamDataType::from_sql_name(&self.name).or(self.data_type)
        } else {
            None
        };
        Self {
            namespace_id: self.namespace_id.clone(),
            name: self.name.clone(),
            data_type,
            syntactic_modifier: None,
            is_array: self.is_array,
            not_null: self.not_null,
            nonempty: self.nonempty,
        }
    }

    /// SQL display name. Named types use `schema.name`, never a synthesized TypeId.
    pub fn resolved_type_name(&self, current: &SchemaId) -> String {
        if let Some(data_type) = self.builtin_data_type() {
            if data_type == KalamDataType::Json && self.name.eq_ignore_ascii_case("jsonb") {
                return "JSONB".to_string();
            }
            if data_type == KalamDataType::Timestamp
                && (self.name.eq_ignore_ascii_case("timestamptz")
                    || self.name.eq_ignore_ascii_case("timestamp with time zone"))
            {
                return "TIMESTAMPTZ".to_string();
            }
            data_type.sql_name()
        } else {
            let namespace_id = self.namespace_id.as_ref().unwrap_or(current);
            format!("{}.{}", namespace_id.as_str(), self.name)
        }
    }
}

/// One field of `CREATE TYPE name AS (...)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositeTypeField {
    pub name:      String,
    pub type_ref:  TypeReference,
    pub collation: Option<String>,
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
        table_namespace_id: Option<SchemaId>,
        table_name:         String,
    },
}

/// CREATE TYPE statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTypeStatement {
    pub namespace_id:  SchemaId,
    pub name:          String,
    pub if_not_exists: bool,
    pub body:          CreateTypeBody,
    pub comment:       Option<String>,
}

const UNION_RESERVED: &str = "UNION types are reserved and not supported in V1";
const INTERFACE_RESERVED: &str = "INTERFACE types are reserved and not supported in V1";

impl CreateTypeStatement {
    pub fn parse(sql: &str, default_namespace: &SchemaId) -> DdlResult<Self> {
        let mut rest = sql.trim().trim_end_matches(';');
        if take_keyword_ci(&mut rest, "CREATE OR REPLACE TYPE") {
            return Err("CREATE OR REPLACE TYPE is not supported in PostgreSQL 18; use DROP TYPE \
                        or ALTER TYPE instead"
                .to_string());
        }
        if !take_keyword_ci(&mut rest, "CREATE TYPE") {
            return Err("Expected CREATE TYPE statement".to_string());
        }
        if take_keyword_ci(&mut rest, "IF NOT EXISTS") {
            return Err("CREATE TYPE IF NOT EXISTS is not supported in PostgreSQL 18; check \
                        pg_type or use a DO block instead"
                .to_string());
        }

        let (qual, after_name) = split_qualified_ident(rest)?;
        let (namespace_id, name) = (qual.namespace_or(default_namespace), qual.name);
        if after_name.trim().is_empty() {
            return Err("Shell type 'CREATE TYPE name' without AS (...) or AS ENUM (...) is not \
                        supported"
                .to_string());
        }
        let (body, after_body) = parse_type_body(after_name.trim_start())?;

        let (comment, leftover) = parse_optional_comment(after_body)?;
        if comment.is_some() {
            return Err(format!(
                "Inline COMMENT on CREATE TYPE is not supported in PostgreSQL 18; use COMMENT ON \
                 TYPE {namespace_id}.{name} IS '...' instead"
            ));
        }
        if !leftover.trim().is_empty() {
            return Err(format!("Unexpected tokens after type definition '{}'", leftover.trim()));
        }

        Ok(Self {
            namespace_id,
            name,
            if_not_exists: false,
            body,
            comment: None,
        })
    }
}

/// DROP TYPE [IF EXISTS] name [, ...] [CASCADE|RESTRICT]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropTypeStatement {
    pub namespace_id: SchemaId,
    pub name:         String,
    pub targets:      Vec<(SchemaId, String)>,
    pub if_exists:    bool,
    pub cascade:      bool,
}

impl DropTypeStatement {
    pub fn parse(sql: &str, default_namespace: &SchemaId) -> DdlResult<Self> {
        let mut rest = sql.trim().trim_end_matches(';');
        if !take_keyword_ci(&mut rest, "DROP TYPE") {
            return Err("Expected DROP TYPE statement".to_string());
        }
        let if_exists = take_keyword_ci(&mut rest, "IF EXISTS");
        let rest = rest.trim_start();
        if rest.is_empty() {
            return Err("Expected type name after DROP TYPE".to_string());
        }

        let parts = split_top_level(rest, ',');
        let mut targets = Vec::with_capacity(parts.len());
        let mut cascade = false;

        for (index, part) in parts.iter().enumerate() {
            let (qual, after) = split_qualified_ident(part.trim())?;
            let mut leftover = after.trim_start();
            if index + 1 == parts.len() {
                if take_keyword_ci(&mut leftover, "CASCADE") {
                    cascade = true;
                } else {
                    let _ = take_keyword_ci(&mut leftover, "RESTRICT");
                }
            }
            if !leftover.trim().is_empty() {
                return Err(
                    "Expected CASCADE, RESTRICT, or end of statement after DROP TYPE".to_string()
                );
            }
            targets.push((qual.namespace_or(default_namespace), qual.name));
        }

        let (namespace_id, name) = targets
            .first()
            .cloned()
            .ok_or_else(|| "Expected type name after DROP TYPE".to_string())?;

        Ok(Self {
            namespace_id,
            name,
            targets,
            if_exists,
            cascade,
        })
    }
}

fn parse_type_body(mut rest: &str) -> DdlResult<(CreateTypeBody, &str)> {
    if take_keyword_ci(&mut rest, "AS UNION") {
        return Err(UNION_RESERVED.to_string());
    }
    if take_keyword_ci(&mut rest, "AS INTERFACE") {
        return Err(INTERFACE_RESERVED.to_string());
    }
    if take_keyword_ci(&mut rest, "AS RANGE") {
        return Err("CREATE TYPE ... AS RANGE is not supported in this release".to_string());
    }
    if take_keyword_ci(&mut rest, "AS ENUM") {
        let (labels, leftover) = parse_enum_labels(rest)?;
        return Ok((CreateTypeBody::Enum { labels }, leftover));
    }
    if take_keyword_ci(&mut rest, "FROM TABLE") {
        return Err("CREATE TYPE ... FROM TABLE is not supported in PostgreSQL 18; reference the \
                    table's implicit composite row type directly"
            .to_string());
    }
    if take_keyword_ci(&mut rest, "AS") {
        if !rest.starts_with('(') {
            return Err("Expected AS ( ... ) composite definition".to_string());
        }
        let (fields, leftover) = parse_composite_fields(rest)?;
        return Ok((CreateTypeBody::Composite { fields }, leftover));
    }
    if rest.starts_with('(') {
        return Err("Base type 'CREATE TYPE name (INPUT = ..., OUTPUT = ...)' is not supported; \
                    use CREATE TYPE name AS (...) for composite types"
            .to_string());
    }
    Err("Expected AS (...) or AS ENUM (...)".to_string())
}

#[derive(Debug, Clone)]
pub(crate) struct QualifiedIdent {
    pub namespace_id: Option<SchemaId>,
    pub name:         String,
    pub name_quoted:  bool,
}

impl QualifiedIdent {
    pub(crate) fn namespace_or(&self, default: &SchemaId) -> SchemaId {
        self.namespace_id.clone().unwrap_or_else(|| default.clone())
    }
}

pub(crate) fn split_qualified_ident(input: &str) -> DdlResult<(QualifiedIdent, &str)> {
    let (first, first_quoted, rest) = take_ident_with_quote(input)?;
    let rest_trimmed = crate::ddl::parsing::skip_ws_and_comments(rest);
    if let Some(after_dot) = rest_trimmed.strip_prefix('.') {
        let (second, second_quoted, rest2) =
            take_ident_with_quote(crate::ddl::parsing::skip_ws_and_comments(after_dot))?;
        Ok((
            QualifiedIdent {
                namespace_id: Some(parse_namespace_id(&first, first_quoted)?),
                name:         second,
                name_quoted:  second_quoted,
            },
            rest2,
        ))
    } else {
        Ok((
            QualifiedIdent {
                namespace_id: None,
                name:         first,
                name_quoted:  first_quoted,
            },
            rest,
        ))
    }
}

pub(crate) fn validate_identifier_bytes(ident: &str, label: &str) -> DdlResult<()> {
    if ident.is_empty() {
        return Err(format!("{label} cannot be empty"));
    }
    if ident.len() > PG_NAMEDATALEN_MAX_BYTES {
        return Err(format!(
            "{label} exceeds {PG_NAMEDATALEN_MAX_BYTES} bytes (got {} bytes)",
            ident.len()
        ));
    }
    if ident.contains('\0') {
        return Err(format!("{label} cannot contain NUL bytes"));
    }
    Ok(())
}

pub(crate) fn take_ident(input: &str) -> DdlResult<(String, &str)> {
    let (name, _quoted, rest) = take_ident_with_quote(input)?;
    Ok((name, rest))
}

pub(crate) fn take_ident_with_quote(input: &str) -> DdlResult<(String, bool, &str)> {
    let input = crate::ddl::parsing::skip_ws_and_comments(input);
    if input.is_empty() {
        return Err("Expected identifier".to_string());
    }
    if let Some(after_open) = input.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = after_open.char_indices().peekable();
        while let Some((idx, ch)) = chars.next() {
            if ch == '"' {
                if chars.peek().map(|(_, next)| *next) == Some('"') {
                    chars.next();
                    out.push('"');
                } else {
                    validate_identifier_bytes(&out, "Identifier")?;
                    return Ok((out, true, &after_open[idx + 1..]));
                }
            } else {
                out.push(ch);
            }
        }
        return Err("Unterminated quoted identifier".to_string());
    }
    let mut chars = input.char_indices();
    let Some((_, first)) = chars.next() else {
        return Err("Expected identifier".to_string());
    };
    if !first.is_alphabetic() && first != '_' {
        return Err(format!("Invalid identifier starting with '{first}'"));
    }
    let mut end = first.len_utf8();
    for (idx, ch) in chars {
        if ch.is_alphanumeric() || ch == '_' || ch == '$' {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    let folded = input[..end].to_ascii_lowercase();
    validate_identifier_bytes(&folded, "Identifier")?;
    Ok((folded, false, &input[end..]))
}

pub(crate) fn validate_enum_label(label: &str) -> DdlResult<()> {
    if label.contains('\0') {
        return Err("ENUM label cannot contain NUL bytes".to_string());
    }
    if label.len() > ENUM_LABEL_MAX_LEN {
        return Err(format!(
            "ENUM label exceeds {ENUM_LABEL_MAX_LEN} bytes (got {} bytes)",
            label.len()
        ));
    }
    Ok(())
}

fn parse_enum_labels(input: &str) -> DdlResult<(Vec<String>, &str)> {
    let (body, leftover) = split_parens(input)?;
    if body.trim().is_empty() {
        return Ok((Vec::new(), leftover));
    }
    let mut labels = Vec::new();
    let mut seen = HashSet::new();
    for part in split_top_level(body, ',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            return Err("Unexpected empty ENUM label item in list".to_string());
        }
        let label = parse_sql_string(trimmed)?;
        validate_enum_label(&label)?;
        if !seen.insert(label.clone()) {
            return Err(format!("duplicate ENUM label '{label}'"));
        }
        labels.push(label);
    }
    Ok((labels, leftover))
}

fn parse_composite_fields(input: &str) -> DdlResult<(Vec<CompositeTypeField>, &str)> {
    let (body, leftover) = split_parens(input)?;
    if body.trim().is_empty() {
        return Ok((Vec::new(), leftover));
    }
    let mut fields = Vec::new();
    let mut seen = HashSet::new();
    for part in split_top_level(body, ',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            return Err("Unexpected trailing or empty attribute in composite type".to_string());
        }
        let field = parse_field(trimmed)?;
        if !seen.insert(field.name.clone()) {
            return Err(format!("duplicate composite field '{}'", field.name));
        }
        fields.push(field);
    }
    Ok((fields, leftover))
}

pub(crate) fn parse_field(input: &str) -> DdlResult<CompositeTypeField> {
    let (name, rest) = take_ident(input)?;
    let rest = rest.trim_start();
    if rest.is_empty() {
        return Err(format!("Missing type for field '{name}'"));
    }
    let (type_ref, rest) = parse_type_reference(rest)?;
    let mut rest = rest.trim_start();

    let mut collation = None;
    if take_keyword_ci(&mut rest, "COLLATE") {
        let (coll, next) = take_collation_name(rest)?;
        collation = Some(coll);
        rest = next.trim_start();
    }

    if !rest.is_empty() {
        let mut probe = rest;
        if take_keyword_ci(&mut probe, "NOT NULL")
            || take_keyword_ci(&mut probe, "NULL")
            || take_keyword_ci(&mut probe, "NONEMPTY")
            || take_keyword_ci(&mut probe, "DEFAULT")
            || take_keyword_ci(&mut probe, "CHECK")
            || take_keyword_ci(&mut probe, "PRIMARY KEY")
            || take_keyword_ci(&mut probe, "UNIQUE")
            || take_keyword_ci(&mut probe, "REFERENCES")
        {
            return Err(format!(
                "Composite type attribute '{name}' cannot declare NOT NULL, DEFAULT, CHECK, or \
                 NONEMPTY constraints in PostgreSQL 18; use CREATE DOMAIN for reusable value \
                 constraints"
            ));
        }
        let other = rest.split_whitespace().next().unwrap_or(rest);
        return Err(format!("Unexpected field attribute '{other}'"));
    }

    Ok(CompositeTypeField {
        name,
        type_ref,
        collation,
    })
}

pub(crate) fn take_type_attributes<'a>(type_ref: &mut TypeReference, mut rest: &'a str) -> &'a str {
    loop {
        if take_keyword_ci(&mut rest, "NOT NULL") {
            type_ref.not_null = true;
            continue;
        }
        if take_keyword_ci(&mut rest, "NULL") {
            type_ref.not_null = false;
            continue;
        }
        if take_keyword_ci(&mut rest, "NONEMPTY") {
            type_ref.nonempty = true;
            continue;
        }
        break;
    }
    rest.trim_start()
}

pub(crate) fn parse_type_reference(input: &str) -> DdlResult<(TypeReference, &str)> {
    let (mut qual, mut rest) = split_qualified_ident(input)?;
    rest = rest.trim_start();

    let is_unqualified_or_pg_catalog =
        qual.namespace_id.as_ref().is_none_or(SchemaId::is_pg_catalog);
    let has_quoted_upper = qual.name_quoted && qual.name.bytes().any(|b| b.is_ascii_uppercase());

    if is_unqualified_or_pg_catalog && !qual.name_quoted {
        if qual.name == "double" && take_keyword_ci(&mut rest, "PRECISION") {
            qual.name = "double precision".to_string();
            rest = rest.trim_start();
        } else if (qual.name == "character" || qual.name == "char" || qual.name == "bit")
            && take_keyword_ci(&mut rest, "VARYING")
        {
            qual.name = format!("{} varying", qual.name);
            rest = rest.trim_start();
        }
    }

    let mut params = None;
    if rest.starts_with('(') {
        let close =
            matching_paren(rest).ok_or_else(|| "Unterminated type parameter list".to_string())?;
        params = Some(&rest[1..close]);
        rest = rest[close + 1..].trim_start();
    }

    if is_unqualified_or_pg_catalog
        && !qual.name_quoted
        && (qual.name == "timestamp" || qual.name == "time")
    {
        if take_keyword_ci(&mut rest, "WITH TIME ZONE") {
            qual.name = if qual.name == "timestamp" {
                "timestamptz".to_string()
            } else {
                "timetz".to_string()
            };
            rest = rest.trim_start();
        } else if take_keyword_ci(&mut rest, "WITHOUT TIME ZONE") {
            rest = rest.trim_start();
        }
    }

    let mut is_array = false;
    while rest.starts_with('[') {
        let Some(close_bracket) = rest.find(']') else {
            return Err("Unterminated array type suffix '['".to_string());
        };
        let inside = rest[1..close_bracket].trim();
        if !inside.is_empty() && !inside.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("Invalid array bound '{inside}'"));
        }
        is_array = true;
        rest = rest[close_bracket + 1..].trim_start();
    }

    let data_type = if is_unqualified_or_pg_catalog && !has_quoted_upper {
        match params {
            Some(params) if !params.is_empty() => {
                KalamDataType::from_sql_name(&format!("{}({params})", qual.name))
            },
            _ => KalamDataType::from_sql_name(&qual.name),
        }
    } else {
        None
    };

    let namespace_id = if data_type.is_some() {
        None
    } else {
        qual.namespace_id
    };

    let syntactic_modifier = params.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    Ok((
        TypeReference {
            namespace_id,
            name: qual.name,
            data_type,
            syntactic_modifier,
            is_array,
            not_null: false,
            nonempty: false,
        },
        rest,
    ))
}

fn parse_namespace_id(name: &str, quoted: bool) -> DdlResult<SchemaId> {
    validate_identifier_bytes(name, "Schema name")?;
    if quoted {
        SchemaId::try_new(name.to_string()).map_err(|error| error.to_string())
    } else {
        SchemaId::try_parse_reference(name).map_err(|error| error.to_string())
    }
}

fn split_parens(input: &str) -> DdlResult<(&str, &str)> {
    let input = input.trim();
    if !input.starts_with('(') {
        return Err("Expected '('".to_string());
    }
    let close = matching_paren(input).ok_or_else(|| "Unterminated '('".to_string())?;
    Ok((&input[1..close], &input[close + 1..]))
}

pub(crate) fn matching_paren(input: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut i = 0usize;
    let mut depth = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\'' {
                        i += 1;
                        if i < bytes.len() && bytes[i] == b'\'' {
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
            },
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'"' {
                        i += 1;
                        if i < bytes.len() && bytes[i] == b'"' {
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
            },
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            },
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                let mut cdepth = 1usize;
                while i + 1 < bytes.len() && cdepth > 0 {
                    if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        cdepth += 1;
                        i += 2;
                    } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        cdepth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if cdepth > 0 {
                    return None;
                }
            },
            b'(' => {
                depth += 1;
                i += 1;
            },
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
                i += 1;
            },
            _ => {
                i += 1;
            },
        }
    }
    None
}

pub(crate) fn split_top_level(input: &str, sep: char) -> Vec<&str> {
    let bytes = input.as_bytes();
    let sep_byte = sep as u8;
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut depth = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\'' {
                        i += 1;
                        if i < bytes.len() && bytes[i] == b'\'' {
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
            },
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'"' {
                        i += 1;
                        if i < bytes.len() && bytes[i] == b'"' {
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
            },
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            },
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                let mut cdepth = 1usize;
                while i + 1 < bytes.len() && cdepth > 0 {
                    if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        cdepth += 1;
                        i += 2;
                    } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        cdepth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            },
            b'(' => {
                depth += 1;
                i += 1;
            },
            b')' => {
                depth -= 1;
                i += 1;
            },
            b if b == sep_byte && depth == 0 => {
                parts.push(&input[start..i]);
                i += 1;
                start = i;
            },
            _ => {
                i += 1;
            },
        }
    }
    parts.push(&input[start..]);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns() -> SchemaId {
        SchemaId::new("app")
    }

    #[test]
    fn parse_composite_type() {
        let stmt = CreateTypeStatement::parse(
            "CREATE TYPE chat.recipient_result AS (
                user_id TEXT,
                delivered BOOLEAN
            )",
            &ns(),
        )
        .unwrap();
        assert_eq!(stmt.namespace_id.as_str(), "chat");
        assert_eq!(stmt.name, "recipient_result");
        match stmt.body {
            CreateTypeBody::Composite { fields } => {
                assert_eq!(fields.len(), 2);
                assert!(!fields[0].type_ref.not_null);
                assert!(!fields[0].type_ref.is_array);
            },
            _ => panic!("expected composite"),
        }
    }

    #[test]
    fn parse_empty_composite_and_empty_enum() {
        let comp = CreateTypeStatement::parse("CREATE TYPE chat.empty_comp AS ()", &ns()).unwrap();
        assert_eq!(comp.body, CreateTypeBody::Composite { fields: vec![] });

        let en =
            CreateTypeStatement::parse("CREATE TYPE chat.empty_enum AS ENUM ()", &ns()).unwrap();
        assert_eq!(en.body, CreateTypeBody::Enum { labels: vec![] });
    }

    #[test]
    fn parse_enum_type_allows_empty_and_colon_labels() {
        let stmt = CreateTypeStatement::parse(
            "CREATE TYPE chat.message_status AS ENUM ('', 'ready:now', 'sent', 'delivered')",
            &ns(),
        )
        .unwrap();
        match stmt.body {
            CreateTypeBody::Enum { labels } => {
                assert_eq!(labels, vec!["", "ready:now", "sent", "delivered"]);
            },
            _ => panic!("expected enum"),
        }
    }

    #[test]
    fn reject_from_table_and_if_not_exists() {
        let err = CreateTypeStatement::parse("CREATE TYPE chat.user FROM TABLE chat.users", &ns())
            .unwrap_err();
        assert!(err.contains("FROM TABLE"), "{err}");

        let err2 =
            CreateTypeStatement::parse("CREATE TYPE IF NOT EXISTS chat.user AS (id TEXT)", &ns())
                .unwrap_err();
        assert!(err2.contains("IF NOT EXISTS"), "{err2}");
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
    fn reject_not_null_and_nonempty_on_composite_attributes() {
        let err =
            CreateTypeStatement::parse("CREATE TYPE t AS (note TEXT NOT NULL)", &ns()).unwrap_err();
        assert!(err.contains("CREATE DOMAIN"), "{err}");

        let err2 =
            CreateTypeStatement::parse("CREATE TYPE t AS (note TEXT NONEMPTY)", &ns()).unwrap_err();
        assert!(err2.contains("CREATE DOMAIN"), "{err2}");
    }

    #[test]
    fn array_and_collation_in_composite_type() {
        let stmt = CreateTypeStatement::parse(
            "CREATE TYPE chat.send_message_result AS (
                recipients chat.recipient_result[],
                locale_label TEXT COLLATE \"C\"
            )",
            &ns(),
        )
        .unwrap();
        match stmt.body {
            CreateTypeBody::Composite { fields } => {
                assert!(fields[0].type_ref.is_array);
                assert_eq!(
                    fields[0].type_ref.namespace_id.as_ref().map(SchemaId::as_str),
                    Some("chat")
                );
                assert_eq!(fields[1].collation.as_deref(), Some("C"));
            },
            _ => panic!("expected composite"),
        }
    }

    #[test]
    fn reject_inline_type_comment() {
        let err = CreateTypeStatement::parse(
            "CREATE TYPE chat.address AS (city TEXT) COMMENT 'Postal address'",
            &ns(),
        )
        .unwrap_err();
        assert!(err.contains("COMMENT ON TYPE"), "{err}");
    }

    #[test]
    fn reject_duplicate_enum_labels() {
        let err = CreateTypeStatement::parse(
            "CREATE TYPE chat.status AS ENUM ('active', 'active')",
            &ns(),
        )
        .unwrap_err();
        assert!(err.contains("duplicate ENUM label"), "{err}");
    }

    #[test]
    fn enum_label_byte_limit_enforced_in_utf8_bytes() {
        // 32 two-byte characters = 64 UTF-8 bytes -> must fail even though char count is 32
        let label_64_bytes = "é".repeat(32);
        assert_eq!(label_64_bytes.len(), 64);
        let sql = format!("CREATE TYPE chat.status AS ENUM ('{label_64_bytes}')");
        let err = CreateTypeStatement::parse(&sql, &ns()).unwrap_err();
        assert!(err.contains("63 bytes"), "{err}");

        // 63 ASCII bytes -> succeeds
        let label_63_bytes = "a".repeat(63);
        let ok_sql = format!("CREATE TYPE chat.status AS ENUM ('{label_63_bytes}')");
        assert!(CreateTypeStatement::parse(&ok_sql, &ns()).is_ok());
    }

    #[test]
    fn allow_case_distinct_enum_labels() {
        let stmt = CreateTypeStatement::parse(
            "CREATE TYPE chat.status AS ENUM ('Active', 'active')",
            &ns(),
        )
        .unwrap();
        match stmt.body {
            CreateTypeBody::Enum { labels } => {
                assert_eq!(labels, vec!["Active", "active"]);
            },
            _ => panic!("expected enum"),
        }
    }

    #[test]
    fn reject_duplicate_composite_fields() {
        let err =
            CreateTypeStatement::parse("CREATE TYPE chat.address AS (city TEXT, city INT)", &ns())
                .unwrap_err();
        assert!(err.contains("duplicate composite field"), "{err}");
    }

    #[test]
    fn parameterized_builtins_keep_kalam_data_type() {
        let stmt = CreateTypeStatement::parse(
            "CREATE TYPE chat.payload AS (
                amount DECIMAL(10, 2),
                vector EMBEDDING(384)
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

    #[test]
    fn quoted_uppercase_type_name_is_not_treated_as_builtin() {
        let stmt = CreateTypeStatement::parse(
            "CREATE TYPE \"Api\".\"Holder\" AS (\"Val\" \"INT\")",
            &ns(),
        )
        .unwrap();
        assert_eq!(stmt.namespace_id.as_str(), "api");
        assert_eq!(stmt.name, "Holder");
        match stmt.body {
            CreateTypeBody::Composite { fields } => {
                assert_eq!(fields[0].name, "Val");
                assert_eq!(fields[0].type_ref.name, "INT");
                assert_eq!(fields[0].type_ref.data_type, None);
            },
            _ => panic!("expected composite"),
        }
    }
}
