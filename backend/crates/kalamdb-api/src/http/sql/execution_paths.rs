use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use actix_web::{http::StatusCode, HttpRequest, HttpResponse};
use bytes::Bytes;
use kalamdb_commons::{models::NamespaceId, schemas::TableType};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    schema_registry::SchemaRegistry,
    sql::{
        context::ExecutionContext,
        executor::{
            request_transaction_state::{
                map_request_transaction_error, AppContextRequestTransactionCoordinator,
                RequestTransactionBatchGuard,
            },
            PreparedExecutionStatement, ScalarValue, SqlExecutor,
        },
        SqlImpersonationService,
    },
};
use kalamdb_raft::GroupId;
use kalamdb_sql::classifier::SqlStatementKind;
use kalamdb_system::FileSubfolderState;

use super::{
    file_utils::{stage_and_finalize_files, substitute_file_placeholders},
    forward::{
        forward_sql_to_group_leader_raw, forwarded_sql_response_to_http, handle_not_leader_error,
        prepared_statement_target_group, should_route_batch_statements_individually,
    },
    helpers::{
        cleanup_files, execute_single_statement, execute_single_statement_raw,
        execution_result_to_query_result, stream_sql_rows_response,
    },
    models::{ErrorCode, QueryRequest, QueryResult, SqlResponse},
    request::took_ms,
    statements::{
        prepare_metadata_or_http_error, resolve_execute_as_user, resolve_result_username,
        PreparedApiExecutionStatement,
    },
};

#[inline]
fn message_contains_any(message: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| message.contains(needle))
}

#[inline]
fn is_permission_error_message(message: &str) -> bool {
    message_contains_any(
        message,
        &[
            "access denied",
            "permission denied",
            "unauthorized",
            "not authorized",
            "forbidden",
            "insufficient privileges",
        ],
    )
}

#[inline]
fn is_table_discovery_error_message(message: &str) -> bool {
    (message.contains("table") && message.contains("not found"))
        || (message.contains("relation") && message.contains("does not exist"))
        || message.contains("unknown table")
}

#[inline]
fn is_leader_routing_error_message(message: &str) -> bool {
    message.contains("not leader")
        || message.contains("not_leader")
        || message.contains("unknown leader")
        || message.contains("no cluster leader")
        || message.contains("no raft leader")
        || message.contains("forward request to cluster leader")
        || message.contains("failed to forward request to cluster leader")
        || message.contains("forward to leader")
}

#[inline]
fn is_safe_validation_error_message(message: &str) -> bool {
    (message.contains("column") && message.contains("not found"))
        || (message.contains("field") && message.contains("not found"))
        || message.contains("no field named")
        || message.contains("schema error: no field named")
        || message.contains("primary key")
        || message.contains("constraint violation")
        || message.contains("already exists")
        || message.contains("duplicate")
        || message.contains("unique constraint")
        || message.contains("unique index")
}

#[inline]
fn classify_sql_error(err: &KalamDbError) -> (StatusCode, ErrorCode, bool) {
    match err {
        KalamDbError::NotLeader { .. } => {
            (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::NotLeader, true)
        },
        KalamDbError::PermissionDenied(_) | KalamDbError::Unauthorized(_) => {
            (StatusCode::FORBIDDEN, ErrorCode::PermissionDenied, true)
        },
        KalamDbError::InvalidSql(_) => (StatusCode::BAD_REQUEST, ErrorCode::InvalidSql, true),
        KalamDbError::AlreadyExists(_)
        | KalamDbError::InvalidOperation(_)
        | KalamDbError::InvalidSchemaEvolution(_)
        | KalamDbError::SystemColumnViolation(_)
        | KalamDbError::ConstraintViolation(_)
        | KalamDbError::Conflict(_)
        | KalamDbError::NamespaceNotFound(_)
        | KalamDbError::IdempotentConflict(_)
        | KalamDbError::ParamCountExceeded { .. }
        | KalamDbError::ParamSizeExceeded { .. }
        | KalamDbError::ParamCountMismatch { .. }
        | KalamDbError::ParamsNotSupported { .. }
        | KalamDbError::ParameterBindingError { .. }
        | KalamDbError::Timeout { .. }
        | KalamDbError::NotImplemented { .. } => {
            (StatusCode::BAD_REQUEST, ErrorCode::SqlExecutionError, true)
        },
        KalamDbError::TableNotFound(_) => {
            (StatusCode::BAD_REQUEST, ErrorCode::SqlExecutionError, false)
        },
        KalamDbError::NotFound(message) => {
            let message_lower = message.to_lowercase();
            if is_table_discovery_error_message(&message_lower) {
                (StatusCode::BAD_REQUEST, ErrorCode::SqlExecutionError, false)
            } else {
                (StatusCode::BAD_REQUEST, ErrorCode::SqlExecutionError, true)
            }
        },
        KalamDbError::ExecutionError(message) => {
            let message_lower = message.to_lowercase();
            if is_leader_routing_error_message(&message_lower) {
                (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::NotLeader, true)
            } else if is_permission_error_message(&message_lower) {
                (StatusCode::FORBIDDEN, ErrorCode::PermissionDenied, true)
            } else if is_safe_validation_error_message(&message_lower) {
                (StatusCode::BAD_REQUEST, ErrorCode::SqlExecutionError, true)
            } else {
                (StatusCode::BAD_REQUEST, ErrorCode::SqlExecutionError, false)
            }
        },
        _ => (StatusCode::BAD_REQUEST, ErrorCode::SqlExecutionError, false),
    }
}

fn build_sql_error_response(
    status: StatusCode,
    code: ErrorCode,
    message: &str,
    details: Option<&str>,
    took: f64,
    is_admin: bool,
    preserve_message: bool,
) -> HttpResponse {
    let payload = if preserve_message {
        if is_admin {
            details.map_or_else(
                || SqlResponse::error(code, message, took),
                |detail| SqlResponse::error_with_details(code, message, detail, took),
            )
        } else {
            SqlResponse::error(code, message, took)
        }
    } else if let Some(detail) = details {
        SqlResponse::error_with_details_for_privilege(code, message, detail, took, is_admin)
    } else {
        SqlResponse::error_for_privilege(code, message, took, is_admin)
    };

    HttpResponse::build(status).json(payload)
}

fn build_kalamdb_error_response(err: &KalamDbError, took: f64, is_admin: bool) -> HttpResponse {
    let (status, code, preserve_message) = classify_sql_error(err);
    let message = err.user_message();
    build_sql_error_response(status, code, message.as_ref(), None, took, is_admin, preserve_message)
}

fn push_or_accumulate_batch_result(
    result: QueryResult,
    is_batch: bool,
    total_inserted: &mut usize,
    total_updated: &mut usize,
    total_deleted: &mut usize,
    results: &mut Vec<QueryResult>,
) {
    if is_batch {
        if let Some(message) = result.message.as_deref() {
            if message.contains("Inserted") {
                *total_inserted += result.row_count;
                return;
            }
            if message.contains("Updated") {
                *total_updated += result.row_count;
                return;
            }
            if message.contains("Deleted") {
                *total_deleted += result.row_count;
                return;
            }
        }
    }

    results.push(result);
}

fn statement_mutates_meta(
    statement: &PreparedApiExecutionStatement,
    app_context: &AppContext,
    routing_user_id: &kalamdb_commons::models::UserId,
) -> bool {
    let target_group = prepared_statement_target_group(statement, app_context, routing_user_id);
    if target_group != Some(GroupId::Meta) {
        return false;
    }

    statement
        .prepared_statement
        .classified_statement
        .as_ref()
        .is_some_and(|classified| classified.is_write_operation())
}

fn is_transient_forwarded_metadata_error(response: &SqlResponse) -> bool {
    let Some(error) = response.error.as_ref() else {
        return false;
    };

    if !matches!(error.code, ErrorCode::SqlExecutionError | ErrorCode::TableNotFound) {
        return false;
    }

    let mut message = error.message.to_ascii_lowercase();
    if let Some(details) = error.details.as_deref() {
        message.push(' ');
        message.push_str(&details.to_ascii_lowercase());
    }

    message.contains("table") && message.contains("not found")
        || message.contains("relation") && message.contains("does not exist")
        || message.contains("unknown table")
        || message.contains("namespace") && message.contains("not found")
        || message.contains("schema") && message.contains("not found")
}

#[allow(clippy::too_many_arguments)]
async fn forward_batch_statement_to_group(
    target_group: GroupId,
    statement: &PreparedApiExecutionStatement,
    http_req: &HttpRequest,
    request_namespace: Option<NamespaceId>,
    app_context: &AppContext,
    request_id: Option<&str>,
    start_time: Instant,
    retry_metadata_lag: bool,
) -> Result<SqlResponse, HttpResponse> {
    const MAX_ATTEMPTS: u32 = 5;
    const INITIAL_BACKOFF_MS: u64 = 5;

    let request = QueryRequest {
        sql: statement.prepared_statement.sql.clone(),
        params: None,
        namespace_id: request_namespace,
    };

    for attempt in 0..MAX_ATTEMPTS {
        let response = forward_sql_to_group_leader_raw(
            target_group,
            http_req,
            &request,
            app_context,
            request_id,
            start_time,
        )
        .await?;

        let status =
            StatusCode::from_u16(response.status_code as u16).unwrap_or(StatusCode::BAD_GATEWAY);
        let parsed = serde_json::from_slice::<SqlResponse>(&response.body);

        if status.is_success() {
            return parsed.map_err(|err| {
                HttpResponse::BadGateway().json(SqlResponse::error(
                    ErrorCode::ForwardFailed,
                    &format!("Failed to decode forwarded SQL response: {}", err),
                    took_ms(start_time),
                ))
            });
        }

        if retry_metadata_lag && attempt + 1 < MAX_ATTEMPTS {
            if let Ok(parsed_response) = parsed.as_ref() {
                if is_transient_forwarded_metadata_error(parsed_response) {
                    let backoff_ms = INITIAL_BACKOFF_MS * (1 << attempt);
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    continue;
                }
            }
        }

        return Err(forwarded_sql_response_to_http(response, start_time));
    }

    Err(HttpResponse::GatewayTimeout().json(SqlResponse::error(
        ErrorCode::ForwardFailed,
        "Timed out waiting for forwarded SQL metadata visibility",
        took_ms(start_time),
    )))
}

fn build_statement_error_response(
    err: &(dyn std::error::Error + 'static),
    statement_index: usize,
    sql: &str,
    took: f64,
    is_admin: bool,
) -> HttpResponse {
    if let Some(kalamdb_err) = err.downcast_ref::<KalamDbError>() {
        let (status, code, preserve_message) = classify_sql_error(kalamdb_err);
        let message = kalamdb_err.statement_failure_message(statement_index);
        return build_sql_error_response(
            status,
            code,
            &message,
            Some(sql),
            took,
            is_admin,
            preserve_message,
        );
    }

    let err_msg = err.to_string();
    if is_leader_routing_error_message(&err_msg.to_lowercase()) {
        let message = format!("Statement {statement_index} failed: {err_msg}");
        return build_sql_error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::NotLeader,
            &message,
            Some(sql),
            took,
            is_admin,
            true,
        );
    }

    let message = format!("Statement {statement_index} failed: {err_msg}");
    build_sql_error_response(
        StatusCode::BAD_REQUEST,
        ErrorCode::SqlExecutionError,
        &message,
        Some(sql),
        took,
        is_admin,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_file_upload_path(
    is_multipart: bool,
    mut files: Option<HashMap<String, (String, Bytes, Option<String>)>>,
    required_files: &[String],
    prepared_statements: &[PreparedApiExecutionStatement],
    app_context: &Arc<AppContext>,
    sql_executor: &Arc<SqlExecutor>,
    exec_ctx: &ExecutionContext,
    impersonation_service: &SqlImpersonationService,
    authorized_username: &str,
    _default_namespace: &NamespaceId,
    params: Vec<ScalarValue>,
    schema_registry: &SchemaRegistry,
    start_time: Instant,
) -> HttpResponse {
    if !is_multipart {
        return HttpResponse::BadRequest().json(SqlResponse::error_for_privilege(
            ErrorCode::InvalidInput,
            "FILE placeholders require multipart/form-data",
            took_ms(start_time),
            exec_ctx.is_admin(),
        ));
    }

    if prepared_statements.len() != 1 {
        return HttpResponse::BadRequest().json(SqlResponse::error_for_privilege(
            ErrorCode::InvalidInput,
            "File uploads require a single SQL statement",
            took_ms(start_time),
            exec_ctx.is_admin(),
        ));
    }

    let stmt = &prepared_statements[0];
    let execute_as_user = match resolve_execute_as_user(stmt, impersonation_service, exec_ctx).await
    {
        Ok(uid) => uid,
        Err(err) => {
            return build_kalamdb_error_response(&err, took_ms(start_time), exec_ctx.is_admin());
        },
    };

    let mut files_map = files.take().unwrap_or_default();
    if !required_files.is_empty() {
        files_map = files_map.into_iter().filter(|(key, _)| required_files.contains(key)).collect();
    }

    let table_id = match stmt.prepared_statement.table_id.clone() {
        Some(tid) => tid,
        None => {
            return HttpResponse::BadRequest().json(SqlResponse::error_for_privilege(
                ErrorCode::InvalidInput,
                "Could not determine target table from SQL. Use fully qualified table name \
                 (namespace.table).",
                took_ms(start_time),
                exec_ctx.is_admin(),
            ));
        },
    };

    let table_entry = match schema_registry.get(&table_id) {
        Some(cached) => cached.table_entry(),
        None => {
            return HttpResponse::BadRequest().json(SqlResponse::error_for_privilege(
                ErrorCode::TableNotFound,
                &format!("Table '{}' not found", table_id),
                took_ms(start_time),
                exec_ctx.is_admin(),
            ));
        },
    };

    let storage_id = table_entry.storage_id.clone();
    let table_type = table_entry.table_type;

    if execute_as_user.is_some() && table_type == TableType::Shared {
        return HttpResponse::BadRequest().json(SqlResponse::error_for_privilege(
            ErrorCode::SqlExecutionError,
            &format!(
                "EXECUTE AS USER is not allowed on SHARED tables (table '{}'). AS USER \
                 impersonation is only supported for USER and STREAM tables.",
                table_id
            ),
            took_ms(start_time),
            exec_ctx.is_admin(),
        ));
    }

    let user_id = match table_type {
        TableType::User => execute_as_user.clone().or_else(|| Some(exec_ctx.user_id().clone())),
        TableType::Shared => None,
        TableType::Stream | TableType::System => {
            return HttpResponse::BadRequest().json(SqlResponse::error_for_privilege(
                ErrorCode::InvalidInput,
                "File uploads are not supported for stream or system tables",
                took_ms(start_time),
                exec_ctx.is_admin(),
            ));
        },
    };

    let manifest_service = app_context.manifest_service();
    let mut subfolder_state = match manifest_service.get_file_subfolder_state(&table_id) {
        Ok(Some(state)) => state,
        Ok(None) => FileSubfolderState::new(),
        Err(e) => {
            log::warn!("Failed to get subfolder state for {}: {}", table_id, e);
            FileSubfolderState::new()
        },
    };

    let file_service = app_context.file_storage_service();
    let file_refs = if files_map.is_empty() {
        HashMap::new()
    } else {
        match stage_and_finalize_files(
            file_service.as_ref(),
            &files_map,
            &storage_id,
            table_type,
            &table_id,
            user_id.as_ref(),
            &mut subfolder_state,
            None,
        )
        .await
        {
            Ok(refs) => refs,
            Err(e) => {
                return HttpResponse::InternalServerError().json(SqlResponse::error_for_privilege(
                    e.code,
                    &e.message,
                    took_ms(start_time),
                    exec_ctx.is_admin(),
                ));
            },
        }
    };

    let modified_sql = substitute_file_placeholders(&stmt.prepared_statement.sql, &file_refs);

    let modified_metadata = match prepare_metadata_or_http_error(
        sql_executor,
        &modified_sql,
        exec_ctx,
        start_time,
    ) {
        Ok(metadata) => metadata,
        Err(resp) => return resp,
    };

    let effective_username =
        resolve_result_username(authorized_username, stmt.execute_as_username.as_deref());

    match execute_single_statement(
        &modified_metadata,
        app_context,
        sql_executor,
        exec_ctx,
        execute_as_user,
        params,
    )
    .await
    {
        Ok(result) => {
            let result = result.with_as_user(effective_username);
            if let Err(e) = manifest_service.update_file_subfolder_state(&table_id, subfolder_state)
            {
                log::warn!("Failed to update subfolder state for {}: {}", table_id, e);
            }
            HttpResponse::Ok().json(SqlResponse::success(vec![result], took_ms(start_time)))
        },
        Err(err) => {
            cleanup_files(
                &file_refs,
                &storage_id,
                table_type,
                &table_id,
                user_id.as_ref(),
                app_context,
            )
            .await;
            build_statement_error_response(
                err.as_ref(),
                1,
                &modified_sql,
                took_ms(start_time),
                exec_ctx.is_admin(),
            )
        },
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_batch_path(
    prepared_statements: &[PreparedApiExecutionStatement],
    app_context: &Arc<AppContext>,
    sql_executor: &Arc<SqlExecutor>,
    exec_ctx: &ExecutionContext,
    impersonation_service: &SqlImpersonationService,
    authorized_username: &str,
    params: Vec<ScalarValue>,
    http_req: &HttpRequest,
    req_for_forward: &QueryRequest,
    start_time: Instant,
) -> HttpResponse {
    let is_batch = prepared_statements.len() > 1;
    let stmt_count = prepared_statements.len();
    let route_statements_individually = should_route_batch_statements_individually(
        prepared_statements,
        &req_for_forward.params,
        app_context.as_ref(),
        exec_ctx.user_id(),
    );
    let mut results = Vec::with_capacity(stmt_count);
    let mut total_inserted = 0usize;
    let mut total_updated = 0usize;
    let mut total_deleted = 0usize;
    let mut meta_changed_in_batch = false;
    let mut params_remaining = Some(params);
    let request_transaction_coordinator =
        AppContextRequestTransactionCoordinator::new(app_context.as_ref());
    let mut request_transaction_guard = RequestTransactionBatchGuard::from_request_id(
        exec_ctx.request_id(),
        &request_transaction_coordinator,
    );
    let mut statement_exec_ctx = exec_ctx.clone();

    let mut idx = 0;
    while idx < stmt_count {
        let stmt = &prepared_statements[idx];

        // ── Transaction batch INSERT path ───────────────────────────────
        // When an explicit transaction is active and we see consecutive INSERT
        // statements targeting the same table (no EXECUTE AS USER, no params),
        // collect them and process through the transaction batch insert path.
        if let Some(transaction_id) = request_transaction_guard.active_transaction_id() {
            if is_batchable_insert(stmt) {
                let batch_table_id = stmt.prepared_statement.table_id.as_ref();
                let mut batch_end = idx + 1;
                while batch_end < stmt_count
                    && is_batchable_insert(&prepared_statements[batch_end])
                    && prepared_statements[batch_end].prepared_statement.table_id.as_ref()
                        == batch_table_id
                {
                    batch_end += 1;
                }
                let batch_len = batch_end - idx;

                if batch_len > 1 {
                    let batch_stmts: Vec<&PreparedExecutionStatement> = prepared_statements
                        [idx..batch_end]
                        .iter()
                        .map(|s| &s.prepared_statement)
                        .collect();
                    let batch_start = Instant::now();

                    match sql_executor.try_batch_insert_in_transaction(
                        &batch_stmts,
                        exec_ctx,
                        transaction_id,
                    ) {
                        Ok(Some(results)) => {
                            let batch_rows: usize = results.iter().map(|r| r.affected_rows()).sum();
                            let batch_ms = batch_start.elapsed().as_secs_f64() * 1000.0;
                            log::debug!(
                                target: "sql::exec",
                                "✅ Batch INSERT ({} stmts, {} rows) | took={:.3}ms",
                                batch_len,
                                batch_rows,
                                batch_ms,
                            );
                            total_inserted += batch_rows;
                            idx = batch_end;
                            request_transaction_guard.sync(&request_transaction_coordinator);
                            continue;
                        },
                        Ok(None) => { /* fast path not applicable, fall through */ },
                        Err(err) => {
                            let _ = request_transaction_guard
                                .rollback_if_active(&request_transaction_coordinator);
                            return build_statement_error_response(
                                &err,
                                idx + 1,
                                &prepared_statements[idx].prepared_statement.sql,
                                took_ms(start_time),
                                exec_ctx.is_admin(),
                            );
                        },
                    }
                }
            }
        }

        // ── Per-statement execution (original path) ────────────────────
        let execute_as_user =
            match resolve_execute_as_user(stmt, impersonation_service, &statement_exec_ctx).await {
                Ok(uid) => uid,
                Err(err) => {
                    let _ = request_transaction_guard
                        .rollback_if_active(&request_transaction_coordinator);
                    return build_kalamdb_error_response(
                        &err,
                        took_ms(start_time),
                        statement_exec_ctx.is_admin(),
                    );
                },
            };

        if execute_as_user.is_some()
            && stmt.prepared_statement.table_type == Some(TableType::Shared)
        {
            if let Some(table_id) = stmt.prepared_statement.table_id.as_ref() {
                let _ =
                    request_transaction_guard.rollback_if_active(&request_transaction_coordinator);
                return HttpResponse::BadRequest().json(SqlResponse::error_for_privilege(
                    ErrorCode::SqlExecutionError,
                    &format!(
                        "EXECUTE AS USER is not allowed on SHARED tables (table '{}'). AS USER \
                         impersonation is only supported for USER and STREAM tables.",
                        table_id
                    ),
                    took_ms(start_time),
                    statement_exec_ctx.is_admin(),
                ));
            }
        }

        let routing_user_id =
            execute_as_user.as_ref().unwrap_or_else(|| statement_exec_ctx.user_id());

        if route_statements_individually {
            if let Some(target_group) =
                prepared_statement_target_group(stmt, app_context.as_ref(), routing_user_id)
            {
                if !app_context.executor().is_leader(target_group).await {
                    match forward_batch_statement_to_group(
                        target_group,
                        stmt,
                        http_req,
                        Some(statement_exec_ctx.default_namespace()),
                        app_context.as_ref(),
                        statement_exec_ctx.request_id(),
                        start_time,
                        meta_changed_in_batch,
                    )
                    .await
                    {
                        Ok(forwarded_response) => {
                            for result in forwarded_response.results {
                                push_or_accumulate_batch_result(
                                    result,
                                    is_batch,
                                    &mut total_inserted,
                                    &mut total_updated,
                                    &mut total_deleted,
                                    &mut results,
                                );
                            }

                            if statement_mutates_meta(stmt, app_context.as_ref(), routing_user_id) {
                                meta_changed_in_batch = true;
                            }

                            if let Some(classified) =
                                stmt.prepared_statement.classified_statement.as_ref()
                            {
                                if let SqlStatementKind::UseNamespace(use_namespace) =
                                    classified.kind()
                                {
                                    statement_exec_ctx = statement_exec_ctx
                                        .clone()
                                        .with_namespace_id(use_namespace.namespace.clone());
                                }
                            }

                            request_transaction_guard.sync(&request_transaction_coordinator);
                            idx += 1;
                            continue;
                        },
                        Err(response) => {
                            let _ = request_transaction_guard
                                .rollback_if_active(&request_transaction_coordinator);
                            return response;
                        },
                    }
                }
            }
        }

        let stmt_start = Instant::now();
        let effective_username =
            resolve_result_username(authorized_username, stmt.execute_as_username.as_deref());

        let is_last = idx + 1 == stmt_count;

        let stmt_params = if is_last {
            params_remaining.take().unwrap_or_default()
        } else {
            params_remaining.as_ref().cloned().unwrap_or_default()
        };

        match execute_single_statement_raw(
            &stmt.prepared_statement,
            sql_executor,
            &statement_exec_ctx,
            execute_as_user.clone(),
            stmt_params,
        )
        .await
        {
            Ok(exec_result) => {
                let stmt_duration_secs = stmt_start.elapsed().as_secs_f64();
                let stmt_duration_ms = stmt_duration_secs * 1000.0;
                let row_count = exec_result.affected_rows();

                let safe_sql = if log::log_enabled!(log::Level::Debug) {
                    Some(kalamdb_commons::helpers::security::redact_sensitive_sql(
                        &stmt.prepared_statement.sql,
                    ))
                } else {
                    None
                };
                if let Some(safe_sql) = safe_sql.as_ref() {
                    log::debug!(
                        target: "sql::exec",
                        "✅ SQL executed | sql='{}' | user='{}' | role='{:?}' | rows={} | took={:.3}ms",
                        safe_sql,
                        statement_exec_ctx.user_id().as_str(),
                        statement_exec_ctx.user_role(),
                        row_count,
                        stmt_duration_ms
                    );
                }

                app_context.slow_query_logger().log_if_slow(
                    stmt.prepared_statement.track_slow_query,
                    &stmt.prepared_statement.sql,
                    stmt_duration_secs,
                    row_count,
                    statement_exec_ctx.user_id().clone(),
                    stmt.prepared_statement
                        .table_type
                        .unwrap_or(kalamdb_core::schema_registry::TableType::User),
                    stmt.prepared_statement.table_id.as_ref().map(|id| id.table_name().clone()),
                );

                if !is_batch {
                    if let kalamdb_core::sql::ExecutionResult::Rows {
                        batches,
                        row_count,
                        schema,
                    } = exec_result
                    {
                        let effective_role = if execute_as_user.is_some() {
                            Some(kalamdb_commons::Role::User)
                        } else {
                            Some(statement_exec_ctx.user_role())
                        };
                        return match stream_sql_rows_response(
                            batches,
                            schema,
                            effective_role,
                            effective_username,
                            row_count,
                            took_ms(start_time),
                        ) {
                            Ok(response) => response,
                            Err(err) => {
                                let _ = request_transaction_guard
                                    .rollback_if_active(&request_transaction_coordinator);
                                HttpResponse::InternalServerError().json(
                                    SqlResponse::error_for_privilege(
                                        ErrorCode::InternalError,
                                        &format!("Failed to stream SQL response: {}", err),
                                        took_ms(start_time),
                                        statement_exec_ctx.is_admin(),
                                    ),
                                )
                            },
                        };
                    }
                }

                let effective_role = if execute_as_user.is_some() {
                    Some(kalamdb_commons::Role::User)
                } else {
                    Some(statement_exec_ctx.user_role())
                };
                let result = match execution_result_to_query_result(exec_result, effective_role) {
                    Ok(result) => result.with_as_user(effective_username),
                    Err(err) => {
                        let _ = request_transaction_guard
                            .rollback_if_active(&request_transaction_coordinator);
                        return HttpResponse::InternalServerError().json(
                            SqlResponse::error_for_privilege(
                                ErrorCode::InternalError,
                                &format!("Failed to serialize SQL result: {}", err),
                                took_ms(start_time),
                                statement_exec_ctx.is_admin(),
                            ),
                        );
                    },
                };

                if statement_mutates_meta(stmt, app_context.as_ref(), routing_user_id) {
                    meta_changed_in_batch = true;
                }

                push_or_accumulate_batch_result(
                    result,
                    is_batch,
                    &mut total_inserted,
                    &mut total_updated,
                    &mut total_deleted,
                    &mut results,
                );

                if let Some(classified) = stmt.prepared_statement.classified_statement.as_ref() {
                    if let SqlStatementKind::UseNamespace(use_namespace) = classified.kind() {
                        statement_exec_ctx = statement_exec_ctx
                            .clone()
                            .with_namespace_id(use_namespace.namespace.clone());
                    }
                }
            },
            Err(err) => {
                let _ =
                    request_transaction_guard.rollback_if_active(&request_transaction_coordinator);

                if let Some(kalamdb_err) = err.downcast_ref::<kalamdb_core::error::KalamDbError>() {
                    if let Some(response) = handle_not_leader_error(
                        kalamdb_err,
                        http_req,
                        req_for_forward,
                        app_context,
                        statement_exec_ctx.request_id(),
                        start_time,
                    )
                    .await
                    {
                        return response;
                    }
                }

                return build_statement_error_response(
                    err.as_ref(),
                    idx + 1,
                    &stmt.prepared_statement.sql,
                    took_ms(start_time),
                    statement_exec_ctx.is_admin(),
                );
            },
        }

        request_transaction_guard.sync(&request_transaction_coordinator);
        idx += 1;
    }

    if let Err(err) = request_transaction_guard.ensure_closed(&request_transaction_coordinator) {
        let err = map_request_transaction_error(err);
        return build_kalamdb_error_response(&err, took_ms(start_time), exec_ctx.is_admin());
    }

    if is_batch {
        if total_inserted > 0 {
            results.push(
                QueryResult::with_affected_rows(
                    total_inserted,
                    Some(format!("Inserted {} row(s)", total_inserted)),
                )
                .with_as_user(authorized_username.to_string()),
            );
        }
        if total_updated > 0 {
            results.push(
                QueryResult::with_affected_rows(
                    total_updated,
                    Some(format!("Updated {} row(s)", total_updated)),
                )
                .with_as_user(authorized_username.to_string()),
            );
        }
        if total_deleted > 0 {
            results.push(
                QueryResult::with_affected_rows(
                    total_deleted,
                    Some(format!("Deleted {} row(s)", total_deleted)),
                )
                .with_as_user(authorized_username.to_string()),
            );
        }
    }

    HttpResponse::Ok().json(SqlResponse::success(results, took_ms(start_time)))
}

/// Check if a prepared statement is a simple INSERT eligible for batching:
/// no EXECUTE AS USER, has a table_id and table_type, and is classified as INSERT.
fn is_batchable_insert(stmt: &PreparedApiExecutionStatement) -> bool {
    if stmt.execute_as_username.is_some() {
        return false;
    }
    if stmt.prepared_statement.table_id.is_none() || stmt.prepared_statement.table_type.is_none() {
        return false;
    }
    matches!(
        stmt.prepared_statement.classified_statement.as_ref().map(|c| c.kind()),
        Some(SqlStatementKind::Insert(_))
    )
}
