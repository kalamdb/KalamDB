# Backend Cargo Audit - 2026-05-24

Scope: backend-focused dependency cleanup and CRAP hotspot review.

Commands used:

```bash
cargo machete --with-metadata --skip-target-dir
cargo crap --path backend --summary
cargo crap --path backend --format markdown --top 100
```

Caveats:

- `cargo-machete` is intentionally fast and imprecise. Treat every finding as "verify before remove," especially for feature-gated, test-only, build-only, proc-macro, and reflection-like usage.
- `cargo-crap` was run without LCOV coverage input, so coverage is treated as 0%. That makes this a complexity-first prioritization pass, not a final change-risk score.
- `cargo-crap --path backend` scoped the CRAP scan to `backend/` only.
- `cargo-machete` had to run from the repo root because it operates on the workspace layout from the current directory. The findings below are filtered to backend crates only.

## Summary

Backend CRAP summary:

- Analyzed functions: 6764
- Functions above threshold 30: 935
- Worst offender: `SqlStatement::classify_and_parse` in `backend/crates/kalamdb-dialect/src/classifier/engine/core.rs:43` with CRAP 14280.0

Top backend crates by crappy-function count from the workspace summary pass:

| Crate | Crappy functions |
|---|---:|
| `kalamdb-server` | 189 |
| `kalamdb-core` | 188 |
| `kalamdb-commons` | 172 |
| `kalamdb-dialect` | 154 |
| `kalamdb-tables` | 146 |
| `kalamdb-raft` | 94 |
| `kalamdb-api` | 92 |
| `kalamdb-jobs` | 80 |
| `kalamdb-system` | 76 |
| `kalamdb-handlers-ddl` | 54 |
| `kalamdb-store` | 48 |
| `kalamdb-flush` | 40 |
| `kalamdb-filestore` | 36 |
| `kalamdb-auth` | 34 |
| `kalamdb-live` | 34 |

Suggested fix order:

1. `kalamdb-dialect`: parser/classifier complexity is the clearest hotspot concentration.
2. `kalamdb-configs` and `kalamdb-raft`: large single-function hotspots with obvious extraction seams.
3. `kalamdb-commons`: scalar conversion and enum/string conversion clusters.
4. `kalamdb-core`, `kalamdb-api`, and `kalamdb-jobs`: DML/SQL execution paths.
5. `kalamdb-tables`, `kalamdb-system`, and `kalamdb-store`: storage/planner/provider hot paths.
6. Backend tests last, after production paths stop dominating the list.

## Cargo-Machete Findings

These are unconfirmed unused-dependency findings from `cargo machete --with-metadata --skip-target-dir`.

| Crate manifest | Flagged dependencies |
|---|---|
| `backend/Cargo.toml` | `arrow`, `cc`, `kalamdb-sharding`, `kalamdb-sql` |
| `backend/crates/kalamdb-api/Cargo.toml` | `cc`, `tempfile` |
| `backend/crates/kalamdb-store/Cargo.toml` | `criterion` |
| `backend/crates/kalamdb-live/Cargo.toml` | `serde` |
| `backend/crates/kalamdb-system/Cargo.toml` | `tempfile`, `tracing` |
| `backend/crates/kalamdb-raft/Cargo.toml` | `tempfile` |
| `backend/crates/kalamdb-handlers/crates/admin/Cargo.toml` | `datafusion` |
| `backend/crates/kalamdb-handlers/crates/user/Cargo.toml` | `log` |
| `backend/crates/kalamdb-handlers/crates/support/Cargo.toml` | `kalamdb-filestore` |
| `backend/crates/kalamdb-dialect/Cargo.toml` | `tempfile` |
| `backend/crates/kalamdb-session/Cargo.toml` | `tokio` |
| `backend/crates/kalamdb-observability/Cargo.toml` | `cc`, `chrono` |
| `backend/crates/kalamdb-datafusion-sources/Cargo.toml` | `datafusion-common`, `datafusion-expr`, `parquet`, `thiserror`, `tokio` |
| `backend/crates/kalamdb-core/Cargo.toml` | `kalamdb-publisher`, `parquet`, `regex`, `tempfile`, `tokio-util`, `zip` |
| `backend/crates/kalamdb-publisher/Cargo.toml` | `tokio` |
| `backend/crates/kalamdb-filestore/Cargo.toml` | `once_cell`, `tempfile` |
| `backend/crates/kalamdb-flush/Cargo.toml` | `parquet`, `tracing` |
| `backend/crates/kalamdb-dba/Cargo.toml` | `serde_json` |
| `backend/crates/kalamdb-session-datafusion/Cargo.toml` | `tokio` |

Verification notes before removing anything:

- Check `build.rs`, `tests/`, benches, examples, and feature-gated modules before deleting a flagged dependency.
- Prefer confirming with `rg` or the compiler before editing manifests.
- If a dependency is intentionally retained but `cargo-machete` cannot see the usage, add it to `[package.metadata.cargo-machete].ignored` instead of repeatedly rediscovering it.

## Detailed CRAP Hotspots

Top 100 backend hotspots from `cargo crap --path backend --format markdown --top 100`:

| Flag | CRAP | CC | Cov % | Function | Location |
|---|---:|---:|---:|---|---|
| x | 14280.0 | 119 | n/a | `SqlStatement::classify_and_parse` | `backend/crates/kalamdb-dialect/src/classifier/engine/core.rs:43` |
| x | 13572.0 | 116 | n/a | `test_flush_concurrency_and_correctness_over_http` | `backend/tests/testserver/flush/test_flush_unregistered_suite_http.rs:148` |
| x | 7482.0 | 86 | n/a | `CreateTableStatement::parse` | `backend/crates/kalamdb-dialect/src/ddl/create_table/parser.rs:33` |
| x | 6642.0 | 81 | n/a | `ServerConfig::apply_env_overrides` | `backend/crates/kalamdb-configs/src/config/override.rs:72` |
| x | 5402.0 | 73 | n/a | `MetaStateMachine::apply_command` | `backend/crates/kalamdb-raft/src/state_machine/meta.rs:141` |
| x | 4422.0 | 66 | n/a | `scalar_value_to_json` | `backend/crates/kalamdb-commons/src/conversions/arrow_json_conversion.rs:1006` |
| x | 3540.0 | 59 | n/a | `SqlStatement::name` | `backend/crates/kalamdb-dialect/src/classifier/types.rs:318` |
| x | 3422.0 | 58 | n/a | `decode_scalar_payload` | `backend/crates/kalamdb-commons/src/serialization/row_codec.rs:496` |
| x | 2862.0 | 53 | n/a | `ExtensionStatement::parse` | `backend/crates/kalamdb-dialect/src/parser/extensions.rs:151` |
| x | 2070.0 | 45 | n/a | `Predicate::compile` | `backend/crates/kalamdb-row-filter/src/predicate.rs:73` |
| x | 1892.0 | 43 | n/a | `execute_batch_path` | `backend/crates/kalamdb-api/src/http/sql/execution_paths.rs:586` |
| x | 1722.0 | 41 | n/a | `estimate_scalar_value_size` | `backend/crates/kalamdb-commons/src/conversions/scalar_size.rs:31` |
| x | 1640.0 | 40 | n/a | `Predicate::evaluate` | `backend/crates/kalamdb-row-filter/src/predicate.rs:254` |
| x | 1560.0 | 39 | n/a | `UserExportExecutor::execute` | `backend/crates/kalamdb-jobs/src/executors/user_export.rs:94` |
| x | 1560.0 | 39 | n/a | `JobsManager::execute_job` | `backend/crates/kalamdb-jobs/src/jobs_manager/runner.rs:703` |
| x | 1560.0 | 39 | n/a | `SqlExecutor::execute_via_datafusion` | `backend/crates/kalamdb-core/src/sql/executor/sql_executor.rs:1066` |
| x | 1560.0 | 39 | n/a | `test_scenario_14_rag_docs_with_files_and_vector_search` | `backend/tests/scenarios/scenario_14_vector_rag.rs:174` |
| x | 1482.0 | 38 | n/a | `RaftExecutor::get_cluster_info` | `backend/crates/kalamdb-raft/src/executor/raft.rs:218` |
| x | 1482.0 | 38 | n/a | `apply_alter_operation` | `backend/crates/kalamdb-handlers/crates/ddl/src/table/alter.rs:581` |
| x | 1482.0 | 38 | n/a | `split_statements` | `backend/crates/kalamdb-dialect/src/batch_execution.rs:325` |
| x | 1482.0 | 38 | n/a | `OAuthProvider::detect_from_issuer` | `backend/crates/kalamdb-commons/src/models/oauth_provider.rs:250` |
| x | 1406.0 | 37 | n/a | `detect_mime_type` | `backend/crates/kalamdb-filestore/src/files/staging.rs:206` |
| x | 1406.0 | 37 | n/a | `test_flush_policy_and_parquet_output_over_http` | `backend/tests/testserver/flush/test_flush_policy_verification_http.rs:102` |
| x | 1406.0 | 37 | n/a | `test_scenario_01_chat_app_core` | `backend/tests/scenarios/scenario_01_chat_app.rs:35` |
| x | 1332.0 | 36 | n/a | `CoreClusterHandler::handle_forward_sql` | `backend/crates/kalamdb-core/src/cluster_handler.rs:256` |
| x | 1332.0 | 36 | n/a | `try_batch_inserts_in_transaction` | `backend/crates/kalamdb-core/src/sql/executor/transaction_batch_insert.rs:345` |
| x | 1332.0 | 36 | n/a | `test_flush_realtime_soak_preserves_all_rows_and_updates_over_http` | `backend/tests/testserver/flush/test_flush_resilience_http.rs:1025` |
| x | 1332.0 | 36 | n/a | `test_chat_app_endurance_with_100_parallel_users` | `backend/tests/endurance_test.rs:239` |
| x | 1190.0 | 34 | n/a | `test_user_writes_queries_flush_jobs_and_live_subscription_overlap_cleanly_over_http` | `backend/tests/testserver/flush/test_flush_resilience_http.rs:544` |
| x | 1190.0 | 34 | n/a | `test_flush_batch_and_transaction_variations_preserve_exact_rows_over_http` | `backend/tests/testserver/flush/test_flush_resilience_http.rs:812` |
| x | 1122.0 | 33 | n/a | `test_scenario_13_mixed_workload_soak` | `backend/tests/scenarios/scenario_13_soak_test.rs:30` |
| x | 1056.0 | 32 | n/a | `parse_multipart_request` | `backend/crates/kalamdb-api/src/http/sql/file_utils.rs:82` |
| x | 1056.0 | 32 | n/a | `ColumnArgs::parse` | `backend/crates/kalamdb-macros/src/lib.rs:450` |
| x | 1056.0 | 32 | n/a | `encode_scalar_payload` | `backend/crates/kalamdb-commons/src/serialization/row_codec.rs:354` |
| x | 1056.0 | 32 | n/a | `OAuthProvider::as_str` | `backend/crates/kalamdb-commons/src/models/oauth_provider.rs:85` |
| x | 1056.0 | 32 | n/a | `OAuthProvider::prefix` | `backend/crates/kalamdb-commons/src/models/oauth_provider.rs:126` |
| x | 1056.0 | 32 | n/a | `OAuthProvider::from_prefix` | `backend/crates/kalamdb-commons/src/models/oauth_provider.rs:172` |
| x | 1056.0 | 32 | n/a | `OAuthProvider::from_str_lossy` | `backend/crates/kalamdb-commons/src/models/oauth_provider.rs:210` |
| x | 992.0 | 31 | n/a | `KalamDataType::from_arrow_type` | `backend/crates/kalamdb-commons/src/conversions/arrow_conversion.rs:103` |
| x | 992.0 | 31 | n/a | `test_pk_uniqueness_hot_and_cold_over_http` | `backend/tests/testserver/flush/test_pk_uniqueness_hot_cold_http.rs:63` |
| x | 870.0 | 29 | n/a | `execute_file_upload_path` | `backend/crates/kalamdb-api/src/http/sql/execution_paths.rs:368` |
| x | 870.0 | 29 | n/a | `search_hot_candidates` | `backend/crates/kalamdb-vector/src/hot_query_cache.rs:174` |
| x | 870.0 | 29 | n/a | `DmlExecutor::apply_user_transaction_batch_with_commit_seq` | `backend/crates/kalamdb-core/src/applier/executor/dml.rs:765` |
| x | 870.0 | 29 | n/a | `json_value_to_scalar_for_column` | `backend/crates/kalamdb-commons/src/conversions/scalar_json.rs:6` |
| x | 812.0 | 28 | n/a | `AlterUserHandler::execute` | `backend/crates/kalamdb-handlers/crates/user/src/user/alter.rs:36` |
| x | 812.0 | 28 | n/a | `build_table_definition` | `backend/crates/kalamdb-handlers/crates/support/src/table_creation.rs:200` |
| x | 812.0 | 28 | n/a | `arrow_value_to_scalar` | `backend/crates/kalamdb-commons/src/conversions/arrow_json_conversion.rs:815` |
| x | 812.0 | 28 | n/a | `StoredScalarValue::from` | `backend/crates/kalamdb-commons/src/models/rows/row.rs:138` |
| x | 812.0 | 28 | n/a | `test_user_tables_lifecycle_and_isolation_over_http` | `backend/tests/testserver/tables/test_user_tables_http.rs:37` |
| x | 812.0 | 28 | n/a | `test_scenario_09_ddl_while_active` | `backend/tests/scenarios/scenario_09_ddl_while_active.rs:23` |
| x | 812.0 | 28 | n/a | `test_scenario_07_collaborative_editing` | `backend/tests/scenarios/scenario_07_collaborative.rs:30` |
| x | 756.0 | 27 | n/a | `execute_sql_v1` | `backend/crates/kalamdb-api/src/http/sql/execute.rs:114` |
| x | 756.0 | 27 | n/a | `JobsManager::run_loop` | `backend/crates/kalamdb-jobs/src/jobs_manager/runner.rs:208` |
| x | 756.0 | 27 | n/a | `DmlExecutor::apply_shared_transaction_batch_with_commit_seq` | `backend/crates/kalamdb-core/src/applier/executor/dml.rs:977` |
| x | 756.0 | 27 | n/a | `try_build_literal_insert_rows` | `backend/crates/kalamdb-core/src/sql/executor/transaction_batch_insert.rs:167` |
| x | 756.0 | 27 | n/a | `ScalarTag::variant_name` | `backend/crates/kalamdb-commons/src/serialization/generated/row_models_generated.rs:121` |
| x | 702.0 | 26 | n/a | `ServerConfig::validate` | `backend/crates/kalamdb-configs/src/config/loader.rs:105` |
| x | 702.0 | 26 | n/a | `map_sql_type_to_arrow` | `backend/crates/kalamdb-dialect/src/compatibility.rs:15` |
| x | 702.0 | 26 | n/a | `SqlExecutor::execute_with_metadata` | `backend/crates/kalamdb-core/src/sql/executor/sql_executor.rs:665` |
| x | 702.0 | 26 | n/a | `to_scalar_tag` | `backend/crates/kalamdb-commons/src/serialization/row_codec.rs:324` |
| x | 702.0 | 26 | n/a | `ScalarValue::from` | `backend/crates/kalamdb-commons/src/models/rows/row.rs:189` |
| x | 702.0 | 26 | n/a | `test_automatic_user_flush_waits_for_row_limit_and_writes_only_user_files_over_http` | `backend/tests/testserver/flush/test_flush_policy_verification_http.rs:478` |
| x | 702.0 | 26 | n/a | `test_parameterized_dml_over_http` | `backend/tests/testserver/sql/test_dml_parameters_http.rs:47` |
| x | 702.0 | 26 | n/a | `test_scenario_05_dashboards_shared_reference` | `backend/tests/scenarios/scenario_05_dashboards.rs:28` |
| x | 650.0 | 25 | n/a | `DropTableHandler::execute` | `backend/crates/kalamdb-handlers/crates/ddl/src/table/drop.rs:131` |
| x | 650.0 | 25 | n/a | `QueryParser::validate_subscription_select` | `backend/crates/kalamdb-dialect/src/parser/query_parser.rs:370` |
| x | 650.0 | 25 | n/a | `authenticate_user_password` | `backend/crates/kalamdb-auth/src/services/unified/password.rs:16` |
| x | 650.0 | 25 | n/a | `StorageHealthService::run_full_health_check` | `backend/crates/kalamdb-filestore/src/health/service.rs:69` |
| x | 650.0 | 25 | n/a | `parse_string_as_scalar` | `backend/crates/kalamdb-commons/src/conversions/scalar_string.rs:37` |
| x | 650.0 | 25 | n/a | `SystemTable::from_name` | `backend/crates/kalamdb-commons/src/system_tables.rs:178` |
| x | 650.0 | 25 | n/a | `test_user_file_access_matrix` | `backend/tests/testserver/files/test_file_permissions_http.rs:350` |
| x | 600.0 | 24 | n/a | `handle_subscribe` | `backend/crates/kalamdb-api/src/ws/events/subscription.rs:25` |
| x | 600.0 | 24 | n/a | `JobsTableProvider::list_jobs_filtered` | `backend/crates/kalamdb-system/src/providers/jobs/jobs_provider.rs:181` |
| x | 600.0 | 24 | n/a | `RaftManager::initialize_cluster` | `backend/crates/kalamdb-raft/src/manager/raft_manager.rs:451` |
| x | 600.0 | 24 | n/a | `CreateUserHandler::execute` | `backend/crates/kalamdb-handlers/crates/user/src/user/create.rs:37` |
| x | 600.0 | 24 | n/a | `CreateUserStatement::parse_tokens` | `backend/crates/kalamdb-dialect/src/ddl/user_commands.rs:123` |
| x | 600.0 | 24 | n/a | `VectorSearchScanSource::produce_batch` | `backend/crates/kalamdb-vector/src/sql/vector_search.rs:170` |
| x | 600.0 | 24 | n/a | `ManifestAccessPlanner::scan_parquet_files_async` | `backend/crates/kalamdb-tables/src/manifest/planner.rs:99` |
| x | 600.0 | 24 | n/a | `PkExistenceChecker::check_cold_storage` | `backend/crates/kalamdb-tables/src/utils/pk/existence_checker.rs:127` |
| x | 600.0 | 24 | n/a | `pk_exists_batch_in_cold` | `backend/crates/kalamdb-tables/src/utils/base.rs:1414` |
| x | 600.0 | 24 | n/a | `extract_seq_bounds_from_filter` | `backend/crates/kalamdb-tables/src/utils/row_utils.rs:71` |
| x | 600.0 | 24 | n/a | `collect_replaced_file_refs_for_update` | `backend/crates/kalamdb-core/src/applier/executor/utils/fileref_util.rs:55` |
| x | 600.0 | 24 | n/a | `SystemTable::table_name` | `backend/crates/kalamdb-commons/src/system_tables.rs:88` |
| x | 600.0 | 24 | n/a | `ScalarValuePayload::run_verifier` | `backend/crates/kalamdb-commons/src/serialization/generated/row_models_generated.rs:522` |
| x | 600.0 | 24 | n/a | `HttpTestServer::execute_sql_with_auth_and_params` | `backend/tests/common/testserver/http_server.rs:487` |
| x | 552.0 | 23 | n/a | `server_setup_handler` | `backend/crates/kalamdb-api/src/http/auth/setup.rs:55` |
| x | 552.0 | 23 | n/a | `JobsTableProvider::list_jobs_filtered_async` | `backend/crates/kalamdb-system/src/providers/jobs/jobs_provider.rs:271` |
| x | 552.0 | 23 | n/a | `RaftManager::wait_for_peer_online` | `backend/crates/kalamdb-raft/src/manager/raft_manager.rs:689` |
| x | 552.0 | 23 | n/a | `expr_to_column_default` | `backend/crates/kalamdb-dialect/src/ddl/alter_table.rs:405` |
| x | 552.0 | 23 | n/a | `SharedTableProvider::persist_insert_batch_rows` | `backend/crates/kalamdb-tables/src/shared_tables/shared_table_provider.rs:1499` |
| x | 552.0 | 23 | n/a | `SqlExecutor::execute_dml_via_datafusion_inner` | `backend/crates/kalamdb-core/src/sql/executor/sql_executor.rs:977` |
| x | 552.0 | 23 | n/a | `test_scenario_10_multi_tenant_isolation` | `backend/tests/scenarios/scenario_10_multi_tenant.rs:20` |
| x | 506.0 | 22 | n/a | `download_file` | `backend/crates/kalamdb-api/src/http/files/download.rs:27` |
| x | 506.0 | 22 | n/a | `ErrorCode::as_str` | `backend/crates/kalamdb-api/src/http/sql/models/sql_response.rs:80` |
| x | 506.0 | 22 | n/a | `ErrorCode::from` | `backend/crates/kalamdb-api/src/http/sql/models/sql_response.rs:132` |
| x | 506.0 | 22 | n/a | `LiveQueryManager::register_subscription_with_initial_data` | `backend/crates/kalamdb-live/src/manager/queries_manager.rs:203` |
| x | 506.0 | 22 | n/a | `RaftManager::shutdown` | `backend/crates/kalamdb-raft/src/manager/raft_manager.rs:1961` |
| x | 506.0 | 22 | n/a | `pk_exists_in_cold` | `backend/crates/kalamdb-tables/src/utils/base.rs:1207` |
| x | 506.0 | 22 | n/a | `UserTableProvider::update_by_pk_value` | `backend/crates/kalamdb-tables/src/user_tables/user_table_provider.rs:1104` |
| x | 506.0 | 22 | n/a | `scalar_to_json_for_column` | `backend/crates/kalamdb-commons/src/conversions/scalar_json.rs:108` |

This embedded table is the top 100 only. The full backend pass reported 935 functions above the threshold, so use the command at the top of this note to regenerate a larger or exhaustive markdown artifact when you start grinding through a specific crate.