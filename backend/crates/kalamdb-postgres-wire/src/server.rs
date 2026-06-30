use std::{future::Future, net::SocketAddr, sync::Arc};

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
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub tls_enabled: bool,
    pub tls_cert_path: Option<String>,
    pub tls_key_path: Option<String>,
}

impl PostgresWireListenerConfig {
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

pub struct KalamWireHandlers {
    startup_handler: Arc<KalamStartupHandler>,
    query_handler: Arc<KalamQueryHandler>,
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
    let listener = TcpListener::bind(config.bind_addr()).await?;
    accept_loop(listener, handlers, tls_acceptor, shutdown).await
}

fn load_tls_acceptor(config: &PostgresWireListenerConfig) -> std::io::Result<Option<TlsAcceptor>> {
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

    while tasks.join_next().await.is_some() {}

    Ok(())
}
