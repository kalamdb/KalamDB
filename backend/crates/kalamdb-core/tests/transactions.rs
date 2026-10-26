//! Linked together so CI does not link the full engine once per source file.

#[path = "support/mod.rs"]
mod support;

#[path = "snapshot_isolation.rs"]
mod snapshot_isolation;
#[path = "sql_insert_transaction_semantics.rs"]
mod sql_insert_transaction_semantics;
#[path = "sql_transaction_commit.rs"]
mod sql_transaction_commit;
#[path = "sql_transaction_multi_block.rs"]
mod sql_transaction_multi_block;
#[path = "sql_transaction_nested_begin.rs"]
mod sql_transaction_nested_begin;
#[path = "sql_transaction_rollback.rs"]
mod sql_transaction_rollback;
#[path = "sql_transaction_unclosed.rs"]
mod sql_transaction_unclosed;
#[path = "system_transactions_view.rs"]
mod system_transactions_view;
#[path = "transaction_buffer_limit.rs"]
mod transaction_buffer_limit;
#[path = "transaction_cluster_failover.rs"]
mod transaction_cluster_failover;
#[path = "transaction_cluster_group_rejection.rs"]
mod transaction_cluster_group_rejection;
#[path = "transaction_commit_live_fanout.rs"]
mod transaction_commit_live_fanout;
#[path = "transaction_ddl_rejection.rs"]
mod transaction_ddl_rejection;
#[path = "transaction_races.rs"]
mod transaction_races;
#[path = "transaction_stream_table_rejection.rs"]
mod transaction_stream_table_rejection;
#[path = "transaction_timeout.rs"]
mod transaction_timeout;
#[path = "transaction_user_batch_commit.rs"]
mod transaction_user_batch_commit;
