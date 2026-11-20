//! Typed insert and update inputs for each generated table.
//!
//! `Table.insert_sql` takes a mapping, so without these mypy sees
//! `Mapping[str, Any]` and checks nothing: a wrong field type is caught on the
//! row dataclass and waved through on the write. A `TypedDict` per operation
//! gives the mapping real keys and real value types, the same way the
//! TypeScript target types a Drizzle insert.

use std::collections::HashSet;

use serde::Serialize;

use super::rows::PyColumn;
use crate::workflow::schema::output::unique_ident;

/// One generated `TypedDict`, rendered whole.
///
/// The declaration has three shapes and the template picks none of them: an
/// `#if` chain around multi-line class bodies made Handlebars' standalone-tag
/// whitespace rules decide the blank lines between declarations, which is not
/// something generated Python should depend on.
#[derive(Serialize)]
pub struct TypedDictContext {
    pub ident: String,
    pub body: String,
}

/// A column as it appears in a write input.
struct WriteField<'a> {
    sql_name: &'a str,
    py_type: String,
    required: bool,
}

/// The insert input: every writable column, required unless the server can
/// fill it.
///
/// Read-only columns are left out rather than marked optional. `insert_sql`
/// rejects them outright, so offering the key would type-check something the
/// runtime always refuses.
pub fn insert_typed_dict(
    row_ident: &str,
    table_label: &str,
    columns: &[PyColumn],
    used: &mut HashSet<String>,
) -> TypedDictContext {
    let fields: Vec<WriteField<'_>> = columns
        .iter()
        .filter(|column| !column.is_read_only)
        .map(|column| WriteField {
            sql_name: &column.sql_name,
            py_type: column.py_type.clone(),
            required: !column.nullable && !column.has_default,
        })
        .collect();
    typed_dict(
        unique_ident(format!("{row_ident}Insert"), used),
        format!("Values accepted by {table_label}.insert_sql."),
        fields,
        used,
    )
}

/// The update input: every writable non-key column, all optional.
///
/// Key columns are excluded because `update_sql` addresses the row by key and
/// refuses to assign one.
pub fn update_typed_dict(
    row_ident: &str,
    table_label: &str,
    columns: &[PyColumn],
    used: &mut HashSet<String>,
) -> TypedDictContext {
    let fields: Vec<WriteField<'_>> = columns
        .iter()
        .filter(|column| !column.is_read_only && !column.is_key)
        .map(|column| WriteField {
            sql_name: &column.sql_name,
            py_type: column.py_type.clone(),
            required: false,
        })
        .collect();
    typed_dict(
        unique_ident(format!("{row_ident}Update"), used),
        format!("Values accepted by {table_label}.update_sql."),
        fields,
        used,
    )
}

fn typed_dict(
    ident: String,
    doc: String,
    fields: Vec<WriteField<'_>>,
    used: &mut HashSet<String>,
) -> TypedDictContext {
    // A column name that is not a Python identifier cannot be a class
    // attribute, so the whole declaration drops to the functional form, which
    // takes any string as a key but cannot mark one required.
    if fields.iter().any(|field| !is_attribute_name(field.sql_name)) {
        let keys: String = fields
            .iter()
            .map(|field| {
                format!(
                    "        \"{}\": {},\n",
                    super::rows::escape_python_string(field.sql_name),
                    field.py_type
                )
            })
            .collect();
        let body = format!(
            "# {doc}\n\
             # Written in the functional form because a column name is not a Python\n\
             # identifier. Every key is optional here, so NOT NULL stays a runtime check.\n\
             {ident} = TypedDict(\n    \"{ident}\",\n    {{\n{keys}    }},\n    total=False,\n)\n"
        );
        return TypedDictContext { ident, body };
    }

    let render = |required: bool| -> String {
        fields
            .iter()
            .filter(|field| field.required == required)
            .map(|field| format!("    {}: {}\n", field.sql_name, field.py_type))
            .collect()
    };
    let required_fields = render(true);
    let optional_fields = render(false);

    let mut body = String::new();
    let base = if required_fields.is_empty() {
        "TypedDict".to_string()
    } else {
        // Required and optional keys are split across two classes because
        // `Required`/`NotRequired` only reached `typing` in 3.11 and the SDK
        // supports 3.9.
        let base = unique_ident(format!("_{ident}Required"), used);
        body.push_str(&format!("class {base}(TypedDict):\n{required_fields}\n\n"));
        base
    };
    body.push_str(&format!("class {ident}({base}, total=False):\n    \"\"\"{doc}\"\"\"\n\n"));
    // A table whose every writable column is required still needs a body.
    body.push_str(if optional_fields.is_empty() {
        "    pass\n"
    } else {
        &optional_fields
    });
    TypedDictContext { ident, body }
}

/// Whether a SQL column name can be written as a class attribute.
///
/// Keywords are excluded along with non-identifier characters: `class: str` in
/// a class body is a syntax error, not a key named "class".
fn is_attribute_name(name: &str) -> bool {
    if name.is_empty() || name.starts_with(|ch: char| ch.is_ascii_digit()) {
        return false;
    }
    if !name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return false;
    }
    // A TypedDict key has to stay the SQL name, so unlike a dataclass field
    // there is nowhere to append the disambiguating underscore to.
    !super::rows::is_python_keyword(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_use_the_class_form() {
        assert!(is_attribute_name("author"));
        assert!(is_attribute_name("_version"));
        assert!(is_attribute_name("user2"));
    }

    #[test]
    fn keywords_and_punctuation_force_the_functional_form() {
        assert!(!is_attribute_name("class"));
        assert!(!is_attribute_name("from"));
        assert!(!is_attribute_name("user-id"));
        assert!(!is_attribute_name("2fa"));
        assert!(!is_attribute_name(""));
    }
}
