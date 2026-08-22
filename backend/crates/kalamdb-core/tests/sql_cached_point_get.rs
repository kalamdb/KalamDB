mod support;

use std::{collections::HashMap, sync::Arc};

use datafusion_common::ScalarValue;
use kalamdb_commons::{
    models::{
        datatypes::KalamDataType,
        schemas::{ColumnDefinition, TableDefinition, TableOptions},
        KalamCellValue, TableId, TableName, UserId,
    },
    schemas::ColumnDefault,
    Role, TableType,
};
use kalamdb_core::{
    app_context::AppContext,
    sql::{ExecutionContext, ExecutionResult, SqlExecutor},
};
use support::{
    create_cluster_app_context, create_context_files_table, create_executor, create_shared_table,
    create_user_table, execute_ok, execute_ok_with_params, insert_sql, observer_exec_ctx,
    result_rows, unique_namespace,
};

fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn file_ref_json(id: &str, sha256: &str) -> String {
    format!(
        r#"{{"id":"{id}","sub":"f0001","name":"index.md","size":12,"mime":"text/markdown","sha256":"{sha256}"}}"#
    )
}

fn cell_text(row: &HashMap<String, KalamCellValue>, column: &str) -> String {
    let value = row
        .get(column)
        .unwrap_or_else(|| panic!("missing column '{column}' in {row:?}"));
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    if let Some(number) = value.as_i64() {
        return number.to_string();
    }
    value.to_string()
}

fn file_ref_sha256(row: &HashMap<String, KalamCellValue>) -> String {
    let value = row.get("file_ref").unwrap_or_else(|| panic!("missing file_ref in {row:?}"));
    if let Some(text) = value.as_str() {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text) {
            if let Some(sha) = parsed.get("sha256").and_then(|sha| sha.as_str()) {
                return sha.to_string();
            }
        }
        return text.to_string();
    }
    if let Some(sha) = value.get("sha256").and_then(|sha| sha.as_str()) {
        return sha.to_string();
    }
    panic!("file_ref was not a FileRef JSON value: {value}");
}

fn assert_single_column(row: &HashMap<String, KalamCellValue>, column: &str) {
    assert!(
        row.contains_key(column),
        "expected column '{column}', got {:?}",
        row.keys().collect::<Vec<_>>()
    );
    assert!(
        row.keys().filter(|name| *name != column).all(|name| name.starts_with('_')),
        "SELECT {column} must not expose other user columns, got {:?}",
        row.keys().collect::<Vec<_>>()
    );
}

async fn create_stream_table(app_ctx: &Arc<AppContext>, table_name: &str) -> TableId {
    let table_id =
        TableId::new(unique_namespace("sql_point_get_stream"), TableName::new(table_name));
    let id_col = ColumnDefinition::new(
        1,
        "id".to_string(),
        1,
        KalamDataType::BigInt,
        false,
        true,
        false,
        ColumnDefault::None,
        None,
    );
    let name_col = ColumnDefinition::simple(2, "name", 2, KalamDataType::Text);
    let mut table_def = TableDefinition::new(
        table_id.namespace_id().clone(),
        table_id.table_name().clone(),
        TableType::Stream,
        vec![id_col, name_col],
        TableOptions::stream(120),
        None,
    )
    .expect("create stream table definition");
    app_ctx
        .system_columns_service()
        .add_system_columns(&mut table_def)
        .expect("add system columns");
    app_ctx
        .schema_registry()
        .register_table(table_def)
        .expect("register stream table");
    table_id
}

fn assert_name_point_get(label: &str, result: ExecutionResult, expected_name: &str) {
    let rows = result_rows(result);
    assert_eq!(rows.len(), 1, "{label} should return one row");
    assert_single_column(&rows[0], "name");
    assert_eq!(
        cell_text(&rows[0], "name"),
        expected_name,
        "{label} must return the selected name, not the primary key"
    );
}

async fn repeat_name_lookup(
    executor: &SqlExecutor,
    exec_ctx: &ExecutionContext,
    sql: &str,
    params: Vec<ScalarValue>,
    expected_name: &str,
    repeats: usize,
) {
    for i in 0..repeats {
        let result = execute_ok_with_params(executor, exec_ctx, sql, params.clone()).await;
        assert_name_point_get(&format!("lookup {i}"), result, expected_name);
    }
}

#[tokio::test]
#[ntest::timeout(20000)]
async fn cached_point_get_keeps_file_ref_select_list_after_update() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let table_id = create_context_files_table(
        &app_ctx,
        &unique_namespace("sql_point_get_files"),
        "context_files",
    )
    .await;
    let executor = create_executor(Arc::clone(&app_ctx));
    let exec_ctx = ExecutionContext::new(
        UserId::from("sql-point-get-file-user"),
        Role::User,
        app_ctx.base_session_context(),
    );

    let first_sha = "7505522b895796b845b734eab8baef4a60b6fe224cc2615978ee26a768bdb392";
    let second_sha = "8505522b895796b845b734eab8baef4a60b6fe224cc2615978ee26a768bdb393";
    let first_ref = file_ref_json("328909262921138176", first_sha);
    let second_ref = file_ref_json("328909262921138177", second_sha);
    let qualified = format!("{}.{}", table_id.namespace_id(), table_id.table_name());

    execute_ok(
        &executor,
        &exec_ctx,
        &format!(
            "INSERT INTO {qualified} (path, file_ref) VALUES ('index.md', {})",
            sql_string(&first_ref)
        ),
    )
    .await;

    let select_file_ref = format!("SELECT file_ref FROM {qualified} WHERE path = $1");
    let path_param = vec![ScalarValue::Utf8(Some("index.md".to_string()))];

    for i in 0..4 {
        let rows = result_rows(
            execute_ok_with_params(&executor, &exec_ctx, &select_file_ref, path_param.clone())
                .await,
        );
        assert_eq!(rows.len(), 1, "file_ref lookup {i} should return one row");
        assert_single_column(&rows[0], "file_ref");
        assert_eq!(
            file_ref_sha256(&rows[0]),
            first_sha,
            "file_ref lookup {i} must keep the FILE column, not return path"
        );
    }

    execute_ok(
        &executor,
        &exec_ctx,
        &format!(
            "UPDATE {qualified} SET file_ref = {} WHERE path = 'index.md'",
            sql_string(&second_ref)
        ),
    )
    .await;

    let updated = result_rows(
        execute_ok_with_params(&executor, &exec_ctx, &select_file_ref, path_param.clone()).await,
    );
    assert_eq!(file_ref_sha256(&updated[0]), second_sha);

    let missing = result_rows(
        execute_ok_with_params(
            &executor,
            &exec_ctx,
            &select_file_ref,
            vec![ScalarValue::Utf8(Some("missing.md".to_string()))],
        )
        .await,
    );
    assert!(missing.is_empty(), "missing PK must not invent a file_ref row");

    let star = result_rows(
        execute_ok_with_params(
            &executor,
            &exec_ctx,
            &format!("SELECT * FROM {qualified} WHERE path = $1"),
            path_param.clone(),
        )
        .await,
    );
    assert_eq!(cell_text(&star[0], "path"), "index.md");
    assert_eq!(file_ref_sha256(&star[0]), second_sha);

    let reordered = result_rows(
        execute_ok_with_params(
            &executor,
            &exec_ctx,
            &format!("SELECT file_ref, path FROM {qualified} WHERE path = $1"),
            path_param.clone(),
        )
        .await,
    );
    assert_eq!(file_ref_sha256(&reordered[0]), second_sha);
    assert_eq!(cell_text(&reordered[0], "path"), "index.md");

    let pk_only = result_rows(
        execute_ok_with_params(
            &executor,
            &exec_ctx,
            &format!("SELECT path FROM {qualified} WHERE path = $1"),
            path_param.clone(),
        )
        .await,
    );
    assert_eq!(cell_text(&pk_only[0], "path"), "index.md");
    assert!(!pk_only[0].contains_key("file_ref"));

    let literal = result_rows(
        execute_ok(
            &executor,
            &exec_ctx,
            &format!("SELECT file_ref FROM {qualified} WHERE path = 'index.md'"),
        )
        .await,
    );
    assert_eq!(file_ref_sha256(&literal[0]), second_sha);
    let literal_cached = result_rows(
        execute_ok(
            &executor,
            &exec_ctx,
            &format!("SELECT file_ref FROM {qualified} WHERE path = 'index.md'"),
        )
        .await,
    );
    assert_eq!(file_ref_sha256(&literal_cached[0]), second_sha);

    let limited = format!("SELECT file_ref FROM {qualified} WHERE path = $1 LIMIT 1");
    for i in 0..3 {
        let rows = result_rows(
            execute_ok_with_params(&executor, &exec_ctx, &limited, path_param.clone()).await,
        );
        assert_eq!(rows.len(), 1, "LIMIT 1 lookup {i}");
        assert_single_column(&rows[0], "file_ref");
        assert_eq!(file_ref_sha256(&rows[0]), second_sha);
    }
}

#[tokio::test]
#[ntest::timeout(20000)]
async fn cached_point_get_projects_non_pk_columns_across_table_types() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let executor = create_executor(Arc::clone(&app_ctx));
    let exec_ctx = observer_exec_ctx(&app_ctx);

    let user_table =
        create_user_table(&app_ctx, &unique_namespace("sql_point_get_user"), "blogs").await;
    let shared_table =
        create_shared_table(&app_ctx, &unique_namespace("sql_point_get_shared"), "blogs").await;
    let stream_table = create_stream_table(&app_ctx, "events").await;

    execute_ok(&executor, &exec_ctx, &insert_sql(&user_table, 1, "alpha")).await;
    execute_ok(&executor, &exec_ctx, &insert_sql(&user_table, 2, "beta")).await;
    execute_ok(&executor, &exec_ctx, &insert_sql(&shared_table, 1, "shared-alpha")).await;
    execute_ok(&executor, &exec_ctx, &insert_sql(&stream_table, 1, "stream-alpha")).await;

    let user_sql = format!(
        "SELECT name FROM {}.{} WHERE id = $1",
        user_table.namespace_id(),
        user_table.table_name()
    );
    repeat_name_lookup(
        &executor,
        &exec_ctx,
        &user_sql,
        vec![ScalarValue::Int64(Some(1))],
        "alpha",
        4,
    )
    .await;
    repeat_name_lookup(
        &executor,
        &exec_ctx,
        &user_sql,
        vec![ScalarValue::Int64(Some(2))],
        "beta",
        2,
    )
    .await;

    let aliased = result_rows(
        execute_ok_with_params(
            &executor,
            &exec_ctx,
            &format!(
                "SELECT name AS title FROM {}.{} WHERE id = $1",
                user_table.namespace_id(),
                user_table.table_name()
            ),
            vec![ScalarValue::Int64(Some(1))],
        )
        .await,
    );
    let aliased_cached = result_rows(
        execute_ok_with_params(
            &executor,
            &exec_ctx,
            &format!(
                "SELECT name AS title FROM {}.{} WHERE id = $1",
                user_table.namespace_id(),
                user_table.table_name()
            ),
            vec![ScalarValue::Int64(Some(1))],
        )
        .await,
    );
    for (label, rows) in [("first", &aliased), ("cached", &aliased_cached)] {
        assert_eq!(rows.len(), 1, "{label} alias lookup");
        assert_eq!(
            cell_text(&rows[0], "title"),
            "alpha",
            "{label} aliased SELECT must keep the output name"
        );
        assert!(!rows[0].contains_key("id"), "{label} alias lookup must not return the PK");
    }

    repeat_name_lookup(
        &executor,
        &exec_ctx,
        &format!(
            "SELECT name FROM {}.{} WHERE id = $1",
            shared_table.namespace_id(),
            shared_table.table_name()
        ),
        vec![ScalarValue::Int64(Some(1))],
        "shared-alpha",
        3,
    )
    .await;
    repeat_name_lookup(
        &executor,
        &exec_ctx,
        &format!(
            "SELECT name FROM {}.{} WHERE id = $1",
            stream_table.namespace_id(),
            stream_table.table_name()
        ),
        vec![ScalarValue::Int64(Some(1))],
        "stream-alpha",
        3,
    )
    .await;
}
