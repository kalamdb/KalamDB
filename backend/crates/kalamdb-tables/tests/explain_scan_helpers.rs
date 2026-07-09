use datafusion::{
    arrow::{array::Array, array::StringArray, record_batch::RecordBatch},
    execution::context::SessionContext,
};

pub fn explain_plan_text_from_batches(batches: &[RecordBatch]) -> String {
    let mut lines = Vec::new();
    for batch in batches {
        let plan_type = batch
            .column_by_name("plan_type")
            .and_then(|column| column.as_any().downcast_ref::<StringArray>());
        let plan = batch
            .column_by_name("plan")
            .and_then(|column| column.as_any().downcast_ref::<StringArray>());
        let (Some(plan_type), Some(plan)) = (plan_type, plan) else {
            continue;
        };
        for row in 0..batch.num_rows() {
            if plan_type.is_valid(row) && plan.is_valid(row) {
                lines.push(format!("{} | {}", plan_type.value(row), plan.value(row)));
            }
        }
    }
    lines.join("\n")
}

pub async fn explain_analyze_plan_text(ctx: &SessionContext, sql: &str) -> String {
    let batches = ctx
        .sql(sql)
        .await
        .unwrap_or_else(|error| panic!("EXPLAIN ANALYZE failed for '{sql}': {error}"))
        .collect()
        .await
        .unwrap_or_else(|error| panic!("collect EXPLAIN ANALYZE failed for '{sql}': {error}"));
    explain_plan_text_from_batches(&batches)
}

pub fn assert_explain_contains(plan_text: &str, expected_fragments: &[&str], context: &str) {
    for fragment in expected_fragments {
        assert!(
            plan_text.contains(fragment),
            "{context}: expected '{fragment}' in EXPLAIN ANALYZE output:\n{plan_text}"
        );
    }
}

pub async fn assert_explain_analyze_scan_targets(
    ctx: &SessionContext,
    table_name: &str,
    expected_fragments: &[&str],
    context: &str,
) {
    let sql = format!("EXPLAIN ANALYZE SELECT * FROM {table_name}");
    let plan_text = explain_analyze_plan_text(ctx, &sql).await;
    assert_explain_contains(&plan_text, expected_fragments, context);
}
