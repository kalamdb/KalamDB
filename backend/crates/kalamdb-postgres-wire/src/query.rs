use std::{fmt::Debug, sync::Arc};

use async_trait::async_trait;
use futures_util::{Sink, SinkExt};
use kalamdb_backend::session::BackendSessionState;
use pgwire::{
    api::{
        portal::Portal,
        query::{ExtendedQueryHandler, SimpleQueryHandler},
        results::Response,
        stmt::StoredStatement,
        store::{Entry, PortalStore},
        ClientInfo, ClientPortalStore, DEFAULT_NAME,
    },
    error::{ErrorInfo, PgWireError, PgWireResult},
    messages::{
        extendedquery::{
            Bind, BindComplete, Close, CloseComplete, Parse, ParseComplete, Sync as PgSync,
            TARGET_TYPE_BYTE_PORTAL, TARGET_TYPE_BYTE_STATEMENT,
        },
        response::{ReadyForQuery, TransactionStatus},
        PgWireBackendMessage,
    },
};

use crate::{
    connection::{WireConnectionState, WirePortal, WirePreparedStatement},
    params::portal_parameters_to_scalar_values,
    sql_exec::{backend_session_error_to_response, pg_error, WireSqlExecutor},
    statement::{KalamQueryParser, WireCachedStatement},
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
        // The per-statement cap is 1 MiB. A simple-query script is many
        // statements in one message, so the cap applies after the split.
        // PostgreSQL accepts the PGlite speed scripts (~2 MiB) this way.
        state.ensure_simple_query_message_bytes(sql).map_err(limit_error_to_pg)?;

        let statements = match kalamdb_sql::split_statements(sql) {
            Ok(statements) => statements,
            Err(_) => vec![sql.to_string()],
        };

        if statements.is_empty() {
            return Ok(vec![Response::EmptyQuery]);
        }

        for statement in &statements {
            state.ensure_statement_sql_bytes(statement).map_err(limit_error_to_pg)?;
        }

        if statements.len() == 1 {
            return self.execute_single_sql_statement(state, statements[0].as_str()).await;
        }

        let mut all_responses = Vec::with_capacity(statements.len());
        let mut implicit_tx_active = false;
        let was_in_explicit_block = matches!(
            self.sql_executor.session_manager().block_state(state.session_id()),
            Ok(BackendSessionState::InTransaction { .. }
                | BackendSessionState::InFailedTransaction)
        );
        let can_use_implicit_tx = !was_in_explicit_block
            && statements.iter().all(|stmt| {
                self.sql_executor.statement_supports_implicit_batch_transaction(stmt, state)
            });

        if can_use_implicit_tx {
            if self
                .sql_executor
                .session_manager()
                .begin_block(state.session_id())
                .await
                .is_ok()
            {
                implicit_tx_active = true;
                state.session_config().begin();
            }
        }

        let mut batch_failed = false;
        for stmt in &statements {
            let responses = self.execute_single_sql_statement(state, stmt.as_str()).await?;
            let has_error = responses.iter().any(|r| matches!(r, Response::Error(_)));
            all_responses.extend(responses);
            if has_error {
                batch_failed = true;
                break;
            }
        }

        if implicit_tx_active {
            if batch_failed {
                let _ =
                    self.sql_executor.session_manager().rollback_block(state.session_id()).await;
            } else if let Err(error) =
                self.sql_executor.session_manager().commit_block(state.session_id()).await
            {
                let _ =
                    self.sql_executor.session_manager().rollback_block(state.session_id()).await;
                all_responses.push(backend_session_error_to_response(error));
            }
        }

        if implicit_tx_active {
            if batch_failed || all_responses.iter().any(|r| matches!(r, Response::Error(_))) {
                state.session_config().rollback();
            } else {
                state.session_config().commit();
            }
        }
        Ok(all_responses)
    }

    async fn execute_single_sql_statement(
        &self,
        state: &WireConnectionState,
        sql: &str,
    ) -> PgWireResult<Vec<Response>> {
        self.sql_executor.execute(state, sql).await
    }

    fn resolve_transaction_status<C>(&self, client: &C) -> TransactionStatus
    where
        C: ClientInfo,
    {
        if let Some(state) = client.session_extensions().get::<WireConnectionState>() {
            if let Ok(block_state) =
                self.sql_executor.session_manager().block_state(state.session_id())
            {
                return match block_state {
                    BackendSessionState::Idle => TransactionStatus::Idle,
                    BackendSessionState::InTransaction { .. } => TransactionStatus::Transaction,
                    BackendSessionState::InFailedTransaction => TransactionStatus::Error,
                };
            }
        }
        client.transaction_status()
    }

    async fn execute_cached_with_format(
        &self,
        state: &WireConnectionState,
        cached: &WireCachedStatement,
        params: Vec<kalamdb_core::sql::ScalarValue>,
        column_format: &pgwire::api::portal::Format,
    ) -> PgWireResult<Vec<Response>> {
        self.sql_executor
            .execute_metadata(state, &cached.metadata, params, Some(column_format))
            .await
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
            .ensure_statement_sql_bytes(message.query.as_str())
            .map_err(limit_error_to_pg)?;
        state
            .ensure_prepared_statement_capacity(statement_name)
            .map_err(limit_error_to_pg)?;

        let parser = self.query_parser();
        match StoredStatement::parse(client, &message, parser).await? {
            Some(stmt) => {
                state
                    .put_prepared_statement(WirePreparedStatement {
                        name: stmt.id.clone(),
                        sql: message.query,
                    })
                    .map_err(limit_error_to_pg)?;
                client.portal_store().put_statement(Arc::new(stmt));
            },
            None => client.portal_store().put_empty_statement(statement_name),
        }
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

        match client.portal_store().get_statement(statement_name) {
            Some(Entry::Value(statement)) => {
                let portal = Portal::try_new(&message, statement)?;
                state
                    .put_portal(WirePortal {
                        name: portal.name.clone(),
                        statement_name: statement_name.to_string(),
                    })
                    .map_err(limit_error_to_pg)?;
                client.portal_store().put_portal(Arc::new(portal));
            },
            Some(Entry::Empty) => {
                if !message.parameters.is_empty() {
                    return Err(PgWireError::UserError(Box::new(ErrorInfo::new(
                        "ERROR".to_owned(),
                        "08P01".to_owned(),
                        format!(
                            "bind message supplies {} parameters, but prepared statement {:?} \
                             requires 0",
                            message.parameters.len(),
                            statement_name
                        ),
                    ))));
                }
                client.portal_store().put_empty_portal(portal_name);
            },
            None => {
                return Err(PgWireError::StatementNotFound(statement_name.to_string()));
            },
        }
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
        let status = self.resolve_transaction_status(client);
        client.set_transaction_status(status);
        client
            .send(PgWireBackendMessage::ReadyForQuery(ReadyForQuery::new(status)))
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
        let params = {
            let _span = kalamdb_observability::kdb_info_span_entered!("wire.bind");
            portal_parameters_to_scalar_values(portal, &portal.statement.statement.parameter_types)?
        };
        let mut responses = self
            .execute_cached_with_format(
                &state,
                &portal.statement.statement,
                params,
                &portal.result_column_format,
            )
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
