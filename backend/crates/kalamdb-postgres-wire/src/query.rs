use std::{fmt::Debug, sync::Arc};

use async_trait::async_trait;
use futures_util::{Sink, SinkExt};
use pgwire::{
    api::{
        portal::Portal,
        query::{ExtendedQueryHandler, SimpleQueryHandler},
        results::{Response, Tag},
        stmt::StoredStatement,
        store::PortalStore,
        ClientInfo, ClientPortalStore, DEFAULT_NAME,
    },
    error::{ErrorInfo, PgWireError, PgWireResult},
    messages::{
        extendedquery::{
            Bind, BindComplete, Close, CloseComplete, Parse, ParseComplete, Sync as PgSync,
            TARGET_TYPE_BYTE_PORTAL, TARGET_TYPE_BYTE_STATEMENT,
        },
        response::ReadyForQuery,
        PgWireBackendMessage,
    },
};

use crate::{
    connection::{WireConnectionState, WirePortal, WirePreparedStatement},
    params::portal_parameters_to_scalar_values,
    sql_exec::{backend_session_error_to_response, pg_error, WireSqlExecutor},
    statement::{KalamQueryParser, WireCachedStatement},
    tx_control::{
        classify_transaction_control, execute_transaction_control, WireTransactionOutcome,
    },
};

#[derive(Clone)]
pub struct KalamQueryHandler {
    sql_executor: Arc<WireSqlExecutor>,
    query_parser: Arc<KalamQueryParser>,
}

impl KalamQueryHandler {
    pub fn new(sql_executor: Arc<WireSqlExecutor>) -> Self {
        let query_parser = Arc::new(KalamQueryParser::new(
            sql_executor.app_context(),
            sql_executor.core_sql_executor(),
        ));
        Self {
            sql_executor,
            query_parser,
        }
    }

    async fn execute_sql(
        &self,
        state: &WireConnectionState,
        sql: &str,
    ) -> PgWireResult<Vec<Response>> {
        if let Some(control) = classify_transaction_control(sql) {
            let outcome = execute_transaction_control(
                self.sql_executor.session_manager(),
                state.session_id(),
                control,
            )
            .await;
            return Ok(vec![match outcome {
                Ok(WireTransactionOutcome::Began { .. }) => {
                    Response::TransactionStart(Tag::new("BEGIN"))
                },
                Ok(WireTransactionOutcome::Committed) => {
                    Response::TransactionEnd(Tag::new("COMMIT"))
                },
                Ok(WireTransactionOutcome::RolledBack) => {
                    Response::TransactionEnd(Tag::new("ROLLBACK"))
                },
                Err(error) => backend_session_error_to_response(error),
            }]);
        }

        self.sql_executor.execute(state, sql).await
    }

    async fn execute_cached(
        &self,
        state: &WireConnectionState,
        cached: &WireCachedStatement,
        params: Vec<kalamdb_core::sql::ScalarValue>,
    ) -> PgWireResult<Vec<Response>> {
        if let Some(control) = classify_transaction_control(cached.metadata.sql.as_str()) {
            let outcome = execute_transaction_control(
                self.sql_executor.session_manager(),
                state.session_id(),
                control,
            )
            .await;
            return Ok(vec![match outcome {
                Ok(WireTransactionOutcome::Began { .. }) => {
                    Response::TransactionStart(Tag::new("BEGIN"))
                },
                Ok(WireTransactionOutcome::Committed) => {
                    Response::TransactionEnd(Tag::new("COMMIT"))
                },
                Ok(WireTransactionOutcome::RolledBack) => {
                    Response::TransactionEnd(Tag::new("ROLLBACK"))
                },
                Err(error) => backend_session_error_to_response(error),
            }]);
        }

        self.sql_executor.execute_metadata(state, &cached.metadata, params).await
    }
}

#[async_trait]
impl SimpleQueryHandler for KalamQueryHandler {
    async fn do_query<C>(&self, client: &mut C, query: &str) -> PgWireResult<Vec<Response>>
    where
        C: ClientInfo + ClientPortalStore + Unpin + Send + Sync,
        C::PortalStore: pgwire::api::store::PortalStore,
    {
        let state = client
            .session_extensions()
            .get::<WireConnectionState>()
            .ok_or_else(|| pg_error("wire connection state is missing"))?;
        self.execute_sql(&state, query).await
    }
}

#[async_trait]
impl ExtendedQueryHandler for KalamQueryHandler {
    type Statement = WireCachedStatement;
    type QueryParser = KalamQueryParser;

    fn query_parser(&self) -> Arc<Self::QueryParser> {
        self.query_parser.clone()
    }

    async fn on_parse<C>(&self, client: &mut C, message: Parse) -> PgWireResult<()>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore<Statement = Self::Statement>,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        let state = client
            .session_extensions()
            .get::<WireConnectionState>()
            .ok_or_else(|| pg_error("wire connection state is missing"))?;
        let statement_name = message.name.as_deref().unwrap_or(DEFAULT_NAME);
        state
            .ensure_prepared_statement_capacity(statement_name)
            .map_err(limit_error_to_pg)?;

        let parser = self.query_parser();
        let stmt = StoredStatement::parse(client, &message, parser).await?;
        state
            .put_prepared_statement(WirePreparedStatement {
                name: stmt.id.clone(),
                sql: message.query,
            })
            .map_err(limit_error_to_pg)?;
        client.portal_store().put_statement(Arc::new(stmt));
        client.send(PgWireBackendMessage::ParseComplete(ParseComplete::new())).await?;

        Ok(())
    }

    async fn on_bind<C>(&self, client: &mut C, message: Bind) -> PgWireResult<()>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore<Statement = Self::Statement>,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        let state = client
            .session_extensions()
            .get::<WireConnectionState>()
            .ok_or_else(|| pg_error("wire connection state is missing"))?;
        let statement_name = message.statement_name.as_deref().unwrap_or(DEFAULT_NAME);
        let portal_name = message.portal_name.as_deref().unwrap_or(DEFAULT_NAME);
        state.ensure_portal_capacity(portal_name).map_err(limit_error_to_pg)?;

        let statement = client
            .portal_store()
            .get_statement(statement_name)
            .ok_or_else(|| PgWireError::StatementNotFound(statement_name.to_string()))?;
        let portal = Portal::try_new(&message, statement)?;
        state
            .put_portal(WirePortal {
                name: portal.name.clone(),
                statement_name: statement_name.to_string(),
            })
            .map_err(limit_error_to_pg)?;
        client.portal_store().put_portal(Arc::new(portal));
        client.send(PgWireBackendMessage::BindComplete(BindComplete::new())).await?;

        Ok(())
    }

    async fn on_sync<C>(&self, client: &mut C, _message: PgSync) -> PgWireResult<()>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore<Statement = Self::Statement>,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        if let Some(state) = client.session_extensions().get::<WireConnectionState>() {
            state.remove_portal(DEFAULT_NAME);
        }
        client.portal_store().rm_portal(DEFAULT_NAME);
        client
            .send(PgWireBackendMessage::ReadyForQuery(ReadyForQuery::new(
                client.transaction_status(),
            )))
            .await?;
        client.flush().await?;

        Ok(())
    }

    async fn on_close<C>(&self, client: &mut C, message: Close) -> PgWireResult<()>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore<Statement = Self::Statement>,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        let name = message.name.as_deref().unwrap_or(DEFAULT_NAME);
        if let Some(state) = client.session_extensions().get::<WireConnectionState>() {
            match message.target_type {
                TARGET_TYPE_BYTE_STATEMENT => {
                    state.remove_prepared_statement(name);
                    client.portal_store().rm_statement(name);
                },
                TARGET_TYPE_BYTE_PORTAL => {
                    state.remove_portal(name);
                    client.portal_store().rm_portal(name);
                },
                _ => {},
            }
        }
        client.send(PgWireBackendMessage::CloseComplete(CloseComplete::new())).await?;

        Ok(())
    }

    async fn do_query<C>(
        &self,
        client: &mut C,
        portal: &Portal<Self::Statement>,
        _max_rows: usize,
    ) -> PgWireResult<Response>
    where
        C: ClientInfo + ClientPortalStore + Unpin + Send + Sync,
        C::PortalStore: pgwire::api::store::PortalStore<Statement = Self::Statement>,
    {
        let state = client
            .session_extensions()
            .get::<WireConnectionState>()
            .ok_or_else(|| pg_error("wire connection state is missing"))?;
        let params =
            portal_parameters_to_scalar_values(portal, &portal.statement.statement.parameter_types)?;
        let mut responses = self
            .execute_cached(&state, &portal.statement.statement, params)
            .await?;
        Ok(responses.remove(0))
    }
}

fn limit_error_to_pg(error: crate::connection::WireConnectionStateError) -> PgWireError {
    PgWireError::UserError(Box::new(ErrorInfo::new(
        "ERROR".to_string(),
        "54000".to_string(),
        error.to_string(),
    )))
}

impl Debug for KalamQueryHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KalamQueryHandler").finish_non_exhaustive()
    }
}
