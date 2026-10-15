//! Catalog → JS namespace map shared by CoreFunctionHost and tests.

use std::collections::BTreeMap;

use crate::idents::{method_ident, namespace_object_ident};

/// `{ "api": { "createOrder": "api.create_order", "create_order": "api.create_order" } }` for `__kalamMakeCtx`.
pub fn build_routine_js_map<'a>(
    routines: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
) -> String {
    let mut nested: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (namespace, name, id) in routines {
        let camel_ns = namespace_object_ident(namespace);
        let camel_name = method_ident(name);
        {
            let map = nested.entry(camel_ns).or_default();
            map.insert(camel_name.clone(), id.to_string());
            map.insert(name.to_string(), id.to_string());
        }
        if namespace != namespace_object_ident(namespace) {
            let map = nested.entry(namespace.to_string()).or_default();
            map.insert(camel_name, id.to_string());
            map.insert(name.to_string(), id.to_string());
        }
    }
    serde_json::to_string(&nested).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_namespace_and_camel_method() {
        let json = build_routine_js_map([("chat", "create_message", "chat.create_message")]);
        assert!(json.contains(r#""createMessage":"chat.create_message""#));
        assert!(json.contains(r#""create_message":"chat.create_message""#));
    }
}
