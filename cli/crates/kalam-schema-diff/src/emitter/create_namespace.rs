pub(super) fn emit_create_namespace(namespace: &str) -> String {
    format!("CREATE NAMESPACE IF NOT EXISTS {namespace};")
}
