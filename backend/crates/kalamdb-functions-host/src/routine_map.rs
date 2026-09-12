//! Catalog → JS namespace map shared by CoreFunctionHost and tests.

use std::collections::BTreeMap;

use crate::idents::{method_ident, namespace_object_ident};

/// `{ "api": { "createOrder": "api.create_order" } }` for `__kalamMakeCtx`.
pub fn build_routine_js_map<'a>(
    routines: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
) -> String {
    let mut nested: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (namespace, name, id) in routines {
        nested
            .entry(namespace_object_ident(namespace))
            .or_default()
            .insert(method_ident(name), id.to_string());
    }
    serde_json::to_string(&nested).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_namespace_and_camel_method() {
        let json = build_routine_js_map([("chat", "create_message", "chat.create_message")]);
        assert_eq!(json, r#"{"chat":{"createMessage":"chat.create_message"}}"#);
    }
}
