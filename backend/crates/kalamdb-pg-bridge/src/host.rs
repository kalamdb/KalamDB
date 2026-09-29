use std::sync::Arc;

use kalamdb_auth::{CoreUsersRepo, UserRepository};
use kalamdb_core::app_context::AppContext;
use kalamdb_pg::{KalamPgService, PgServiceServer};
use kalamdb_raft::{named_service_routes, RaftExecutor};

use crate::OperationService;

/// Build an in-process extension service bound to this database.
///
/// Does not mount anything on the RPC listener. Tests use this to call the
/// bridge without enabling it for the process.
pub fn connect(app_ctx: &Arc<AppContext>) -> Arc<KalamPgService> {
    build(app_ctx)
}

/// Mount the extension gRPC service when `pg_extension.enabled` is true.
///
/// Call this before the Raft executor starts. When the flag is false, this
/// returns without allocating the service, the users repo, or RPC routes.
pub fn install(app_ctx: &Arc<AppContext>) -> Option<Arc<KalamPgService>> {
    if !app_ctx.config().pg_extension.enabled {
        log::debug!("PostgreSQL extension gRPC is disabled");
        return None;
    }

    let executor = app_ctx.executor();
    let Some(raft) = executor.as_any().downcast_ref::<RaftExecutor>() else {
        log::warn!("PostgreSQL extension gRPC was not mounted because the executor is not Raft");
        return None;
    };

    let service = build(app_ctx);
    raft.set_extra_rpc_routes(named_service_routes(PgServiceServer::new(service.as_ref().clone())));
    log::info!("PostgreSQL extension gRPC mounted on the cluster RPC listener");
    Some(service)
}

fn build(app_ctx: &Arc<AppContext>) -> Arc<KalamPgService> {
    let mtls = app_ctx.config().rpc_tls.enabled && app_ctx.config().rpc_tls.require_client_cert;
    let pg_auth_token = app_ctx.config().auth.pg_auth_token.clone();
    let users: Arc<dyn UserRepository> =
        Arc::new(CoreUsersRepo::new(app_ctx.system_tables().users()));
    let executor = Arc::new(OperationService::new(Arc::clone(app_ctx)));
    Arc::new(
        KalamPgService::new(mtls, pg_auth_token)
            .with_bearer_auth(users)
            .with_backend_session_manager(app_ctx.backend_session_manager())
            .with_operation_executor(executor),
    )
}

#[cfg(test)]
mod tests {
    use kalamdb_core::test_helpers::test_app_context_simple;

    use super::install;

    #[tokio::test]
    async fn install_does_nothing_when_the_extension_is_disabled() {
        let app_ctx = test_app_context_simple();
        assert!(!app_ctx.config().pg_extension.enabled);
        assert!(install(&app_ctx).is_none());
    }
}
