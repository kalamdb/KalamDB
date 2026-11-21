pub(super) fn emit_create_namespace(namespace: &str) -> String {
    format!("CREATE SCHEMA IF NOT EXISTS {namespace};")
}
