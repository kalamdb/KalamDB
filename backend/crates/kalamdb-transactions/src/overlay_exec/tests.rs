use std::{collections::BTreeMap, sync::Arc};

use datafusion::{
    arrow::{
        array::{Int64Array, StringArray},
        datatypes::{DataType, Field, Schema, SchemaRef},
        record_batch::RecordBatch,
    },
    common::{tree_node::TreeNodeRecursion, Result as DataFusionResult},
    error::DataFusionError,
    execution::{SendableRecordBatchStream, TaskContext},
    physical_expr::{EquivalenceProperties, PhysicalExpr},
    physical_plan::{
        ChildrenPropertiesMode, DisplayAs, DisplayFormatType, ExecutionPlan, Partitioning,
        PlanProperties, ReplaceChildrenOptions,
    },
    scalar::ScalarValue,
};
use kalamdb_commons::{
    models::{rows::Row, OperationKind, SchemaId, TableName, TransactionId, UserId},
    TableId, TableType,
};

use super::merge::{merge_batches_with_overlay, merge_record_batches, record_batch_to_rows};
use super::TransactionOverlayExec;
use crate::{
    overlay::{TransactionOverlay, TransactionOverlayEntry},
    query_context::TransactionOverlayView,
};

fn row(values: &[(&str, ScalarValue)]) -> Row {
    let mut fields = BTreeMap::new();
    for (name, value) in values {
        fields.insert((*name).to_string(), value.clone());
    }
    Row::new(fields)
}

#[test]
fn overlay_merge_replaces_updates_filters_deletes_and_appends_inserts() {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let base_batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("before"), Some("remove")])),
        ],
    )
    .expect("base batch");

    let transaction_id = TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
    let table_id = TableId::new(SchemaId::new("app"), TableName::new("items"));
    let mut overlay = TransactionOverlay::new(transaction_id.clone());
    overlay.apply_entry(TransactionOverlayEntry {
        transaction_id: transaction_id.clone(),
        mutation_order: 0,
        table_id: table_id.clone(),
        table_type: TableType::Shared,
        user_id: None,
        operation_kind: OperationKind::Update,
        primary_key: "1".to_string(),
        payload: row(&[("name", ScalarValue::Utf8(Some("after".to_string())))]),
        tombstone: false,
    });
    overlay.apply_entry(TransactionOverlayEntry {
        transaction_id: transaction_id.clone(),
        mutation_order: 1,
        table_id: table_id.clone(),
        table_type: TableType::Shared,
        user_id: None,
        operation_kind: OperationKind::Delete,
        primary_key: "2".to_string(),
        payload: Row::new(BTreeMap::new()),
        tombstone: true,
    });
    overlay.apply_entry(TransactionOverlayEntry {
        transaction_id,
        mutation_order: 2,
        table_id: table_id.clone(),
        table_type: TableType::Shared,
        user_id: None,
        operation_kind: OperationKind::Insert,
        primary_key: "3".to_string(),
        payload: row(&[
            ("id", ScalarValue::Int64(Some(3))),
            ("name", ScalarValue::Utf8(Some("inserted".to_string()))),
        ]),
        tombstone: false,
    });

    let merged = merge_batches_with_overlay(
        &schema,
        &table_id,
        "id",
        &overlay,
        None,
        &[base_batch],
        None,
        None,
    )
    .expect("merged batch");

    assert_eq!(merged.num_rows(), 2);
    let rows = record_batch_to_rows(&merged).expect("rows");
    assert_eq!(rows[0].values.get("id"), Some(&ScalarValue::Int64(Some(1))));
    assert_eq!(rows[0].values.get("name"), Some(&ScalarValue::Utf8(Some("after".to_string()))));
    assert_eq!(rows[1].values.get("id"), Some(&ScalarValue::Int64(Some(3))));
    assert_eq!(
        rows[1].values.get("name"),
        Some(&ScalarValue::Utf8(Some("inserted".to_string())))
    );
}

#[test]
fn overlay_update_outside_the_scan_is_not_emitted_as_a_partial_row() {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let base_batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1])),
            Arc::new(StringArray::from(vec![Some("kept")])),
        ],
    )
    .expect("base batch");

    let transaction_id = TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
    let table_id = TableId::new(SchemaId::new("app"), TableName::new("items"));
    let mut overlay = TransactionOverlay::new(transaction_id.clone());
    overlay.apply_entry(TransactionOverlayEntry {
        transaction_id,
        mutation_order: 0,
        table_id: table_id.clone(),
        table_type: TableType::Shared,
        user_id: None,
        operation_kind: OperationKind::Update,
        primary_key: "9".to_string(),
        payload: row(&[("name", ScalarValue::Utf8(Some("patched".to_string())))]),
        tombstone: false,
    });

    let merged = merge_batches_with_overlay(
        &schema,
        &table_id,
        "id",
        &overlay,
        None,
        &[base_batch],
        None,
        None,
    )
    .expect("merged batch");

    assert_eq!(merged.num_rows(), 1);
    let rows = record_batch_to_rows(&merged).expect("rows");
    assert_eq!(rows[0].values.get("id"), Some(&ScalarValue::Int64(Some(1))));
}

#[test]
fn shared_update_staged_by_session_user_patches_scanned_row() {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let base_batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("before"), Some("gone")])),
        ],
    )
    .expect("base batch");

    let transaction_id = TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
    let table_id = TableId::new(SchemaId::new("app"), TableName::new("items"));
    let session_user = UserId::new("user-a");
    let mut overlay = TransactionOverlay::new(transaction_id.clone());
    overlay.apply_entry(TransactionOverlayEntry {
        transaction_id: transaction_id.clone(),
        mutation_order: 0,
        table_id: table_id.clone(),
        table_type: TableType::Shared,
        user_id: Some(session_user.clone()),
        operation_kind: OperationKind::Update,
        primary_key: "1".to_string(),
        payload: row(&[("name", ScalarValue::Utf8(Some("after".to_string())))]),
        tombstone: false,
    });
    overlay.apply_entry(TransactionOverlayEntry {
        transaction_id,
        mutation_order: 1,
        table_id: table_id.clone(),
        table_type: TableType::Shared,
        user_id: Some(session_user),
        operation_kind: OperationKind::Delete,
        primary_key: "2".to_string(),
        payload: Row::new(BTreeMap::new()),
        tombstone: true,
    });

    let merged = merge_batches_with_overlay(
        &schema,
        &table_id,
        "id",
        &overlay,
        None,
        &[base_batch],
        None,
        None,
    )
    .expect("merged batch");

    let rows = record_batch_to_rows(&merged).expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].values.get("name"), Some(&ScalarValue::Utf8(Some("after".to_string()))));
}

#[test]
fn overlay_update_keeps_base_version_and_insert_omits_invented_version() {
    use kalamdb_commons::constants::SystemColumnNames;

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new(SystemColumnNames::VERSION, DataType::Int64, true),
        Field::new("name", DataType::Utf8, true),
    ]));
    let base_version = 42_i64;
    let base_batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1])),
            Arc::new(Int64Array::from(vec![Some(base_version)])),
            Arc::new(StringArray::from(vec![Some("before")])),
        ],
    )
    .expect("base batch");

    let transaction_id = TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
    let table_id = TableId::new(SchemaId::new("app"), TableName::new("items"));
    let mut overlay = TransactionOverlay::new(transaction_id.clone());
    overlay.apply_entry(TransactionOverlayEntry {
        transaction_id: transaction_id.clone(),
        mutation_order: 0,
        table_id: table_id.clone(),
        table_type: TableType::Shared,
        user_id: None,
        operation_kind: OperationKind::Update,
        primary_key: "1".to_string(),
        payload: row(&[
            ("name", ScalarValue::Utf8(Some("after".to_string()))),
            (SystemColumnNames::VERSION, ScalarValue::Int64(Some(99))),
        ]),
        tombstone: false,
    });
    overlay.apply_entry(TransactionOverlayEntry {
        transaction_id,
        mutation_order: 1,
        table_id: table_id.clone(),
        table_type: TableType::Shared,
        user_id: None,
        operation_kind: OperationKind::Insert,
        primary_key: "2".to_string(),
        payload: row(&[
            ("id", ScalarValue::Int64(Some(2))),
            ("name", ScalarValue::Utf8(Some("inserted".to_string()))),
            (SystemColumnNames::VERSION, ScalarValue::Int64(Some(7))),
        ]),
        tombstone: false,
    });

    let merged = merge_batches_with_overlay(
        &schema,
        &table_id,
        "id",
        &overlay,
        None,
        &[base_batch],
        None,
        None,
    )
    .expect("merged batch");
    let rows = record_batch_to_rows(&merged).expect("rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].values.get(SystemColumnNames::VERSION),
        Some(&ScalarValue::Int64(Some(base_version)))
    );
    assert!(
        rows[1].values.get(SystemColumnNames::VERSION).is_none()
            || matches!(
                rows[1].values.get(SystemColumnNames::VERSION),
                Some(ScalarValue::Int64(None) | ScalarValue::Null)
            )
    );
}

#[test]
fn overlay_merge_honors_user_scope_for_same_primary_key() {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let empty_batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(Vec::<i64>::new())),
            Arc::new(StringArray::from(Vec::<Option<&str>>::new())),
        ],
    )
    .expect("empty batch");

    let transaction_id = TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
    let table_id = TableId::new(SchemaId::new("app"), TableName::new("items"));
    let first_user = UserId::new("user-a");
    let second_user = UserId::new("user-b");
    let mut overlay = TransactionOverlay::new(transaction_id.clone());

    overlay.apply_entry(TransactionOverlayEntry {
        transaction_id: transaction_id.clone(),
        mutation_order: 0,
        table_id: table_id.clone(),
        table_type: TableType::User,
        user_id: Some(first_user.clone()),
        operation_kind: OperationKind::Insert,
        primary_key: "1".to_string(),
        payload: row(&[
            ("id", ScalarValue::Int64(Some(1))),
            ("name", ScalarValue::Utf8(Some("alice".to_string()))),
        ]),
        tombstone: false,
    });
    overlay.apply_entry(TransactionOverlayEntry {
        transaction_id,
        mutation_order: 1,
        table_id: table_id.clone(),
        table_type: TableType::User,
        user_id: Some(second_user.clone()),
        operation_kind: OperationKind::Insert,
        primary_key: "1".to_string(),
        payload: row(&[
            ("id", ScalarValue::Int64(Some(1))),
            ("name", ScalarValue::Utf8(Some("bob".to_string()))),
        ]),
        tombstone: false,
    });

    let first_merged = merge_batches_with_overlay(
        &schema,
        &table_id,
        "id",
        &overlay,
        Some(&first_user),
        &[empty_batch.clone()],
        None,
        None,
    )
    .expect("first user merged batch");
    let second_merged = merge_batches_with_overlay(
        &schema,
        &table_id,
        "id",
        &overlay,
        Some(&second_user),
        &[empty_batch],
        None,
        None,
    )
    .expect("second user merged batch");

    let first_rows = record_batch_to_rows(&first_merged).expect("first rows");
    let second_rows = record_batch_to_rows(&second_merged).expect("second rows");
    assert_eq!(first_rows.len(), 1);
    assert_eq!(second_rows.len(), 1);
    assert_eq!(
        first_rows[0].values.get("name"),
        Some(&ScalarValue::Utf8(Some("alice".to_string())))
    );
    assert_eq!(
        second_rows[0].values.get("name"),
        Some(&ScalarValue::Utf8(Some("bob".to_string())))
    );
}

#[tokio::test]
async fn overlay_exec_reads_rows_staged_after_planning_once_across_partitions() {
    use datafusion::{execution::context::SessionContext, physical_plan::collect};
    use kalamdb_datafusion_sources::stream::one_shot_batch_stream;

    #[derive(Debug)]
    struct EmptyTwoPartitionExec {
        schema: SchemaRef,
        properties: Arc<PlanProperties>,
    }

    impl EmptyTwoPartitionExec {
        fn new(schema: SchemaRef) -> Self {
            Self {
                schema: Arc::clone(&schema),
                properties: Arc::new(PlanProperties::new(
                    EquivalenceProperties::new(schema),
                    Partitioning::UnknownPartitioning(2),
                    datafusion::physical_plan::execution_plan::EmissionType::Incremental,
                    datafusion::physical_plan::execution_plan::Boundedness::Bounded,
                )),
            }
        }
    }

    impl DisplayAs for EmptyTwoPartitionExec {
        fn fmt_as(&self, _t: DisplayFormatType, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "EmptyTwoPartitionExec")
        }
    }

    impl ExecutionPlan for EmptyTwoPartitionExec {
        fn name(&self) -> &str {
            Self::static_name()
        }

        fn properties(&self) -> &Arc<PlanProperties> {
            &self.properties
        }

        fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
            Vec::new()
        }

        fn apply_expressions(
            &self,
            _f: &mut dyn FnMut(&Arc<dyn PhysicalExpr>) -> DataFusionResult<TreeNodeRecursion>,
        ) -> DataFusionResult<TreeNodeRecursion> {
            Ok(TreeNodeRecursion::Continue)
        }

        fn replace_children(
            self: Arc<Self>,
            children: Vec<Arc<dyn ExecutionPlan>>,
            _options: ReplaceChildrenOptions,
        ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
            if !children.is_empty() {
                return Err(DataFusionError::Execution(
                    "EmptyTwoPartitionExec does not accept children".to_string(),
                ));
            }
            Ok(self)
        }

        fn with_new_children(
            self: Arc<Self>,
            children: Vec<Arc<dyn ExecutionPlan>>,
        ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
            self.replace_children(
                children,
                ReplaceChildrenOptions::new(ChildrenPropertiesMode::Recompute),
            )
        }

        fn execute(
            &self,
            partition: usize,
            _context: Arc<TaskContext>,
        ) -> DataFusionResult<SendableRecordBatchStream> {
            if partition >= 2 {
                return Err(DataFusionError::Execution(format!(
                    "unexpected partition {partition}",
                )));
            }

            let batch = RecordBatch::new_empty(Arc::clone(&self.schema));
            Ok(one_shot_batch_stream(Arc::clone(&self.schema), async move { Ok(batch) }))
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    #[derive(Debug)]
    struct LiveOverlayView(std::sync::Mutex<TransactionOverlay>);

    impl TransactionOverlayView for LiveOverlayView {
        fn overlay(&self) -> TransactionOverlay {
            self.0.lock().expect("overlay lock").clone()
        }

        fn overlay_for_table(&self, table_id: &TableId) -> Option<TransactionOverlay> {
            self.0.lock().expect("overlay lock").table_overlay(table_id)
        }
    }

    let transaction_id = TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
    let table_id = TableId::new(SchemaId::new("app"), TableName::new("items"));
    let view = Arc::new(LiveOverlayView(std::sync::Mutex::new(TransactionOverlay::new(
        transaction_id.clone(),
    ))));

    let exec = Arc::new(
        TransactionOverlayExec::try_new(
            Arc::new(EmptyTwoPartitionExec::new(Arc::clone(&schema))),
            table_id.clone(),
            "id",
            Arc::clone(&view) as Arc<dyn TransactionOverlayView>,
            None,
            None,
            None,
            None,
        )
        .expect("overlay exec"),
    );

    view.0.lock().expect("overlay lock").apply_entry(TransactionOverlayEntry {
        transaction_id,
        mutation_order: 0,
        table_id,
        table_type: TableType::Shared,
        user_id: None,
        operation_kind: OperationKind::Insert,
        primary_key: "1".to_string(),
        payload: row(&[
            ("id", ScalarValue::Int64(Some(1))),
            ("name", ScalarValue::Utf8(Some("alpha".to_string()))),
        ]),
        tombstone: false,
    });

    assert_eq!(exec.properties().output_partitioning().partition_count(), 1);

    let batches = collect(exec, SessionContext::new().task_ctx())
        .await
        .expect("collect overlay rows");
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 1);
    assert_eq!(
        batches[0]
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("name array")
            .value(0),
        "alpha"
    );
}

#[test]
fn disjoint_overlay_batch_keeps_input_columns() {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let base_batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("one"), Some("two")])),
        ],
    )
    .expect("base batch");
    let transaction_id = TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
    let table_id = TableId::new(SchemaId::new("app"), TableName::new("items"));
    let mut overlay = TransactionOverlay::new(transaction_id.clone());
    overlay.apply_entry(TransactionOverlayEntry {
        transaction_id,
        mutation_order: 0,
        table_id: table_id.clone(),
        table_type: TableType::Shared,
        user_id: None,
        operation_kind: OperationKind::Update,
        primary_key: "9".to_string(),
        payload: row(&[("name", ScalarValue::Utf8(Some("elsewhere".to_string())))]),
        tombstone: false,
    });

    let merged = merge_batches_with_overlay(
        &schema,
        &table_id,
        "id",
        &overlay,
        None,
        &[base_batch.clone()],
        None,
        None,
    )
    .expect("merged batch");
    assert!(Arc::ptr_eq(merged.column(0), base_batch.column(0)));
    assert!(Arc::ptr_eq(merged.column(1), base_batch.column(1)));
}

#[test]
fn dirty_reads_match_for_one_hundred_and_one_thousand_rows() {
    use datafusion::{common::DFSchema, logical_expr::Expr, prelude::SessionContext};

    for row_count in [100_i64, 1_000] {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
            Field::new("color", DataType::Utf8, true),
        ]));
        let ids: Vec<i64> = (0..row_count).collect();
        let names: Vec<Option<&str>> = (0..row_count)
            .map(|id| match id {
                1 | 3 | 4 => Some("keep"),
                2 => Some("other"),
                _ => Some("no"),
            })
            .collect();
        let colors = vec![Some("blue"); row_count as usize];
        let base_batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(names)),
                Arc::new(StringArray::from(colors)),
            ],
        )
        .expect("base batch");

        let transaction_id = TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
        let table_id = TableId::new(SchemaId::new("app"), TableName::new("items"));
        let mut overlay = TransactionOverlay::new(transaction_id.clone());
        overlay.apply_entry(TransactionOverlayEntry {
            transaction_id: transaction_id.clone(),
            mutation_order: 0,
            table_id: table_id.clone(),
            table_type: TableType::Shared,
            user_id: None,
            operation_kind: OperationKind::Update,
            primary_key: "1".to_string(),
            payload: row(&[("name", ScalarValue::Utf8(Some("drop".to_string())))]),
            tombstone: false,
        });
        overlay.apply_entry(TransactionOverlayEntry {
            transaction_id: transaction_id.clone(),
            mutation_order: 1,
            table_id: table_id.clone(),
            table_type: TableType::Shared,
            user_id: None,
            operation_kind: OperationKind::Update,
            primary_key: "2".to_string(),
            payload: row(&[("name", ScalarValue::Utf8(Some("keep".to_string())))]),
            tombstone: false,
        });
        overlay.apply_entry(TransactionOverlayEntry {
            transaction_id: transaction_id.clone(),
            mutation_order: 2,
            table_id: table_id.clone(),
            table_type: TableType::Shared,
            user_id: None,
            operation_kind: OperationKind::Delete,
            primary_key: "3".to_string(),
            payload: Row::new(BTreeMap::new()),
            tombstone: true,
        });
        let inserted_id = row_count + 1;
        overlay.apply_entry(TransactionOverlayEntry {
            transaction_id: transaction_id.clone(),
            mutation_order: 3,
            table_id: table_id.clone(),
            table_type: TableType::Shared,
            user_id: None,
            operation_kind: OperationKind::Insert,
            primary_key: inserted_id.to_string(),
            payload: row(&[
                ("id", ScalarValue::Int64(Some(inserted_id))),
                ("name", ScalarValue::Utf8(Some("keep".to_string()))),
                ("color", ScalarValue::Utf8(Some("green".to_string()))),
            ]),
            tombstone: false,
        });
        overlay.apply_entry(TransactionOverlayEntry {
            transaction_id,
            mutation_order: 4,
            table_id: table_id.clone(),
            table_type: TableType::Shared,
            user_id: None,
            operation_kind: OperationKind::Insert,
            primary_key: (row_count + 2).to_string(),
            payload: row(&[
                ("id", ScalarValue::Int64(Some(row_count + 2))),
                ("name", ScalarValue::Utf8(Some("drop".to_string()))),
                ("color", ScalarValue::Utf8(Some("green".to_string()))),
            ]),
            tombstone: false,
        });

        let ctx = SessionContext::new();
        let df_schema = DFSchema::try_from(Arc::clone(&schema)).expect("df schema");
        let predicate = ctx
            .create_physical_expr(Expr::eq(col_name("name"), lit_utf8("keep")), &df_schema)
            .expect("predicate");
        let merged = merge_record_batches(
            &schema,
            &table_id,
            "id",
            Some(&overlay),
            None,
            vec![base_batch],
            None,
            None,
            Some(&predicate),
            true,
        )
        .expect("dirty read");
        let rows = record_batch_to_rows(&merged).expect("rows");
        let ids = rows
            .iter()
            .map(|row| match row.values.get("id") {
                Some(ScalarValue::Int64(Some(id))) => *id,
                other => panic!("id {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![2, 4, inserted_id]);
        assert_eq!(rows[0].values.get("color"), Some(&ScalarValue::Utf8(Some("blue".to_string()))));
        assert_eq!(rows[0].values.get("name"), Some(&ScalarValue::Utf8(Some("keep".to_string()))));
    }
}

#[test]
fn fetch_is_applied_after_the_residual_filter() {
    use datafusion::{
        common::DFSchema,
        logical_expr::{col, lit},
        prelude::SessionContext,
    };

    let ctx = SessionContext::new();
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let base_batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("one"), Some("two")])),
        ],
    )
    .expect("base batch");
    let df_schema = DFSchema::try_from(Arc::clone(&schema)).expect("schema");
    let predicate = ctx
        .state()
        .create_physical_expr(col("id").gt(lit(1_i64)), &df_schema)
        .expect("predicate");
    let table_id = TableId::new(SchemaId::new("app"), TableName::new("items"));

    let merged = merge_record_batches(
        &schema,
        &table_id,
        "id",
        None,
        None,
        vec![base_batch],
        None,
        Some(1),
        Some(&predicate),
        true,
    )
    .expect("merged batch");

    assert_eq!(merged.num_rows(), 1);
    assert_eq!(merged.column(0).as_any().downcast_ref::<Int64Array>().expect("id").value(0), 2);
}

fn col_name(name: &str) -> datafusion::logical_expr::Expr {
    datafusion::logical_expr::Expr::Column(datafusion::common::Column::from(name))
}

fn lit_utf8(value: &str) -> datafusion::logical_expr::Expr {
    datafusion::logical_expr::Expr::Literal(ScalarValue::Utf8(Some(value.to_string())), None)
}
