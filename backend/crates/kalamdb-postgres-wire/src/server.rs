use std::{future::Future, net::SocketAddr, sync::Arc, time::Duration};

use log::{info, warn};
use pgwire::{
    api::{
        auth::StartupHandler,
        query::{ExtendedQueryHandler, SimpleQueryHandler},
        PgWireServerHandlers,
    },
    tokio::{process_socket, tokio_rustls::rustls, TlsAcceptor},
};
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use tokio::{net::TcpListener, task::JoinSet};

use crate::{query::KalamQueryHandler, startup::KalamStartupHandler};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresWireListenerConfig {
    pub enabled:       bool,
    pub host:          String,
    pub port:          u16,
    pub tls_enabled:   bool,
    pub tls_cert_path: Option<String>,
    pub tls_key_path:  Option<String>,
}

impl PostgresWireListenerConfig {
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

pub struct KalamWireHandlers {
    startup_handler: Arc<KalamStartupHandler>,
    query_handler:   Arc<KalamQueryHandler>,
}

impl KalamWireHandlers {
    pub fn new(
        startup_handler: Arc<KalamStartupHandler>,
        query_handler: Arc<KalamQueryHandler>,
    ) -> Self {
        Self {
            startup_handler,
            query_handler,
        }
    }
}

impl PgWireServerHandlers for KalamWireHandlers {
    fn simple_query_handler(&self) -> Arc<impl SimpleQueryHandler> {
        self.query_handler.clone()
    }

    fn extended_query_handler(&self) -> Arc<impl ExtendedQueryHandler> {
        self.query_handler.clone()
    }

    fn startup_handler(&self) -> Arc<impl StartupHandler> {
        self.startup_handler.clone()
    }
}

pub async fn run_listener_until_shutdown<F>(
    config: PostgresWireListenerConfig,
    handlers: Arc<KalamWireHandlers>,
    shutdown: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send,
{
    if !config.enabled {
        shutdown.await;
        return Ok(());
    }

    let tls_acceptor = load_tls_acceptor(&config)?;
    let listener = bind_listener(&config).await?;
    run_bound_listener_until_shutdown(
        listener,
        handlers,
        tls_acceptor,
        Duration::from_secs(10),
        shutdown,
    )
    .await
}

pub(crate) async fn bind_listener(
    config: &PostgresWireListenerConfig,
) -> std::io::Result<TcpListener> {
    TcpListener::bind(config.bind_addr()).await
}

pub(crate) async fn run_bound_listener_until_shutdown<F>(
    listener: TcpListener,
    handlers: Arc<KalamWireHandlers>,
    tls_acceptor: Option<TlsAcceptor>,
    connection_drain_timeout: Duration,
    shutdown: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send,
{
    accept_loop(listener, handlers, tls_acceptor, connection_drain_timeout, shutdown).await
}

pub(crate) fn load_tls_acceptor(
    config: &PostgresWireListenerConfig,
) -> std::io::Result<Option<TlsAcceptor>> {
    if !config.tls_enabled {
        return Ok(None);
    }

    let cert_path = config.tls_cert_path.as_deref().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "postgres_wire.tls_enabled requires postgres_wire.tls_cert_path",
        )
    })?;
    let key_path = config.tls_key_path.as_deref().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "postgres_wire.tls_enabled requires postgres_wire.tls_key_path",
        )
    })?;

    let certs = CertificateDer::pem_file_iter(cert_path)
        .map_err(invalid_tls_config)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(invalid_tls_config)?;
    if certs.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "postgres_wire.tls_cert_path did not contain any certificates",
        ));
    }
    let key = PrivateKeyDer::from_pem_file(key_path).map_err(invalid_tls_config)?;
    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(invalid_tls_config)?;
    server_config.alpn_protocols = vec![b"postgresql".to_vec()];

    Ok(Some(TlsAcceptor::from(Arc::new(server_config))))
}

fn invalid_tls_config(error: impl std::error::Error + Send + Sync + 'static) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, error)
}

async fn accept_loop<F>(
    listener: TcpListener,
    handlers: Arc<KalamWireHandlers>,
    tls_acceptor: Option<TlsAcceptor>,
    connection_drain_timeout: Duration,
    shutdown: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send,
{
    let mut tasks = JoinSet::new();

    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (socket, _addr): (_, SocketAddr) = result?;
                let handlers = handlers.clone();
                let tls_acceptor = tls_acceptor.clone();
                tasks.spawn(async move {
                    let _ = process_socket(socket, tls_acceptor, handlers).await;
                });
            },
            _ = &mut shutdown => {
                break;
            },
        }
    }

    if tasks.is_empty() {
        return Ok(());
    }

    info!(
        "PostgreSQL wire listener stopped accepting connections; waiting up to {:.1}s for {} \
         active connection(s) to close",
        connection_drain_timeout.as_secs_f64(),
        tasks.len()
    );

    let drain_result = tokio::time::timeout(connection_drain_timeout, async {
        while tasks.join_next().await.is_some() {}
    })
    .await;

    if drain_result.is_err() {
        let remaining = tasks.len();
        warn!(
            "PostgreSQL wire connections did not close within {:.1}s; aborting {} remaining \
             connection task(s)",
            connection_drain_timeout.as_secs_f64(),
            remaining
        );
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{bind_listener, load_tls_acceptor, PostgresWireListenerConfig};

    fn listener_config(host: String, port: u16) -> PostgresWireListenerConfig {
        PostgresWireListenerConfig {
            enabled: true,
            host,
            port,
            tls_enabled: false,
            tls_cert_path: None,
            tls_key_path: None,
        }
    }

    #[tokio::test]
    #[ntest::timeout(1500)]
    async fn bind_listener_reports_an_occupied_port_before_spawn() {
        let occupied = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("reserve occupied port");
        let addr = occupied.local_addr().expect("occupied listener address");
        let config = listener_config(addr.ip().to_string(), addr.port());

        let error = bind_listener(&config)
            .await
            .expect_err("second bind must fail while the real listener is held");

        assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
    }

    #[test]
    fn load_tls_acceptor_rejects_missing_paths_before_spawn() {
        let mut config = listener_config("127.0.0.1".to_string(), 5432);
        config.tls_enabled = true;

        let error = match load_tls_acceptor(&config) {
            Ok(_) => panic!("TLS paths are required"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[tokio::test]
    #[ntest::timeout(1500)]
    async fn bound_listener_is_the_socket_used_by_the_runtime() {
        let config = listener_config("127.0.0.1".to_string(), 0);

        let listener = bind_listener(&config).await.expect("bind listener");
        let addr = listener.local_addr().expect("bound listener address");

        assert_ne!(addr.port(), 0);
        assert_eq!(addr.ip().to_string(), config.host);
        tokio::time::timeout(Duration::from_millis(10), listener.accept())
            .await
            .expect_err("listener should stay idle without clients");
    }
}
