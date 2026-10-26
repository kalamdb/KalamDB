//! SQL execution for the shared session used by HTTP and pgwire.
//!
//! [`SqlExecutor::execute`] dispatches a classified statement. Planning, DML,
//! catalog routines, and request transactions live in the sibling modules.

use std::{sync::Arc, time::Duration};

mod catalog_routines;
mod datafusion_error;
mod equality_update;
mod execute;
mod literal_dml;
mod planning;
mod point_get;
mod postgres_meta;
mod prepare;
mod returning;
mod system_migrations;
mod table_dml;
mod transactions;

use super::{handler_registry::HandlerRegistry, SqlExecutor};
use crate::sql::plan_cache::{SqlCacheRegistry, SqlCacheRegistryConfig};

#[derive(Debug, Clone, Copy)]
pub(super) enum DmlKind {
    Insert,
    Update,
    Delete,
}

fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();

    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }

    let first = needle[0].to_ascii_lowercase();
    haystack.windows(needle.len()).any(|window| {
        window[0].to_ascii_lowercase() == first && window.eq_ignore_ascii_case(needle)
    })
}

fn contains_internal_namespace_hint(sql: &str) -> bool {
    contains_ignore_ascii_case(sql, "system.") || contains_ignore_ascii_case(sql, "dba.")
}

impl SqlExecutor {
    /// Construct a new executor with a pre-built handler registry.
    pub fn new(
        app_context: std::sync::Arc<crate::app_context::AppContext>,
        handler_registry: Arc<HandlerRegistry>,
    ) -> Self {
        let plan_max_entries = app_context.config().execution.sql_plan_cache_max_entries;
        let plan_idle_ttl =
            Duration::from_secs(app_context.config().execution.sql_plan_cache_ttl_seconds);
        let sql_cache_registry = Arc::new(SqlCacheRegistry::new(SqlCacheRegistryConfig::new(
            plan_max_entries,
            plan_idle_ttl,
        )));
        let prepared_statement_cache = moka::sync::Cache::builder()
            .max_capacity(plan_max_entries)
            .time_to_idle(plan_idle_ttl)
            .build();
        let point_read_session_cache = moka::sync::Cache::builder()
            .max_capacity(plan_max_entries.min(64))
            .time_to_idle(plan_idle_ttl)
            .build();
        Self {
            app_context,
            handler_registry,
            sql_cache_registry,
            prepared_statement_cache,
            point_read_session_cache,
            #[cfg(test)]
            point_get_fast_path_hits: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Clear SQL caches that may become stale after DDL operations.
    pub fn clear_plan_cache(&self) {
        self.sql_cache_registry.clear();
        self.prepared_statement_cache.invalidate_all();
        self.point_read_session_cache.invalidate_all();
        self.prepared_statement_cache.run_pending_tasks();
        self.point_read_session_cache.run_pending_tasks();
    }

    /// Get current plan cache size (diagnostics/testing)
    pub fn plan_cache_len(&self) -> usize {
        self.sql_cache_registry.plan_cache().len()
    }

    /// Get current prepared-statement metadata cache size (diagnostics/testing).
    pub fn prepared_statement_cache_len(&self) -> usize {
        self.prepared_statement_cache.run_pending_tasks();
        self.prepared_statement_cache.entry_count() as usize
    }

    #[cfg(test)]
    pub fn point_read_session_cache_len(&self) -> usize {
        self.point_read_session_cache.run_pending_tasks();
        self.point_read_session_cache.entry_count() as usize
    }

    #[cfg(test)]
    pub fn point_get_fast_path_hits(&self) -> u64 {
        self.point_get_fast_path_hits.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::RecordBatch,
        datatypes::{Field, Schema},
    };
    use datafusion::{
        common::DFSchema,
        logical_expr::{lit, EmptyRelation, Limit, LogicalPlan, Sort, SortExpr},
        scalar::ScalarValue,
    };
    use kalamdb_commons::{
        datatypes::KalamDataType,
        models::{rows::row::Row, SchemaId, TableId, UserId},
        schemas::{ColumnDefinition, TableDefinition, TableOptions, TableType},
        BoundExprShape, PolicyCommand, PolicyId, PolicyProgram, PolicyTarget, PrincipalExpr, Role,
        TableName, TablePolicy,
    };
    use kalamdb_tables::{utils::KalamTableProvider, SharedTableProvider};

    use super::{contains_internal_namespace_hint, HandlerRegistry, SqlExecutor};
    use crate::sql::{ExecutionContext, ExecutionResult};

    fn result_row_count(result: ExecutionResult) -> usize {
        match result {
            ExecutionResult::Rows { row_count, .. }
            | ExecutionResult::ScalarRows { row_count, .. } => row_count,
            other => panic!("expected rows, got {other:?}"),
        }
    }

    fn empty_plan() -> LogicalPlan {
        LogicalPlan::EmptyRelation(EmptyRelation {
            produce_one_row: false,
            schema: Arc::new(DFSchema::empty()),
        })
    }

    #[test]
    fn internal_namespace_hint_matches_qualified_system_and_dba_queries() {
        assert!(contains_internal_namespace_hint("SELECT * FROM system.stats LIMIT 100"));
        assert!(contains_internal_namespace_hint("select * from DBA.notifications"));
        assert!(!contains_internal_namespace_hint("SELECT * FROM default.events LIMIT 100"));
    }

    #[test]
    fn logical_plan_has_limit_detects_sort_fetch_and_limit_nodes() {
        let unlimited = empty_plan();
        assert!(!SqlExecutor::logical_plan_has_limit(&unlimited));

        let with_limit = LogicalPlan::Limit(Limit {
            skip: None,
            fetch: Some(Box::new(lit(5000_i64))),
            input: Arc::new(empty_plan()),
        });
        assert!(SqlExecutor::logical_plan_has_limit(&with_limit));

        let with_sort_fetch = LogicalPlan::Sort(Sort {
            expr: vec![SortExpr::new(lit(1_i64), true, false)],
            input: Arc::new(empty_plan()),
            fetch: Some(5000),
        });
        assert!(SqlExecutor::logical_plan_has_limit(&with_sort_fetch));

        let sort_without_fetch = LogicalPlan::Sort(Sort {
            expr: vec![SortExpr::new(lit(1_i64), true, false)],
            input: Arc::new(empty_plan()),
            fetch: None,
        });
        assert!(!SqlExecutor::logical_plan_has_limit(&sort_without_fetch));
    }

    #[test]
    fn unwrap_default_order_wrappers_peels_sort_and_limit_only() {
        let sorted = LogicalPlan::Sort(Sort {
            expr: vec![SortExpr::new(lit(1_i64), true, false)],
            input: Arc::new(empty_plan()),
            fetch: None,
        });
        assert!(matches!(
            SqlExecutor::unwrap_default_order_wrappers(&sorted),
            LogicalPlan::EmptyRelation(_)
        ));

        let limited = LogicalPlan::Limit(Limit {
            skip: None,
            fetch: Some(Box::new(lit(1_i64))),
            input: Arc::new(sorted),
        });
        assert!(matches!(
            SqlExecutor::unwrap_default_order_wrappers(&limited),
            LogicalPlan::EmptyRelation(_)
        ));
    }

    #[test]
    fn project_point_get_batches_selects_non_leading_columns_by_name() {
        use arrow::{array::StringArray, datatypes::DataType};

        let source_schema = Arc::new(Schema::new(vec![
            Field::new("path", DataType::Utf8, false),
            Field::new("file_ref", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&source_schema),
            vec![
                Arc::new(StringArray::from(vec!["notes.md"])),
                Arc::new(StringArray::from(vec!["sha-abc"])),
            ],
        )
        .unwrap();
        let target_schema =
            Arc::new(Schema::new(vec![Field::new("file_ref", DataType::Utf8, true)]));

        let projected = SqlExecutor::project_point_get_batches(
            vec![batch],
            source_schema,
            Arc::clone(&target_schema),
        )
        .expect("projection should keep file_ref");

        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].schema().fields().len(), 1);
        assert_eq!(projected[0].schema().field(0).name(), "file_ref");
        let values = projected[0].column(0).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(values.value(0), "sha-abc");
    }

    #[test]
    fn project_point_get_batches_reorders_columns_by_name() {
        use arrow::{array::StringArray, datatypes::DataType};

        let source_schema = Arc::new(Schema::new(vec![
            Field::new("path", DataType::Utf8, false),
            Field::new("file_ref", DataType::Utf8, true),
            Field::new("body", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&source_schema),
            vec![
                Arc::new(StringArray::from(vec!["notes.md"])),
                Arc::new(StringArray::from(vec!["sha-abc"])),
                Arc::new(StringArray::from(vec!["hello"])),
            ],
        )
        .unwrap();
        let target_schema = Arc::new(Schema::new(vec![
            Field::new("file_ref", DataType::Utf8, true),
            Field::new("path", DataType::Utf8, false),
        ]));

        let projected = SqlExecutor::project_point_get_batches(
            vec![batch],
            source_schema,
            Arc::clone(&target_schema),
        )
        .expect("reordered projection");

        assert_eq!(projected[0].schema().field(0).name(), "file_ref");
        assert_eq!(projected[0].schema().field(1).name(), "path");
        let file_ref = projected[0].column(0).as_any().downcast_ref::<StringArray>().unwrap();
        let path = projected[0].column(1).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(file_ref.value(0), "sha-abc");
        assert_eq!(path.value(0), "notes.md");
    }

    #[test]
    fn project_point_get_batches_returns_none_for_unknown_column() {
        use arrow::{array::StringArray, datatypes::DataType};

        let source_schema = Arc::new(Schema::new(vec![Field::new("path", DataType::Utf8, false)]));
        let batch = RecordBatch::try_new(
            Arc::clone(&source_schema),
            vec![Arc::new(StringArray::from(vec!["notes.md"]))],
        )
        .unwrap();
        let target_schema =
            Arc::new(Schema::new(vec![Field::new("file_ref", DataType::Utf8, true)]));

        assert!(
            SqlExecutor::project_point_get_batches(vec![batch], source_schema, target_schema)
                .is_none()
        );
    }

    #[test]
    fn project_point_get_batches_preserves_empty_target_schema() {
        use arrow::datatypes::DataType;

        let source_schema = Arc::new(Schema::new(vec![
            Field::new("path", DataType::Utf8, false),
            Field::new("file_ref", DataType::Utf8, true),
        ]));
        let target_schema =
            Arc::new(Schema::new(vec![Field::new("file_ref", DataType::Utf8, true)]));

        let projected = SqlExecutor::project_point_get_batches(
            Vec::new(),
            source_schema,
            Arc::clone(&target_schema),
        )
        .expect("empty projection");

        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].num_rows(), 0);
        assert_eq!(projected[0].schema().field(0).name(), "file_ref");
    }

    #[test]
    fn project_point_get_scalar_rows_remaps_alias_when_scan_keeps_pk() {
        use arrow::datatypes::DataType;
        use datafusion::logical_expr::col;

        let source_schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]));
        let target_schema = Arc::new(Schema::new(vec![Field::new("title", DataType::Utf8, true)]));
        let rows = vec![Row::from_vec(vec![
            ("id".to_string(), ScalarValue::Int64(Some(1))),
            ("name".to_string(), ScalarValue::Utf8(Some("alpha".to_string()))),
        ])];
        let exprs = vec![col("name").alias("title")];

        let projected = SqlExecutor::project_point_get_scalar_rows(
            rows,
            &source_schema,
            target_schema,
            Some(exprs.as_slice()),
        )
        .expect("aliased projection");

        assert_eq!(
            projected[0].values.get("title"),
            Some(&ScalarValue::Utf8(Some("alpha".to_string())))
        );
        assert!(!projected[0].values.contains_key("name"));
        assert!(!projected[0].values.contains_key("id"));
    }

    #[test]
    fn project_point_get_scalar_rows_returns_none_for_unknown_alias_without_exprs() {
        use arrow::datatypes::DataType;

        let source_schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]));
        let target_schema = Arc::new(Schema::new(vec![Field::new("title", DataType::Utf8, true)]));
        let rows = vec![Row::from_vec(vec![
            ("id".to_string(), ScalarValue::Int64(Some(1))),
            ("name".to_string(), ScalarValue::Utf8(Some("alpha".to_string()))),
        ])];

        assert!(
            SqlExecutor::project_point_get_scalar_rows(rows, &source_schema, target_schema, None)
                .is_none(),
            "unmapped aliases must fall back to Arrow instead of JSON null"
        );
    }

    #[tokio::test]
    async fn cached_point_plan_binds_rls_principal_per_execution() {
        let app_context = crate::test_helpers::test_app_context_simple();
        let mut table = TableDefinition::new(
            SchemaId::new("security"),
            TableName::new("cached_documents"),
            TableType::Shared,
            vec![
                ColumnDefinition::primary_key(1, "id", 1, KalamDataType::Text),
                ColumnDefinition::simple(2, "owner_id", 2, KalamDataType::Text),
            ],
            TableOptions::shared(),
            None,
        )
        .unwrap();
        app_context.system_columns_service().add_system_columns(&mut table).unwrap();
        app_context.schema_registry().register_table(table).unwrap();
        let table_id = TableId::from_strings("security", "cached_documents");
        let provider = app_context.schema_registry().get_provider(&table_id).unwrap();
        let provider = (provider.as_ref() as &dyn std::any::Any)
            .downcast_ref::<SharedTableProvider>()
            .unwrap();
        provider
            .insert_rows(
                &UserId::new("system"),
                vec![
                    Row::from_vec(vec![
                        ("id".to_string(), ScalarValue::Utf8(Some("doc-a".to_string()))),
                        ("owner_id".to_string(), ScalarValue::Utf8(Some("alice".to_string()))),
                    ]),
                    Row::from_vec(vec![
                        ("id".to_string(), ScalarValue::Utf8(Some("doc-b".to_string()))),
                        ("owner_id".to_string(), ScalarValue::Utf8(Some("bob".to_string()))),
                    ]),
                ],
                &[
                    kalamdb_commons::ids::VersionId::from(1_i64),
                    kalamdb_commons::ids::VersionId::from(2_i64),
                ],
            )
            .await
            .unwrap();

        app_context
            .system_tables()
            .table_policies()
            .create_policy(TablePolicy::new(
                PolicyId::new(table_id.clone(), "owner_read").unwrap(),
                table_id,
                "owner_read",
                PolicyCommand::Select,
                vec![PolicyTarget::Role(Role::User)],
                Some("owner_id = CURRENT_USER".to_string()),
                None,
                Some(PolicyProgram::RowLocal {
                    expr: BoundExprShape::ColumnEqualsPrincipal {
                        column_id: 2,
                        principal: PrincipalExpr::CurrentUser,
                    },
                }),
                None,
                0,
                1,
            ))
            .await
            .unwrap();

        let executor = SqlExecutor::new(app_context.clone(), Arc::new(HandlerRegistry::new()));
        let alice = ExecutionContext::new(
            UserId::new("alice"),
            Role::User,
            app_context.base_session_context(),
        );
        let bob = ExecutionContext::new(
            UserId::new("bob"),
            Role::User,
            app_context.base_session_context(),
        );
        let sql = "SELECT id FROM security.cached_documents WHERE id = $1";

        assert_eq!(
            result_row_count(
                executor
                    .execute(sql, &alice, vec![ScalarValue::Utf8(Some("doc-a".to_string()))])
                    .await
                    .unwrap()
            ),
            1
        );
        assert_eq!(
            result_row_count(
                executor
                    .execute(sql, &bob, vec![ScalarValue::Utf8(Some("doc-a".to_string()))])
                    .await
                    .unwrap()
            ),
            0
        );
        assert_eq!(
            result_row_count(
                executor
                    .execute(sql, &bob, vec![ScalarValue::Utf8(Some("doc-b".to_string()))])
                    .await
                    .unwrap()
            ),
            1
        );
        assert_eq!(
            result_row_count(
                executor
                    .execute(sql, &alice, vec![ScalarValue::Utf8(Some("doc-b".to_string()))])
                    .await
                    .unwrap()
            ),
            0
        );
        assert_eq!(executor.plan_cache_len(), 1);
        assert!(executor.point_get_fast_path_hits() >= 3);
    }

    #[tokio::test]
    async fn postgres_set_search_path_succeeds_without_a_ddl_handler() {
        let app_context = crate::test_helpers::test_app_context_simple();
        let executor = SqlExecutor::new(app_context.clone(), Arc::new(HandlerRegistry::new()));
        let ctx = ExecutionContext::new(
            UserId::from("tabularis"),
            Role::User,
            app_context.base_session_context(),
        );

        let result = executor
            .execute("SET search_path TO public", &ctx, Vec::new())
            .await
            .expect("Tabularis SET search_path must not require a registered DDL handler");
        match result {
            ExecutionResult::Success { message } => assert_eq!(message, "SET"),
            other => panic!("expected SET success, got {other:?}"),
        }

        let reset = executor
            .execute("RESET search_path", &ctx, Vec::new())
            .await
            .expect("RESET search_path should succeed");
        assert!(matches!(reset, ExecutionResult::Success { .. }));
    }
}
