# Production Tests (backend/tests/misc/production)

## Standard Steps
1. Arrange production-like workload and dataset.
2. Execute the operation described by the test name.
3. Assert correctness, error messages, and performance invariants.

## Tests
- test_create_table_without_pk_uses_stable_row_id — [backend/tests/misc/production/test_mvcc_phase2.rs](backend/tests/misc/production/test_mvcc_phase2.rs#L19)
- test_create_table_auto_adds_system_columns — [backend/tests/misc/production/test_mvcc_phase2.rs](backend/tests/misc/production/test_mvcc_phase2.rs#L72)
- test_insert_storage_key_format — [backend/tests/misc/production/test_mvcc_phase2.rs](backend/tests/misc/production/test_mvcc_phase2.rs#L148)
- test_user_table_row_structure — [backend/tests/misc/production/test_mvcc_phase2.rs](backend/tests/misc/production/test_mvcc_phase2.rs#L254)
- test_shared_table_row_structure — [backend/tests/misc/production/test_mvcc_phase2.rs](backend/tests/misc/production/test_mvcc_phase2.rs#L329)
- test_insert_duplicate_pk_rejected — [backend/tests/misc/production/test_mvcc_phase2.rs](backend/tests/misc/production/test_mvcc_phase2.rs#L416)
- test_incremental_sync_seq_threshold — [backend/tests/misc/production/test_mvcc_phase2.rs](backend/tests/misc/production/test_mvcc_phase2.rs#L508)
- test_rocksdb_prefix_scan_user_isolation — [backend/tests/misc/production/test_mvcc_phase2.rs](backend/tests/misc/production/test_mvcc_phase2.rs#L602)
- test_rocksdb_range_scan_efficiency — [backend/tests/misc/production/test_mvcc_phase2.rs](backend/tests/misc/production/test_mvcc_phase2.rs#L680)
- concurrent_inserts_same_user_table — [backend/tests/misc/production/test_production_concurrency.rs](backend/tests/misc/production/test_production_concurrency.rs#L13)
- concurrent_select_queries — [backend/tests/misc/production/test_production_concurrency.rs](backend/tests/misc/production/test_production_concurrency.rs#L81)
- concurrent_duplicate_primary_key_handling — [backend/tests/misc/production/test_production_concurrency.rs](backend/tests/misc/production/test_production_concurrency.rs#L146)
- concurrent_updates_same_row — [backend/tests/misc/production/test_production_concurrency.rs](backend/tests/misc/production/test_production_concurrency.rs#L213)
- concurrent_deletes — [backend/tests/misc/production/test_production_concurrency.rs](backend/tests/misc/production/test_production_concurrency.rs#L283)
- syntax_error_messages_are_clear — [backend/tests/misc/production/test_production_validation.rs](backend/tests/misc/production/test_production_validation.rs#L12)
- table_not_found_error_is_clear — [backend/tests/misc/production/test_production_validation.rs](backend/tests/misc/production/test_production_validation.rs#L36)
- invalid_namespace_name_rejected — [backend/tests/misc/production/test_production_validation.rs](backend/tests/misc/production/test_production_validation.rs#L58)
- table_without_primary_key_rejected — [backend/tests/misc/production/test_production_validation.rs](backend/tests/misc/production/test_production_validation.rs#L90)
- null_constraint_violation_detected — [backend/tests/misc/production/test_production_validation.rs](backend/tests/misc/production/test_production_validation.rs#L122)
- flush_on_stream_table_rejected — [backend/tests/misc/production/test_production_validation.rs](backend/tests/misc/production/test_production_validation.rs#L167)
- user_isolation_in_user_tables — [backend/tests/misc/production/test_production_validation.rs](backend/tests/misc/production/test_production_validation.rs#L202)
- duplicate_primary_key_rejected — [backend/tests/misc/production/test_production_validation.rs](backend/tests/misc/production/test_production_validation.rs#L254)
- drop_nonexistent_table_error_is_clear — [backend/tests/misc/production/test_production_validation.rs](backend/tests/misc/production/test_production_validation.rs#L309)
- invalid_data_type_in_insert_rejected — [backend/tests/misc/production/test_production_validation.rs](backend/tests/misc/production/test_production_validation.rs#L330)
- select_invalid_column_error_is_clear — [backend/tests/misc/production/test_production_validation.rs](backend/tests/misc/production/test_production_validation.rs#L369)
- permission_denied_error_is_clear — [backend/tests/misc/production/test_production_validation.rs](backend/tests/misc/production/test_production_validation.rs#L413)
