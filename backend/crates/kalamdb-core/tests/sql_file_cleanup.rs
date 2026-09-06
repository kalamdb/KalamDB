mod support;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use kalamdb_commons::{
    models::{
        datatypes::KalamDataType,
        schemas::{ColumnDefinition, TableDefinition, TableOptions},
        NamespaceId, TableId, TableName, UserId,
    },
    schemas::ColumnDefault,
    TableAccess, TableType,
};
use kalamdb_core::{app_context::AppContext, sql::context::ExecutionContext};
use kalamdb_store::test_utils::TestDb;
use kalamdb_system::FileRef;
use support::{create_cluster_app_context, create_executor, execute_ok, unique_namespace};

async fn create_user_file_table(
    app_ctx: &Arc<AppContext>,
    namespace: &NamespaceId,
    table_name: &str,
) -> TableId {
    let table_id = TableId::new(namespace.clone(), TableName::new(table_name));
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
    let file_col = ColumnDefinition::simple(2, "file_ref", 2, KalamDataType::File);

    let mut table_def = TableDefinition::new(
        namespace.clone(),
        table_id.table_name().clone(),
        TableType::User,
        vec![id_col, file_col],
        TableOptions::user(),
        None,
    )
    .expect("create user file table definition");
    app_ctx
        .system_columns_service()
        .add_system_columns(&mut table_def)
        .expect("add system columns");

    app_ctx
        .schema_registry()
        .register_table(table_def)
        .expect("register user file table");
    table_id
}

async fn create_shared_file_table(
    app_ctx: &Arc<AppContext>,
    namespace: &NamespaceId,
    table_name: &str,
) -> TableId {
    let table_id = TableId::new(namespace.clone(), TableName::new(table_name));
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
    let file_col = ColumnDefinition::simple(2, "file_ref", 2, KalamDataType::File);

    let mut table_options = TableOptions::shared();
    if let TableOptions::Shared(options) = &mut table_options {
        options.access_level = Some(TableAccess::Public);
    }

    let mut table_def = TableDefinition::new(
        namespace.clone(),
        table_id.table_name().clone(),
        TableType::Shared,
        vec![id_col, file_col],
        table_options,
        None,
    )
    .expect("create shared file table definition");
    app_ctx
        .system_columns_service()
        .add_system_columns(&mut table_def)
        .expect("add system columns");

    app_ctx
        .schema_registry()
        .register_table(table_def)
        .expect("register shared file table");
    table_id
}

fn file_ref(id: &str, name: &str) -> FileRef {
    FileRef::new(
        id.to_string(),
        "f0001".to_string(),
        name.to_string(),
        4,
        "text/plain".to_string(),
        format!("sha-{id}"),
    )
}

fn file_path(
    test_db: &TestDb,
    table_type: TableType,
    table_id: &TableId,
    user_id: Option<&UserId>,
    file_ref: &FileRef,
) -> PathBuf {
    let mut path = PathBuf::from(test_db.storage_dir().expect("storage dir"));
    match table_type {
        TableType::User => {
            path.push("user");
            path.push(table_id.namespace_id().to_string());
            path.push("{table}");
            path.push(user_id.expect("user table needs user id").as_str());
        },
        TableType::Shared => {
            path.push("shared");
            path.push(table_id.namespace_id().to_string());
            path.push("{table}");
        },
        TableType::Stream | TableType::System => unreachable!("file cleanup test table type"),
    }
    path.push(file_ref.relative_path());
    path
}

fn write_blob(path: &Path) {
    fs::create_dir_all(path.parent().expect("blob parent")).expect("create blob parent");
    fs::write(path, b"data").expect("write blob");
}

fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn sql_update_deletes_replaced_user_file_after_commit() {
    let (app_ctx, test_db) = create_cluster_app_context().await;
    let table_id =
        create_user_file_table(&app_ctx, &unique_namespace("sql_file_update"), "files").await;
    let executor = create_executor(app_ctx.clone());
    let user_id = UserId::from("sql-file-update-user");
    let exec_ctx = ExecutionContext::new(
        user_id.clone(),
        kalamdb_commons::Role::User,
        app_ctx.base_session_context(),
    );

    let old_ref = file_ref("old-update", "old.txt");
    let new_ref = file_ref("new-update", "new.txt");
    let old_path = file_path(&test_db, TableType::User, &table_id, Some(&user_id), &old_ref);
    let new_path = file_path(&test_db, TableType::User, &table_id, Some(&user_id), &new_ref);
    write_blob(&old_path);
    write_blob(&new_path);

    let insert = format!(
        "INSERT INTO {}.{} (id, file_ref) VALUES (1, {})",
        table_id.namespace_id(),
        table_id.table_name(),
        sql_string(&old_ref.to_json())
    );
    execute_ok(&executor, &exec_ctx, &insert).await;

    let update = format!(
        "UPDATE {}.{} SET file_ref = {} WHERE id = 1",
        table_id.namespace_id(),
        table_id.table_name(),
        sql_string(&new_ref.to_json())
    );
    execute_ok(&executor, &exec_ctx, &update).await;

    assert!(!old_path.exists(), "old file should be deleted after committed update");
    assert!(new_path.exists(), "new file should remain after committed update");
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn sql_delete_deletes_user_file_after_commit() {
    let (app_ctx, test_db) = create_cluster_app_context().await;
    let table_id =
        create_user_file_table(&app_ctx, &unique_namespace("sql_file_delete"), "files").await;
    let executor = create_executor(app_ctx.clone());
    let user_id = UserId::from("sql-file-delete-user");
    let exec_ctx = ExecutionContext::new(
        user_id.clone(),
        kalamdb_commons::Role::User,
        app_ctx.base_session_context(),
    );

    let old_ref = file_ref("old-delete", "old.txt");
    let old_path = file_path(&test_db, TableType::User, &table_id, Some(&user_id), &old_ref);
    write_blob(&old_path);

    let insert = format!(
        "INSERT INTO {}.{} (id, file_ref) VALUES (1, {})",
        table_id.namespace_id(),
        table_id.table_name(),
        sql_string(&old_ref.to_json())
    );
    execute_ok(&executor, &exec_ctx, &insert).await;

    let delete =
        format!("DELETE FROM {}.{} WHERE id = 1", table_id.namespace_id(), table_id.table_name());
    execute_ok(&executor, &exec_ctx, &delete).await;

    assert!(!old_path.exists(), "old file should be deleted after committed delete");
}

#[tokio::test]
#[ntest::timeout(8000)]
async fn sql_on_conflict_update_deletes_replaced_shared_file_after_commit() {
    let (app_ctx, test_db) = create_cluster_app_context().await;
    let table_id =
        create_shared_file_table(&app_ctx, &unique_namespace("sql_file_upsert"), "files").await;
    let executor = create_executor(app_ctx.clone());
    let user_id = UserId::from("sql-file-upsert-user");
    let exec_ctx = ExecutionContext::new(
        user_id.clone(),
        kalamdb_commons::Role::Dba,
        app_ctx.base_session_context(),
    );

    let old_ref = file_ref("old-upsert", "old.txt");
    let new_ref = file_ref("new-upsert", "new.txt");
    let old_path = file_path(&test_db, TableType::Shared, &table_id, None, &old_ref);
    let new_path = file_path(&test_db, TableType::Shared, &table_id, None, &new_ref);
    write_blob(&old_path);
    write_blob(&new_path);

    let insert = format!(
        "INSERT INTO {}.{} (id, file_ref) VALUES (1, {})",
        table_id.namespace_id(),
        table_id.table_name(),
        sql_string(&old_ref.to_json())
    );
    execute_ok(&executor, &exec_ctx, &insert).await;

    let upsert = format!(
        "INSERT INTO {}.{} (id, file_ref) VALUES (1, {}) ON CONFLICT (id) DO UPDATE SET file_ref \
         = EXCLUDED.file_ref",
        table_id.namespace_id(),
        table_id.table_name(),
        sql_string(&new_ref.to_json())
    );
    execute_ok(&executor, &exec_ctx, &upsert).await;

    assert!(!old_path.exists(), "old file should be deleted after committed upsert");
    assert!(new_path.exists(), "new file should remain after committed upsert");
}
