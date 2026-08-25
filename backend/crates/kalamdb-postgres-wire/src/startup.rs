use std::{
    fmt::Debug,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use futures_util::{Sink, SinkExt};
use kalamdb_auth::{repository::user_repo::UserRepository, WirePasswordAuthRequest};
use kalamdb_backend::manager::BackendSessionManager;
use kalamdb_commons::models::{NamespaceId, Role, SessionOrigin};
use pgwire::{
    api::{
        auth::{
            finish_authentication, protocol_negotiation, save_startup_parameters_to_metadata,
            DefaultServerParameterProvider, ServerParameterProvider, StartupHandler,
        },
        ClientInfo, PidSecretKeyGenerator, RandomPidSecretKeyGenerator, METADATA_DATABASE,
        METADATA_USER,
    },
    error::{ErrorInfo, PgWireError, PgWireResult},
    messages::{startup::Authentication, PgWireBackendMessage, PgWireFrontendMessage},
};
use uuid::Uuid;

use crate::{
    connection::{WireConnectionState, DEFAULT_PORTAL_LIMIT, DEFAULT_PREPARED_STATEMENT_LIMIT},
    handlers::authenticate_startup_password,
};

const INVALID_AUTH_CODE: &str = "28P01";
const CONNECTION_FAILURE_CODE: &str = "08006";

/// Logical database name advertised in `pg_catalog.pg_database` and
/// `CURRENT_DATABASE()`. Clients use this when picking a database from a list.
pub const WIRE_LOGICAL_DATABASE: &str = "kalam";

/// Resolve the PostgreSQL startup `database` parameter to a KalamDB namespace.
///
/// KalamDB is a single logical database. Catalog aliases (`kalam`, `postgres`,
/// empty) open the default namespace so GUI "Load Databases" / reconnect flows
/// work. Any other non-empty name is treated as the initial schema/namespace.
pub fn resolve_wire_startup_schema(database: Option<&str>) -> NamespaceId {
    let trimmed = database.map(str::trim).unwrap_or("");
    if is_wire_catalog_database(trimmed) {
        NamespaceId::default_ns()
    } else {
        NamespaceId::new(trimmed)
    }
}

fn is_wire_catalog_database(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "" | WIRE_LOGICAL_DATABASE | "postgres" | "template1"
    )
}

pub struct KalamStartupHandler<P = DefaultServerParameterProvider> {
    session_manager:          Arc<BackendSessionManager>,
    user_repo:                Arc<dyn UserRepository>,
    parameter_provider:       P,
    pid_secret_key_generator: Arc<dyn PidSecretKeyGenerator>,
    prepared_statement_limit: usize,
    portal_limit:             usize,
}

#[derive(Debug)]
pub struct WireSessionGuard {
    session_manager: Arc<BackendSessionManager>,
    session_id:      String,
}

impl WireSessionGuard {
    pub fn new(session_manager: Arc<BackendSessionManager>, session_id: impl Into<String>) -> Self {
        Self {
            session_manager,
            session_id: session_id.into(),
        }
    }
}

impl Drop for WireSessionGuard {
    fn drop(&mut self) {
        let session_manager = self.session_manager.clone();
        let session_id = self.session_id.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = session_manager.close_session(session_id.as_str()).await;
            });
        }
    }
}

impl KalamStartupHandler {
    pub fn new(
        session_manager: Arc<BackendSessionManager>,
        user_repo: Arc<dyn UserRepository>,
    ) -> Self {
        Self {
            session_manager,
            user_repo,
            parameter_provider: DefaultServerParameterProvider::default(),
            pid_secret_key_generator: Arc::new(RandomPidSecretKeyGenerator::default()),
            prepared_statement_limit: DEFAULT_PREPARED_STATEMENT_LIMIT,
            portal_limit: DEFAULT_PORTAL_LIMIT,
        }
    }
}

impl<P> KalamStartupHandler<P>
where
    P: ServerParameterProvider,
{
    pub fn with_parameter_provider(
        session_manager: Arc<BackendSessionManager>,
        user_repo: Arc<dyn UserRepository>,
        parameter_provider: P,
    ) -> Self {
        Self {
            session_manager,
            user_repo,
            parameter_provider,
            pid_secret_key_generator: Arc::new(RandomPidSecretKeyGenerator::default()),
            prepared_statement_limit: DEFAULT_PREPARED_STATEMENT_LIMIT,
            portal_limit: DEFAULT_PORTAL_LIMIT,
        }
    }

    pub fn with_pid_secret_key_generator(
        mut self,
        generator: Arc<dyn PidSecretKeyGenerator>,
    ) -> Self {
        self.pid_secret_key_generator = generator;
        self
    }

    pub fn with_statement_limits(
        mut self,
        prepared_statement_limit: usize,
        portal_limit: usize,
    ) -> Self {
        self.prepared_statement_limit = prepared_statement_limit;
        self.portal_limit = portal_limit;
        self
    }
}

#[async_trait]
impl<P> StartupHandler for KalamStartupHandler<P>
where
    P: ServerParameterProvider + Send + Sync,
{
    async fn on_startup<C>(
        &self,
        client: &mut C,
        message: PgWireFrontendMessage,
    ) -> PgWireResult<()>
    where
        C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        match message {
            PgWireFrontendMessage::Startup(ref startup) => {
                protocol_negotiation(client, startup).await?;
                save_startup_parameters_to_metadata(client, startup);
                client.set_state(pgwire::api::PgWireConnectionState::AuthenticationInProgress);
                client
                    .send(PgWireBackendMessage::Authentication(Authentication::CleartextPassword))
                    .await?;
            },
            PgWireFrontendMessage::PasswordMessageFamily(password) => {
                let user = client
                    .metadata()
                    .get(METADATA_USER)
                    .cloned()
                    .ok_or(PgWireError::UserNameRequired)?;
                let password = password.into_password()?.password;
                let client_addr = Some(client.socket_addr().to_string());
                let request = WirePasswordAuthRequest {
                    user: user.clone(),
                    password,
                    client_addr: client_addr.clone(),
                };
                let auth =
                    authenticate_startup_password(request, &self.user_repo, current_timestamp_ms())
                        .await
                        .map_err(|_| pg_error("ERROR", INVALID_AUTH_CODE, "Invalid credentials"))?;

                let session_id = Uuid::now_v7().to_string();
                let current_schema = resolve_wire_startup_schema(
                    client.metadata().get(METADATA_DATABASE).map(String::as_str),
                );

                self.session_manager
                    .open_session(
                        SessionOrigin::WireProtocol,
                        session_id.clone(),
                        auth.clone(),
                        Some(current_schema.as_str().to_string()),
                        client_addr,
                    )
                    .map_err(|error| {
                        pg_error(
                            "ERROR",
                            CONNECTION_FAILURE_CODE,
                            format!("failed to open backend session: {error}"),
                        )
                    })?;

                let is_dba = matches!(auth.role, Role::Dba | Role::System);
                let (pid, secret_key) = self.pid_secret_key_generator.generate(client);
                client.set_pid_and_secret_key(pid, secret_key);
                client.session_extensions().insert(WireConnectionState::with_limits(
                    session_id.clone(),
                    auth,
                    current_schema,
                    self.prepared_statement_limit,
                    self.portal_limit,
                ));
                client
                    .session_extensions()
                    .insert(WireSessionGuard::new(self.session_manager.clone(), session_id));
                client.metadata_mut().insert("kalamdb.session_authorization".to_string(), user);
                client.metadata_mut().insert("kalamdb.is_dba".to_string(), is_dba.to_string());

                finish_authentication(client, &self.parameter_provider).await?;
            },
            _ => {},
        }

        Ok(())
    }
}

pub async fn close_wire_session(
    manager: &BackendSessionManager,
    state: &WireConnectionState,
) -> PgWireResult<()> {
    manager
        .close_session(state.session_id())
        .await
        .map_err(|error| pg_error("ERROR", CONNECTION_FAILURE_CODE, error.to_string()))
}

fn current_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn pg_error(
    severity: impl Into<String>,
    code: impl Into<String>,
    message: impl Into<String>,
) -> PgWireError {
    PgWireError::UserError(Box::new(ErrorInfo::new(severity.into(), code.into(), message.into())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_aliases_open_default_namespace() {
        for name in [None, Some(""), Some("kalam"), Some("Kalam"), Some("postgres"), Some("template1")]
        {
            assert_eq!(
                resolve_wire_startup_schema(name).as_str(),
                "default",
                "alias {name:?} should map to default namespace"
            );
        }
    }

    #[test]
    fn other_database_names_become_initial_schema() {
        assert_eq!(resolve_wire_startup_schema(Some("app_ns")).as_str(), "app_ns");
        assert_eq!(resolve_wire_startup_schema(Some("  MyNs  ")).as_str(), "myns");
    }
}
