//! End-to-end gRPC round-trip tests for all PG extension statement types.
//!
//! Each test stands up a tonic server with a mock `OperationExecutor`, connects
//! a `RemoteKalamClient`, and exercises a full round-trip:
//!     client → gRPC → KalamPgService → OperationExecutor → response → client

use std::{sync::Arc, time::Duration};

use arrow::{
    array::{Array, Int64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use async_trait::async_trait;
use kalam_pg_client::RemoteKalamClient;
use kalam_pg_common::RemoteServerConfig;
use kalamdb_commons::models::TransactionId;
use kalamdb_pg::{
    DeleteRequest, InsertRequest, KalamPgService, MutationResult, OperationExecutor,
    PgServiceServer, ScanRequest, ScanResult, UpdateRequest,
};
use tokio::net::TcpListener;
use tonic::{Code, Status};

// ---------------------------------------------------------------------------
// Mock executor
// ---------------------------------------------------------------------------

/// A mock executor that returns predictable results for every operation.
struct MockExecutor;

#[async_trait]
impl OperationExecutor for MockExecutor {
    async fn execute_scan(&self, _request: ScanRequest) -> Result<ScanResult, Status> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])),
                Arc::new(StringArray::from(vec![Some("alice"), Some("bob"), None])),
            ],
        )
        .expect("build batch");
        Ok(ScanResult {
            batches: vec![batch],
        })
    }

    async fn execute_insert(&self, request: InsertRequest) -> Result<MutationResult, Status> {
        Ok(MutationResult {
            affected_rows: request.rows.len() as u64,
        })
    }

    async fn execute_update(&self, _request: UpdateRequest) -> Result<MutationResult, Status> {
        Ok(MutationResult { affected_rows: 1 })
    }

    async fn execute_delete(&self, _request: DeleteRequest) -> Result<MutationResult, Status> {
        Ok(MutationResult { affected_rows: 1 })
    }

    async fn execute_sql(&self, sql: &str) -> Result<String, Status> {
        Ok(format!("executed: {sql}"))
    }

    async fn execute_query(&self, sql: &str) -> Result<(String, Vec<bytes::Bytes>), Status> {
        Ok((format!("executed: {sql}"), Vec::new()))
    }
}

struct NotLeaderExecutor {
    leader_api_addr: String,
    code: Code,
}

impl NotLeaderExecutor {
    fn status(&self) -> Status {
        Status::new(
            self.code,
            format!(
                "External error: Not leader for shard. Leader: Some(\"{}\")",
                self.leader_api_addr
            ),
        )
    }
}

#[async_trait]
impl OperationExecutor for NotLeaderExecutor {
    async fn execute_scan(&self, _request: ScanRequest) -> Result<ScanResult, Status> {
        Err(self.status())
    }

    async fn begin_transaction(&self, _session_id: &str) -> Result<Option<TransactionId>, Status> {
        Err(self.status())
    }

    async fn commit_transaction(
        &self,
        _session_id: &str,
        _transaction_id: &TransactionId,
    ) -> Result<Option<TransactionId>, Status> {
        Err(self.status())
    }

    async fn rollback_transaction(
        &self,
        _session_id: &str,
        _transaction_id: &TransactionId,
    ) -> Result<Option<TransactionId>, Status> {
        Err(self.status())
    }

    async fn execute_insert(&self, _request: InsertRequest) -> Result<MutationResult, Status> {
        Err(self.status())
    }

    async fn execute_update(&self, _request: UpdateRequest) -> Result<MutationResult, Status> {
        Err(self.status())
    }

    async fn execute_delete(&self, _request: DeleteRequest) -> Result<MutationResult, Status> {
        Err(self.status())
    }

    async fn execute_sql(&self, _sql: &str) -> Result<String, Status> {
        Err(self.status())
    }

    async fn execute_query(&self, _sql: &str) -> Result<(String, Vec<bytes::Bytes>), Status> {
        Err(self.status())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn start_server_with_executor_on_ephemeral_port(
    host: &str,
    executor: Arc<dyn OperationExecutor>,
) -> u16 {
    let listener = TcpListener::bind(format!("{host}:0"))
        .await
        .expect("bind ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let service = KalamPgService::new(false, None).with_operation_executor(executor);

    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(PgServiceServer::new(service))
            .serve_with_incoming(incoming)
            .await
            .expect("serve pg grpc");
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    port
}

fn leader_api_port_for_rpc_port(leader_rpc_port: u16) -> u16 {
    leader_rpc_port
        .checked_sub(1000)
        .expect("ephemeral RPC port must support leader API mapping")
}

async fn start_leader_redirect_servers(host: &str, code: Code) -> (u16, u16) {
    let leader_rpc_port =
        start_server_with_executor_on_ephemeral_port(host, Arc::new(MockExecutor)).await;
    let leader_api_port = leader_api_port_for_rpc_port(leader_rpc_port);
    let follower_rpc_port = start_server_with_executor_on_ephemeral_port(
        host,
        Arc::new(NotLeaderExecutor {
            leader_api_addr: format!("http://{host}:{leader_api_port}"),
            code,
        }),
    )
    .await;

    (follower_rpc_port, leader_rpc_port)
}

async fn start_mock_server_and_client() -> RemoteKalamClient {
    let port = start_server_with_executor_on_ephemeral_port("127.0.0.1", Arc::new(MockExecutor))
        .await;
    connect_to("127.0.0.1", port).await
}

async fn connect_to(host: &str, port: u16) -> RemoteKalamClient {
    RemoteKalamClient::connect(RemoteServerConfig {
        host: host.to_string(),
        port,
        ..Default::default()
    })
    .await
    .expect("connect client")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
#[ntest::timeout(10000)]
async fn scan_returns_arrow_batches() {
    let client = start_mock_server_and_client().await;

    let session = client.open_session(None, Some("app")).await.expect("open session");

    let response = client
        .scan("app", "messages", "shared", &session.session_id, None, vec![], None, vec![])
        .await
        .expect("scan");

    assert_eq!(response.batches.len(), 1);
    let batch = &response.batches[0];
    assert_eq!(batch.num_rows(), 3);
    assert_eq!(batch.num_columns(), 2);

    let id_col = batch.column(0).as_any().downcast_ref::<Int64Array>().expect("id column");
    assert_eq!(id_col.value(0), 1);
    assert_eq!(id_col.value(1), 2);
    assert_eq!(id_col.value(2), 3);

    let name_col = batch.column(1).as_any().downcast_ref::<StringArray>().expect("name column");
    assert_eq!(name_col.value(0), "alice");
    assert_eq!(name_col.value(1), "bob");
    assert!(name_col.is_null(2));
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn scan_with_projection_and_limit() {
    let client = start_mock_server_and_client().await;

    let session = client.open_session(None, Some("app")).await.expect("open session");

    let response = client
        .scan(
            "app",
            "messages",
            "shared",
            &session.session_id,
            None,
            vec!["id".to_string()],
            Some(1),
            vec![],
        )
        .await
        .expect("scan with projection");

    // The mock always returns the same data regardless of projection/limit,
    // but this verifies the round-trip doesn't fail with these parameters.
    assert!(!response.batches.is_empty());
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn scan_follows_leader_redirect_hint() {
    let host = "127.0.0.1";
    let (follower_rpc_port, _leader_rpc_port) =
        start_leader_redirect_servers(host, Code::Internal).await;

    let client = connect_to(host, follower_rpc_port).await;
    let session = client.open_session(None, Some("app")).await.expect("open session");

    let response = client
        .scan("app", "messages", "shared", &session.session_id, None, vec![], None, vec![])
        .await
        .expect("scan should follow leader redirect");

    assert_eq!(response.batches.len(), 1);
    assert_eq!(response.batches[0].num_rows(), 3);
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn execute_sql_follows_leader_redirect_hint() {
    let host = "127.0.0.1";
    let (follower_rpc_port, _leader_rpc_port) =
        start_leader_redirect_servers(host, Code::Internal).await;

    let client = connect_to(host, follower_rpc_port).await;
    let session = client.open_session(None, Some("app")).await.expect("open session");

    let response = client
        .execute_sql("ALTER TABLE app.messages ADD COLUMN name TEXT", &session.session_id)
        .await
        .expect("execute_sql should follow leader redirect");

    assert_eq!(response, "executed: ALTER TABLE app.messages ADD COLUMN name TEXT");
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn execute_query_follows_leader_redirect_hint() {
    let host = "127.0.0.1";
    let (follower_rpc_port, _leader_rpc_port) =
        start_leader_redirect_servers(host, Code::Internal).await;

    let client = connect_to(host, follower_rpc_port).await;
    let session = client.open_session(None, Some("app")).await.expect("open session");

    let (message, rows) = client
        .execute_query("SELECT 1", &session.session_id)
        .await
        .expect("execute_query should follow leader redirect");

    assert_eq!(message, "executed: SELECT 1");
    assert!(rows.is_empty());
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn insert_follows_leader_redirect_hint_from_failed_precondition() {
    let host = "127.0.0.1";
    let (follower_rpc_port, _leader_rpc_port) =
        start_leader_redirect_servers(host, Code::FailedPrecondition).await;

    let client = connect_to(host, follower_rpc_port).await;
    let session = client.open_session(None, Some("app")).await.expect("open session");

    let response = client
        .insert(
            "app",
            "messages",
            "shared",
            &session.session_id,
            None,
            vec![r#"{"id":"msg-1"}"#.to_string()],
        )
        .await
        .expect("insert should follow leader redirect");

    assert_eq!(response.affected_rows, 1);
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn begin_transaction_follows_leader_redirect_hint() {
    let host = "127.0.0.1";
    let (follower_rpc_port, _leader_rpc_port) =
        start_leader_redirect_servers(host, Code::FailedPrecondition).await;

    let client = connect_to(host, follower_rpc_port).await;
    let session = client.open_session(None, Some("app")).await.expect("open session");

    let transaction_id = client
        .begin_transaction(&session.session_id)
        .await
        .expect("begin_transaction should follow leader redirect");

    assert!(!transaction_id.is_empty());
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn commit_transaction_follows_leader_redirect_hint() {
    let host = "127.0.0.1";
    let (follower_rpc_port, leader_rpc_port) =
        start_leader_redirect_servers(host, Code::FailedPrecondition).await;

    let follower_client = connect_to(host, follower_rpc_port).await;
    let session = follower_client
        .open_session(None, Some("app"))
        .await
        .expect("open follower session");

    let leader_client = connect_to(host, leader_rpc_port).await;
    leader_client
        .open_session(Some(&session.session_id), Some("app"))
        .await
        .expect("mirror session on leader");
    let transaction_id = leader_client
        .begin_transaction(&session.session_id)
        .await
        .expect("begin leader transaction");

    let committed_id = follower_client
        .commit_transaction(&session.session_id, &transaction_id)
        .await
        .expect("commit_transaction should follow leader redirect");

    assert_eq!(committed_id, transaction_id);
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn rollback_transaction_follows_leader_redirect_hint() {
    let host = "127.0.0.1";
    let (follower_rpc_port, leader_rpc_port) =
        start_leader_redirect_servers(host, Code::FailedPrecondition).await;

    let follower_client = connect_to(host, follower_rpc_port).await;
    let session = follower_client
        .open_session(None, Some("app"))
        .await
        .expect("open follower session");

    let leader_client = connect_to(host, leader_rpc_port).await;
    leader_client
        .open_session(Some(&session.session_id), Some("app"))
        .await
        .expect("mirror session on leader");
    let transaction_id = leader_client
        .begin_transaction(&session.session_id)
        .await
        .expect("begin leader transaction");

    let rolled_back_id = follower_client
        .rollback_transaction(&session.session_id, &transaction_id)
        .await
        .expect("rollback_transaction should follow leader redirect");

    assert_eq!(rolled_back_id, transaction_id);
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn insert_single_row() {
    let client = start_mock_server_and_client().await;

    let session = client.open_session(None, Some("app")).await.expect("open session");

    let response = client
        .insert(
            "app",
            "messages",
            "shared",
            &session.session_id,
            None,
            vec![r#"{"id":{"Int64":"10"},"name":{"Utf8":"charlie"}}"#.to_string()],
        )
        .await
        .expect("insert");

    assert_eq!(response.affected_rows, 1);
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn insert_multiple_rows() {
    let client = start_mock_server_and_client().await;

    let session = client.open_session(None, Some("app")).await.expect("open session");

    let response = client
        .insert(
            "app",
            "messages",
            "shared",
            &session.session_id,
            None,
            vec![
                r#"{"id":{"Int64":"20"},"name":{"Utf8":"dave"}}"#.to_string(),
                r#"{"id":{"Int64":"21"},"name":{"Utf8":"eve"}}"#.to_string(),
                r#"{"id":{"Int64":"22"},"name":{"Utf8":"frank"}}"#.to_string(),
            ],
        )
        .await
        .expect("insert multiple");

    assert_eq!(response.affected_rows, 3);
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn insert_with_user_id() {
    let client = start_mock_server_and_client().await;

    let session = client.open_session(None, Some("app")).await.expect("open session");

    let response = client
        .insert(
            "app",
            "messages",
            "user",
            &session.session_id,
            Some("user-42"),
            vec![r#"{"id":{"Int64":"30"},"name":{"Utf8":"grace"}}"#.to_string()],
        )
        .await
        .expect("insert with user_id");

    assert_eq!(response.affected_rows, 1);
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn update_single_row() {
    let client = start_mock_server_and_client().await;

    let session = client.open_session(None, Some("app")).await.expect("open session");

    let response = client
        .update(
            "app",
            "messages",
            "shared",
            &session.session_id,
            None,
            "1",
            r#"{"name":{"Utf8":"alice-updated"}}"#,
        )
        .await
        .expect("update");

    assert_eq!(response.affected_rows, 1);
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn update_with_user_id() {
    let client = start_mock_server_and_client().await;

    let session = client.open_session(None, Some("app")).await.expect("open session");

    let response = client
        .update(
            "app",
            "messages",
            "user",
            &session.session_id,
            Some("user-42"),
            "1",
            r#"{"name":{"Utf8":"updated-name"}}"#,
        )
        .await
        .expect("update with user_id");

    assert_eq!(response.affected_rows, 1);
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn delete_single_row() {
    let client = start_mock_server_and_client().await;

    let session = client.open_session(None, Some("app")).await.expect("open session");

    let response = client
        .delete("app", "messages", "shared", &session.session_id, None, "1")
        .await
        .expect("delete");

    assert_eq!(response.affected_rows, 1);
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn delete_with_user_id() {
    let client = start_mock_server_and_client().await;

    let session = client.open_session(None, Some("app")).await.expect("open session");

    let response = client
        .delete("app", "messages", "user", &session.session_id, Some("user-42"), "1")
        .await
        .expect("delete with user_id");

    assert_eq!(response.affected_rows, 1);
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn transaction_lifecycle_with_dml() {
    let client = start_mock_server_and_client().await;

    let session = client.open_session(None, Some("app")).await.expect("open session");

    // Begin transaction
    let tx_id = client.begin_transaction(&session.session_id).await.expect("begin tx");
    assert!(!tx_id.is_empty());

    // Insert within transaction
    let ins_resp = client
        .insert(
            "app",
            "messages",
            "shared",
            &session.session_id,
            None,
            vec![r#"{"id":{"Int64":"100"},"name":{"Utf8":"tx-row"}}"#.to_string()],
        )
        .await
        .expect("insert in tx");
    assert_eq!(ins_resp.affected_rows, 1);

    // Update within transaction
    let upd_resp = client
        .update(
            "app",
            "messages",
            "shared",
            &session.session_id,
            None,
            "100",
            r#"{"name":{"Utf8":"tx-row-updated"}}"#,
        )
        .await
        .expect("update in tx");
    assert_eq!(upd_resp.affected_rows, 1);

    // Commit transaction
    client.commit_transaction(&session.session_id, &tx_id).await.expect("commit tx");
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn transaction_rollback() {
    let client = start_mock_server_and_client().await;

    let session = client.open_session(None, Some("app")).await.expect("open session");

    let tx_id = client.begin_transaction(&session.session_id).await.expect("begin tx");

    // Insert then rollback
    client
        .insert(
            "app",
            "messages",
            "shared",
            &session.session_id,
            None,
            vec![r#"{"id":{"Int64":"200"},"name":{"Utf8":"will-be-rolled-back"}}"#.to_string()],
        )
        .await
        .expect("insert in tx");

    client
        .rollback_transaction(&session.session_id, &tx_id)
        .await
        .expect("rollback tx");
}
