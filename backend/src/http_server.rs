//! Shared Actix HTTP server construction and ownership.

use std::{net::SocketAddr, sync::Arc};

use actix_web::{App, HttpServer};
use anyhow::Result;
use kalamdb_configs::ServerConfig;
use kalamdb_core::app_context::AppContext;
use tracing_actix_web::{RootSpanBuilder, TracingLogger};

use crate::{
    http_runtime::{AuthRuntimeMode, HttpRuntimeState},
    lifecycle::ApplicationComponents,
    middleware, routes,
};

struct KalamDbRootSpanBuilder;

impl RootSpanBuilder for KalamDbRootSpanBuilder {
    fn on_request_start(request: &actix_web::dev::ServiceRequest) -> tracing::Span {
        tracing::info_span!(
            parent: None,
            "HTTP request",
            http.method = %request.method(),
            http.route = %request.uri().path(),
            http.status_code = tracing::field::Empty,
            otel.kind = "server",
            otel.status_code = tracing::field::Empty,
        )
    }

    fn on_request_end<B: actix_web::body::MessageBody>(
        span: tracing::Span,
        outcome: &Result<actix_web::dev::ServiceResponse<B>, actix_web::Error>,
    ) {
        match outcome {
            Ok(response) => {
                let status = response.status().as_u16();
                span.record("http.status_code", status);
                span.record("otel.status_code", if status >= 500 { "ERROR" } else { "OK" });
            },
            Err(error) => {
                span.record("otel.status_code", "ERROR");
                span.record("http.status_code", 500u16);
                tracing::error!(parent: &span, error = %error, "HTTP request error");
            },
        }
    }
}

/// Owns the running Actix server task and its shutdown handle.
pub(crate) struct HttpServerRuntime {
    bind_addr:    SocketAddr,
    http_version: &'static str,
    ui_status:    &'static str,
    handle:       actix_web::dev::ServerHandle,
    task:         tokio::task::JoinHandle<std::io::Result<()>>,
    completed:    bool,
}

impl HttpServerRuntime {
    pub(crate) fn start_configured(
        config: &ServerConfig,
        components: &ApplicationComponents,
        app_context: Arc<AppContext>,
        auth_runtime_mode: AuthRuntimeMode,
    ) -> Result<Self> {
        let bind_addr = format!("{}:{}", config.server.host, config.server.port);
        Self::start(config, components, app_context, auth_runtime_mode, &bind_addr)
    }

    pub(crate) fn start_ephemeral(
        config: &ServerConfig,
        components: &ApplicationComponents,
        app_context: Arc<AppContext>,
        auth_runtime_mode: AuthRuntimeMode,
    ) -> Result<Self> {
        let bind_ip = if config.server.host.is_empty() {
            "127.0.0.1"
        } else {
            config.server.host.as_str()
        };
        Self::start(config, components, app_context, auth_runtime_mode, &format!("{bind_ip}:0"))
    }

    fn start(
        config: &ServerConfig,
        components: &ApplicationComponents,
        app_context: Arc<AppContext>,
        auth_runtime_mode: AuthRuntimeMode,
        bind_addr: &str,
    ) -> Result<Self> {
        let runtime = HttpRuntimeState::new(config, components, app_context, auth_runtime_mode)?;
        let ui_status = runtime.ui_status();

        let server = HttpServer::new(move || {
            let runtime = runtime.clone();
            let mut app = App::new()
                .wrap(runtime.connection_protection.clone())
                .wrap(TracingLogger::<KalamDbRootSpanBuilder>::new())
                .wrap(middleware::build_cors_from_settings(runtime.cors_settings.as_ref()))
                .app_data(runtime.app_context.clone())
                .app_data(runtime.session_factory.clone())
                .app_data(runtime.sql_executor.clone())
                .app_data(runtime.rate_limiter.clone())
                .app_data(runtime.live_query_manager.clone())
                .app_data(runtime.user_repo.clone())
                .app_data(runtime.connection_registry.clone())
                .app_data(runtime.auth_settings.clone())
                .configure(routes::configure);

            #[cfg(feature = "embedded-ui")]
            if kalamdb_api::routes::is_embedded_ui_available() {
                let runtime_config = runtime.ui_runtime_config.clone();
                app = app.configure(move |routes| {
                    kalamdb_api::routes::configure_embedded_ui_routes(
                        routes,
                        runtime_config.clone(),
                    );
                });
            } else if let Some(ref path) = runtime.ui_path {
                let path = path.clone();
                let runtime_config = runtime.ui_runtime_config.clone();
                app = app.configure(move |routes| {
                    kalamdb_api::routes::configure_ui_routes(routes, &path, runtime_config.clone());
                });
            }

            #[cfg(not(feature = "embedded-ui"))]
            if let Some(ref path) = runtime.ui_path {
                let path = path.clone();
                let runtime_config = runtime.ui_runtime_config.clone();
                app = app.configure(move |routes| {
                    kalamdb_api::routes::configure_ui_routes(routes, &path, runtime_config.clone());
                });
            }

            app
        })
        .backlog(config.performance.backlog)
        .disable_signals();

        let server = if config.server.enable_http2 {
            server.bind_auto_h2c(bind_addr)?
        } else {
            server.bind(bind_addr)?
        };
        let bind_addr = server
            .addrs()
            .first()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("HTTP server did not bind any socket addresses"))?;
        let server = server
            .workers(effective_workers(config.server.workers))
            .max_connections(config.performance.max_connections)
            .worker_max_blocking_threads(effective_max_blocking_threads(
                config.performance.worker_max_blocking_threads,
            ))
            .keep_alive(std::time::Duration::from_secs(config.performance.keepalive_timeout))
            .client_request_timeout(std::time::Duration::from_secs(
                config.performance.client_request_timeout,
            ))
            .client_disconnect_timeout(std::time::Duration::from_secs(
                config.performance.client_disconnect_timeout,
            ))
            .shutdown_timeout(10)
            .run();
        let handle = server.handle();
        let task = tokio::spawn(server);

        Ok(Self {
            bind_addr,
            http_version: http_version_label(config.server.enable_http2),
            ui_status,
            handle,
            task,
            completed: false,
        })
    }

    pub(crate) fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    pub(crate) fn http_version(&self) -> &'static str {
        self.http_version
    }

    pub(crate) fn ui_status(&self) -> &'static str {
        self.ui_status
    }

    pub(crate) async fn wait(&mut self) -> std::io::Result<()> {
        let result = (&mut self.task).await;
        self.completed = true;
        flatten_server_result(result)
    }

    /// Pause socket acceptance while keeping established connections alive for draining.
    pub(crate) async fn stop_accepting(&self) {
        self.handle.pause().await;
    }

    pub(crate) async fn shutdown(mut self, graceful: bool) -> std::io::Result<()> {
        self.handle.stop(graceful).await;
        if self.completed {
            return Ok(());
        }
        self.completed = true;
        flatten_server_result(self.task.await)
    }
}

pub fn effective_workers(configured: usize) -> usize {
    std::env::var("KALAMDB_SERVER_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .or_else(|| (configured > 0).then_some(configured))
        .unwrap_or_else(|| num_cpus::get().min(4))
}

pub fn effective_max_blocking_threads(configured: usize) -> usize {
    if configured == 0 {
        kalamdb_configs::defaults::default_worker_max_blocking_threads()
    } else {
        configured
    }
}

fn http_version_label(http2_enabled: bool) -> &'static str {
    if http2_enabled {
        "HTTP/2"
    } else {
        "HTTP/1.1"
    }
}

fn flatten_server_result(
    result: Result<std::io::Result<()>, tokio::task::JoinError>,
) -> std::io::Result<()> {
    match result {
        Ok(result) => result,
        Err(error) => Err(std::io::Error::other(format!("HTTP server task failed: {error}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::{flatten_server_result, http_version_label};

    #[test]
    fn flatten_server_result_preserves_inner_io_errors() {
        let result: Result<std::io::Result<()>, tokio::task::JoinError> =
            Ok(Err(std::io::Error::other("accept loop failed")));

        let error = flatten_server_result(result).expect_err("inner server error must propagate");

        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert!(error.to_string().contains("accept loop failed"));
    }

    #[test]
    fn http_version_label_matches_protocol_setting() {
        assert_eq!(http_version_label(true), "HTTP/2");
        assert_eq!(http_version_label(false), "HTTP/1.1");
    }
}
