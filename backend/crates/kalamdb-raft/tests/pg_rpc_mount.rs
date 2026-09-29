use std::{sync::Arc, time::Duration};

use kalam_pg_client::RemoteKalamClient;
use kalam_pg_common::RemoteServerConfig;
use kalamdb_commons::models::NodeId;
use kalamdb_pg::{KalamPgService, PgServiceServer};
use kalamdb_raft::{
    manager::{RaftManager, RaftManagerConfig},
    named_service_routes,
    network::{cluster_handler::NoOpClusterHandler, start_rpc_server},
};

#[tokio::test]
#[ntest::timeout(10000)]
async fn shared_rpc_server_hosts_pg_service() {
    let rpc_addr = "127.0.0.1:19741".to_string();
    let config = RaftManagerConfig {
        node_id: NodeId::new(1),
        rpc_addr: rpc_addr.clone(),
        api_addr: "127.0.0.1:29741".to_string(),
        peers: Vec::new(),
        user_shards: 1,
        shared_shards: 1,
        ..Default::default()
    };

    let manager = Arc::new(RaftManager::new(config));
    let cluster_handler: Arc<dyn kalamdb_raft::ClusterMessageHandler> =
        Arc::new(NoOpClusterHandler);
    let pg_service = KalamPgService::new(false, None).allow_insecure_unauthenticated();
    let routes = named_service_routes(PgServiceServer::new(pg_service));

    start_rpc_server(manager, rpc_addr, cluster_handler, Some(routes))
        .await
        .expect("start shared rpc server");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = RemoteKalamClient::connect(RemoteServerConfig {
        host: "127.0.0.1".to_string(),
        port: 19741,
        ..Default::default()
    })
    .await
    .expect("connect remote client");

    client.ping().await.expect("ping shared rpc service");
    let session = client
        .open_session(None, Some("tenant_a"))
        .await
        .expect("open session over shared rpc server");

    assert!(!session.session_id.is_empty(), "server should issue a session id");
    assert_eq!(session.current_schema.as_deref(), Some("tenant_a"));
}
