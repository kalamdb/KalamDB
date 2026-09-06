//! Shared test helpers for kalam-pg-client integration tests.

use std::{collections::HashMap, sync::Mutex, time::Duration};

use async_trait::async_trait;
use kalam_pg_client::RemoteKalamClient;
use kalam_pg_common::RemoteServerConfig;
use kalamdb_backend::session::LiveSessionTransaction;
use kalamdb_commons::models::TransactionId;
use kalamdb_pg::{
    DeleteRequest, InsertRequest, KalamPgService, MutationResult, OperationExecutor,
    PgServiceServer, ScanRequest, ScanResult, TransactionState, UpdateRequest,
};
use tokio::net::TcpListener;
use tonic::Status;
use uuid::Uuid;

/// In-memory transaction registry for gRPC service tests without a full backend.
#[derive(Default)]
pub struct LocalRegistryExecutor {
    active: Mutex<HashMap<String, TransactionId>>,
}

#[async_trait]
impl OperationExecutor for LocalRegistryExecutor {
    async fn execute_scan(&self, _request: ScanRequest) -> Result<ScanResult, Status> {
        Err(Status::unimplemented("not needed for this test"))
    }

    async fn execute_insert(&self, _request: InsertRequest) -> Result<MutationResult, Status> {
        Err(Status::unimplemented("not needed for this test"))
    }

    async fn execute_update(&self, _request: UpdateRequest) -> Result<MutationResult, Status> {
        Err(Status::unimplemented("not needed for this test"))
    }

    async fn execute_delete(&self, _request: DeleteRequest) -> Result<MutationResult, Status> {
        Err(Status::unimplemented("not needed for this test"))
    }

    async fn execute_sql(&self, _sql: &str) -> Result<String, Status> {
        Err(Status::unimplemented("not needed for this test"))
    }

    async fn execute_query(&self, _sql: &str) -> Result<(String, Vec<bytes::Bytes>), Status> {
        Err(Status::unimplemented("not needed for this test"))
    }

    async fn active_transaction(
        &self,
        session_id: &str,
    ) -> Result<Option<LiveSessionTransaction>, Status> {
        Ok(self
            .active
            .lock()
            .expect("local registry executor active tx")
            .get(session_id)
            .cloned()
            .map(|transaction_id| {
                LiveSessionTransaction::new(
                    session_id.to_string(),
                    transaction_id,
                    TransactionState::OpenRead,
                    false,
                )
            }))
    }

    async fn begin_transaction(&self, session_id: &str) -> Result<Option<TransactionId>, Status> {
        let transaction_id = TransactionId::new(Uuid::now_v7().to_string());
        self.active
            .lock()
            .expect("local registry executor begin active tx")
            .insert(session_id.to_string(), transaction_id.clone());
        Ok(Some(transaction_id))
    }

    async fn commit_transaction(
        &self,
        session_id: &str,
        transaction_id: &TransactionId,
    ) -> Result<Option<TransactionId>, Status> {
        let mut active = self.active.lock().expect("local registry executor commit active tx");
        let Some(current) = active.get(session_id) else {
            return Err(Status::failed_precondition("no active transaction"));
        };
        if current != transaction_id {
            return Err(Status::failed_precondition(format!(
                "transaction ID mismatch: expected '{current}', got '{transaction_id}'"
            )));
        }
        active.remove(session_id);
        Ok(Some(transaction_id.clone()))
    }

    async fn rollback_transaction(
        &self,
        session_id: &str,
        transaction_id: &TransactionId,
    ) -> Result<Option<TransactionId>, Status> {
        let mut active = self.active.lock().expect("local registry executor rollback active tx");
        let Some(current) = active.get(session_id) else {
            return Ok(Some(transaction_id.clone()));
        };
        if current != transaction_id {
            return Err(Status::failed_precondition(format!(
                "transaction ID mismatch: expected '{current}', got '{transaction_id}'"
            )));
        }
        active.remove(session_id);
        Ok(Some(transaction_id.clone()))
    }
}

pub async fn start_server_and_client() -> RemoteKalamClient {
    let port =
        start_server_on_ephemeral_port(std::sync::Arc::new(LocalRegistryExecutor::default())).await;
    connect_to("127.0.0.1", port).await
}

pub async fn start_server_on_ephemeral_port(
    executor: std::sync::Arc<dyn OperationExecutor>,
) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind ephemeral port");
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

    tokio::time::sleep(Duration::from_millis(50)).await;
    port
}

pub async fn connect_to(host: &str, port: u16) -> RemoteKalamClient {
    RemoteKalamClient::connect(RemoteServerConfig {
        host: host.to_string(),
        port,
        ..Default::default()
    })
    .await
    .expect("connect client")
}
