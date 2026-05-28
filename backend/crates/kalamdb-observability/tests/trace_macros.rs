#[test]
fn span_macros_are_usable_from_call_sites() {
    let _guard = kalamdb_observability::kdb_debug_span_entered!(
        "observability.test",
        table_id = "default.test"
    );
    kalamdb_observability::kdb_debug!(table_id = "default.test", "observability test event");
}
