# Scenarios Tests (backend/tests/scenarios)

## Runnable Test Targets

- `test_scenarios` — aggregate driver for every scenario below
- `test_scenarios_realtime` — scenarios 01, 02, 03, 07
- `test_scenarios_lifecycle` — scenarios 05, 06, 09, 10, 11, 14
- `test_scenarios_scale` — scenarios 04, 08, 12, 13

## Standard Steps
1. Arrange scenario fixtures and initial data.
2. Execute the workflow described by the test name.
3. Assert end-to-end invariants, correctness, and artifacts.

## Tests
- test_scenario_01_chat_app_core — [backend/tests/scenarios/scenario_01_chat_app.rs](backend/tests/scenarios/scenario_01_chat_app.rs#L33)
- test_scenario_01_service_writes_as_user — [backend/tests/scenarios/scenario_01_chat_app.rs](backend/tests/scenarios/scenario_01_chat_app.rs#L288)
- test_scenario_01_stream_table_ttl — [backend/tests/scenarios/scenario_01_chat_app.rs](backend/tests/scenarios/scenario_01_chat_app.rs#L353)
- test_scenario_02_offline_sync_parallel — [backend/tests/scenarios/scenario_02_offline_sync.rs](backend/tests/scenarios/scenario_02_offline_sync.rs#L32)
- test_scenario_02_offline_drift_resume — [backend/tests/scenarios/scenario_02_offline_sync.rs](backend/tests/scenarios/scenario_02_offline_sync.rs#L210)
- test_scenario_02_changes_during_snapshot — [backend/tests/scenarios/scenario_02_offline_sync.rs](backend/tests/scenarios/scenario_02_offline_sync.rs#L296)
- test_scenario_03_shopping_cart_parallel — [backend/tests/scenarios/scenario_03_shopping_cart.rs](backend/tests/scenarios/scenario_03_shopping_cart.rs#L34)
- test_scenario_03_filtered_subscription — [backend/tests/scenarios/scenario_03_shopping_cart.rs](backend/tests/scenarios/scenario_03_shopping_cart.rs#L259)
- test_scenario_03_partial_flush — [backend/tests/scenarios/scenario_03_shopping_cart.rs](backend/tests/scenarios/scenario_03_shopping_cart.rs#L359)
- test_scenario_04_iot_telemetry_5k_rows — [backend/tests/scenarios/scenario_04_iot_telemetry.rs](backend/tests/scenarios/scenario_04_iot_telemetry.rs#L26)
- test_scenario_04_anomaly_subscription — [backend/tests/scenarios/scenario_04_iot_telemetry.rs](backend/tests/scenarios/scenario_04_iot_telemetry.rs#L198)
- test_scenario_04_wide_column_scan — [backend/tests/scenarios/scenario_04_iot_telemetry.rs](backend/tests/scenarios/scenario_04_iot_telemetry.rs#L289)
- test_scenario_05_dashboards_shared_reference — [backend/tests/scenarios/scenario_05_dashboards.rs](backend/tests/scenarios/scenario_05_dashboards.rs#L27)
- test_scenario_05_rbac_restrictions — [backend/tests/scenarios/scenario_05_dashboards.rs](backend/tests/scenarios/scenario_05_dashboards.rs#L235)
- test_scenario_05_schema_evolution — [backend/tests/scenarios/scenario_05_dashboards.rs](backend/tests/scenarios/scenario_05_dashboards.rs#L282)
- test_scenario_06_jobs_lifecycle — [backend/tests/scenarios/scenario_06_jobs.rs](backend/tests/scenarios/scenario_06_jobs.rs#L23)
- test_scenario_06_job_idempotency — [backend/tests/scenarios/scenario_06_jobs.rs](backend/tests/scenarios/scenario_06_jobs.rs#L135)
- test_scenario_06_system_jobs_query — [backend/tests/scenarios/scenario_06_jobs.rs](backend/tests/scenarios/scenario_06_jobs.rs#L194)
- test_scenario_06_job_status_transitions — [backend/tests/scenarios/scenario_06_jobs.rs](backend/tests/scenarios/scenario_06_jobs.rs#L225)
- test_scenario_07_collaborative_editing — [backend/tests/scenarios/scenario_07_collaborative.rs](backend/tests/scenarios/scenario_07_collaborative.rs#L29)
- test_scenario_07_presence_subscription — [backend/tests/scenarios/scenario_07_collaborative.rs](backend/tests/scenarios/scenario_07_collaborative.rs#L271)
- test_scenario_08_burst_writes — [backend/tests/scenarios/scenario_08_burst.rs](backend/tests/scenarios/scenario_08_burst.rs#L24)
- test_scenario_08_sustained_load — [backend/tests/scenarios/scenario_08_burst.rs](backend/tests/scenarios/scenario_08_burst.rs#L184)
- test_scenario_08_subscription_reconnect — [backend/tests/scenarios/scenario_08_burst.rs](backend/tests/scenarios/scenario_08_burst.rs#L275)
- test_scenario_09_ddl_while_active — [backend/tests/scenarios/scenario_09_ddl_while_active.rs](backend/tests/scenarios/scenario_09_ddl_while_active.rs#L23)
- test_scenario_09_drop_column — [backend/tests/scenarios/scenario_09_ddl_while_active.rs](backend/tests/scenarios/scenario_09_ddl_while_active.rs#L187)
- test_scenario_09_concurrent_reads_during_ddl — [backend/tests/scenarios/scenario_09_ddl_while_active.rs](backend/tests/scenarios/scenario_09_ddl_while_active.rs#L268)
- test_scenario_10_multi_tenant_isolation — [backend/tests/scenarios/scenario_10_multi_tenant.rs](backend/tests/scenarios/scenario_10_multi_tenant.rs#L19)
- test_scenario_10_subscription_namespace_isolation — [backend/tests/scenarios/scenario_10_multi_tenant.rs](backend/tests/scenarios/scenario_10_multi_tenant.rs#L231)
- test_scenario_10_same_table_name_different_namespaces — [backend/tests/scenarios/scenario_10_multi_tenant.rs](backend/tests/scenarios/scenario_10_multi_tenant.rs#L322)
- test_scenario_11_multi_storage_basic — [backend/tests/scenarios/scenario_11_multi_storage.rs](backend/tests/scenarios/scenario_11_multi_storage.rs#L20)
- test_scenario_11_storage_constraints — [backend/tests/scenarios/scenario_11_multi_storage.rs](backend/tests/scenarios/scenario_11_multi_storage.rs#L158)
- test_scenario_11_table_types_storage — [backend/tests/scenarios/scenario_11_multi_storage.rs](backend/tests/scenarios/scenario_11_multi_storage.rs#L217)
- test_scenario_12_insert_performance — [backend/tests/scenarios/scenario_12_performance.rs](backend/tests/scenarios/scenario_12_performance.rs#L21)
- test_scenario_12_query_time_growth — [backend/tests/scenarios/scenario_12_performance.rs](backend/tests/scenarios/scenario_12_performance.rs#L106)
- test_scenario_12_subscription_snapshot_timing — [backend/tests/scenarios/scenario_12_performance.rs](backend/tests/scenarios/scenario_12_performance.rs#L198)
- test_scenario_12_memory_baseline — [backend/tests/scenarios/scenario_12_performance.rs](backend/tests/scenarios/scenario_12_performance.rs#L289)
- test_scenario_13_mixed_workload_soak — [backend/tests/scenarios/scenario_13_soak_test.rs](backend/tests/scenarios/scenario_13_soak_test.rs#L25)
- test_scenario_13_schema_evolution_under_load — [backend/tests/scenarios/scenario_13_soak_test.rs](backend/tests/scenarios/scenario_13_soak_test.rs#L331)
- test_scenario_13_concurrent_read_write — [backend/tests/scenarios/scenario_13_soak_test.rs](backend/tests/scenarios/scenario_13_soak_test.rs#L419)
