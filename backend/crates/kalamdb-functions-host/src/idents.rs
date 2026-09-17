//! JS identifier helpers shared by CLI generation and the runtime routine map.

pub const DEFAULT_NAMESPACE: &str = "public";

pub fn pascal_case(value: &str) -> String {
    let mut name = String::new();
    let mut capitalize = true;
    for ch in value.chars() {
        if ch == '_' || ch == '-' || ch == '.' {
            capitalize = true;
            continue;
        }
        if capitalize {
            for upper in ch.to_uppercase() {
                name.push(upper);
            }
            capitalize = false;
        } else {
            name.push(ch);
        }
    }
    if name.is_empty() {
        "Value".to_string()
    } else {
        name
    }
}

pub fn camel_case(value: &str) -> String {
    let pascal = pascal_case(value);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => "value".to_string(),
    }
}

pub fn method_ident(name: &str) -> String {
    camel_case(name)
}

pub fn namespace_object_ident(namespace: &str) -> String {
    sanitize_js_ident(&camel_case(namespace))
}

pub fn sanitize_js_ident(value: &str) -> String {
    let mut ident: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if ident.is_empty() || ident.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        ident.insert(0, '_');
    }
    if JS_KEYWORDS.contains(&ident.as_str()) {
        ident.push('_');
    }
    ident
}

const JS_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "await",
    "let",
    "static",
    "implements",
    "interface",
    "package",
    "private",
    "protected",
    "public",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_ident_matches_cli_camel_case() {
        assert_eq!(method_ident("create_message"), "createMessage");
        assert_eq!(namespace_object_ident("chat"), "chat");
        assert_eq!(namespace_object_ident("billing_v2"), "billingV2");
    }
}
