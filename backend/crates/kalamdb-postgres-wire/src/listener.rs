use std::{sync::Arc, time::Duration};

use kalamdb_auth::UserRepository;
use kalamdb_configs::{PostgresWireSettings, ServerConfig};
use kalamdb_core::{app_context::AppContext, sql::executor::SqlExecutor};
use tokio::sync::oneshot;

use crate::{
    query::KalamQueryHandler,
    server::{
        bind_listener, load_tls_acceptor, run_bound_listener_until_shutdown, KalamWireHandlers,
        PostgresWireListenerConfig,
    },
    sql_exec::WireSqlExecutor,
    startup::KalamStartupHandler,
};

/// Runtime dependencies required to start the PostgreSQL wire listener.
pub struct PostgresWireRuntimeDeps {
    pub app_context:  Arc<AppContext>,
    pub sql_executor: Arc<SqlExecutor>,
    pub user_repo:    Arc<dyn UserRepository>,
}

/// Background task handle for the PostgreSQL wire listener.
pub struct PostgresWireListener {
    shutdown:  Option<oneshot::Sender<()>>,
    task:      tokio::task::JoinHandle<std::io::Result<()>>,
    completed: bool,
}

impl PostgresWireListener {
    /// Bind and start the configured PostgreSQL-wire listener.
    ///
    /// Returns only after TLS configuration and socket acquisition have succeeded.
    pub async fn start(
        server_config: &ServerConfig,
        deps: PostgresWireRuntimeDeps,
    ) -> std::io::Result<Option<Self>> {
        if !server_config.postgres_wire.enabled {
            return Ok(None);
        }

        let connection_drain_timeout =
            Duration::from_secs(server_config.performance.client_disconnect_timeout.max(1));
        let listener_config = PostgresWireListenerConfig::from(&server_config.postgres_wire);
        let tls_acceptor = load_tls_acceptor(&listener_config)?;
        let listener = bind_listener(&listener_config).await?;
        let session_manager = deps.app_context.backend_session_manager();
        let startup_handler = Arc::new(
            KalamStartupHandler::new(session_manager.clone(), deps.user_repo)
                .with_statement_limits(
                    server_config.postgres_wire.prepared_statement_limit,
                    server_config.postgres_wire.portal_limit,
                ),
        );
        let wire_sql_executor =
            Arc::new(WireSqlExecutor::new(deps.app_context, deps.sql_executor, session_manager));
        let query_handler = Arc::new(KalamQueryHandler::new(wire_sql_executor));
        let handlers = Arc::new(KalamWireHandlers::new(startup_handler, query_handler));
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let task = tokio::spawn(async move {
            let shutdown = async move {
                let _ = shutdown_rx.await;
            };
            run_bound_listener_until_shutdown(
                listener,
                handlers,
                tls_acceptor,
                connection_drain_timeout,
                shutdown,
            )
            .await
        });

        Ok(Some(Self {
            shutdown: Some(shutdown_tx),
            task,
            completed: false,
        }))
    }

    /// Wait for unexpected listener termination without losing ownership if cancelled by select.
    pub async fn wait(&mut self) -> std::io::Result<()> {
        let result = (&mut self.task).await;
        self.completed = true;
        flatten_task_result(result)
    }

    /// Stop accepting new PostgreSQL-wire connections without waiting for active sessions.
    pub fn stop_accepting(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }

    /// Wait for the listener and active connection drain to finish.
    pub async fn finish(mut self) -> std::io::Result<()> {
        if self.completed {
            return Ok(());
        }
        self.completed = true;
        flatten_task_result(self.task.await)
    }

    pub async fn shutdown(mut self) -> std::io::Result<()> {
        log::info!("Shutting down PostgreSQL wire listener...");
        self.stop_accepting();
        self.finish().await
    }
}

fn flatten_task_result(
    result: Result<std::io::Result<()>, tokio::task::JoinError>,
) -> std::io::Result<()> {
    match result {
        Ok(result) => result,
        Err(error) => {
            Err(std::io::Error::other(format!("PostgreSQL wire listener task failed: {error}")))
        },
    }
}

impl From<&PostgresWireSettings> for PostgresWireListenerConfig {
    fn from(settings: &PostgresWireSettings) -> Self {
        Self {
            enabled:       settings.enabled,
            host:          settings.host.clone(),
            port:          settings.port,
            tls_enabled:   settings.tls_enabled,
            tls_cert_path: settings.tls_cert_path.clone(),
            tls_key_path:  settings.tls_key_path.clone(),
        }
    }
}

/// Startup log segment for the PostgreSQL wire listener, when enabled.
pub fn format_startup_log_segment(settings: &PostgresWireSettings) -> String {
    if !settings.enabled {
        return String::new();
    }

    format!(" | PostgreSQL wire on {}:{}", settings.host, settings.port)
}

#[cfg(test)]
mod tests {
    use super::flatten_task_result;

    #[test]
    fn flatten_task_result_preserves_listener_io_errors() {
        let result: Result<std::io::Result<()>, tokio::task::JoinError> =
            Ok(Err(std::io::Error::other("listener failed")));

        let error = flatten_task_result(result).expect_err("listener error must propagate");

        assert!(error.to_string().contains("listener failed"));
    }
}
