use std::time::Duration;

use anyhow::Result;
use kalam_client::models::ResponseStatus;
use tokio::time::{sleep, Instant};

use super::test_support::{
    auth_helper::create_user_auth_header_default,
    consolidated_helpers::{unique_namespace, unique_table},
    flush::{
        flush_table_and_wait, wait_for_parquet_files_for_table,
        wait_for_parquet_files_for_user_table,
    },
    jobs::wait_for_path_absent,
};

fn find_table_state_files(root: &std::path::Path, namespace: &str) -> Vec<std::path::PathBuf> {
    fn recurse(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>, namespace: &str) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                recurse(&path, out, namespace);
                continue;
            }

            let path_str = path.to_string_lossy();
            let is_state_file = path_str.contains(namespace)
                && (path_str.ends_with(".parquet") || path_str.ends_with("manifest.json"));
            if is_state_file {
                out.push(path);
            }
        }
    }

    let mut out = Vec::new();
    recurse(root, &mut out, namespace);
    out
}

#[tokio::test]
#[ntest::timeout(90000)]
async fn test_drop_namespace_cascade_removes_all_table_state_over_http() -> Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;

    let ns = unique_namespace("ns_drop_cleanup");
    let user_table = "user_events";
    let shared_table = "shared_events";

    let user = unique_table("ns_drop_user");
    let user_auth = create_user_auth_header_default(server, &user).await?;

    let resp = server.execute_sql(&format!("CREATE NAMESPACE {}", ns)).await?;
    anyhow::ensure!(resp.status == ResponseStatus::Success, "CREATE NAMESPACE failed");

    let resp = server
        .execute_sql(&format!(
            "CREATE TABLE {}.{} (id BIGINT PRIMARY KEY, payload TEXT) WITH (TYPE='USER', \
             STORAGE_ID='local', FLUSH_POLICY='rows:2')",
            ns, user_table
        ))
        .await?;
    anyhow::ensure!(resp.status == ResponseStatus::Success, "CREATE USER table failed");

    let resp = server
        .execute_sql(&format!(
            "CREATE TABLE {}.{} (id BIGINT PRIMARY KEY, payload TEXT) WITH (TYPE='SHARED', \
             STORAGE_ID='local', FLUSH_POLICY='rows:2')",
            ns, shared_table
        ))
        .await?;
    anyhow::ensure!(resp.status == ResponseStatus::Success, "CREATE SHARED table failed");

    for id in 1..=3 {
        let user_insert = server
            .execute_sql_with_auth(
                &format!(
                    "INSERT INTO {}.{} (id, payload) VALUES ({}, 'u_{}')",
                    ns, user_table, id, id
                ),
                &user_auth,
            )
            .await?;
        anyhow::ensure!(user_insert.status == ResponseStatus::Success, "USER insert failed");

        let shared_insert = server
            .execute_sql(&format!(
                "INSERT INTO {}.{} (id, payload) VALUES ({}, 's_{}')",
                ns, shared_table, id, id
            ))
            .await?;
        anyhow::ensure!(shared_insert.status == ResponseStatus::Success, "SHARED insert failed");
    }

    flush_table_and_wait(server, &ns, user_table).await?;
    flush_table_and_wait(server, &ns, shared_table).await?;

    let _ = wait_for_parquet_files_for_user_table(
        server,
        &ns,
        user_table,
        &user,
        1,
        Duration::from_secs(20),
    )
    .await?;
    let _ = wait_for_parquet_files_for_table(server, &ns, shared_table, 1, Duration::from_secs(20))
        .await?;

    let namespace_storage_dir = server.storage_root().join(&ns);

    let drop_resp = server.execute_sql(&format!("DROP NAMESPACE {} CASCADE", ns)).await?;
    anyhow::ensure!(drop_resp.status == ResponseStatus::Success, "DROP NAMESPACE failed");

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let ns_count = server
            .execute_sql(&format!(
                "SELECT COUNT(*) AS cnt FROM system.namespaces WHERE namespace_id = '{}'",
                ns
            ))
            .await?;
        let schema_count = server
            .execute_sql(&format!(
                "SELECT COUNT(*) AS cnt FROM system.schemas WHERE namespace_id = '{}'",
                ns
            ))
            .await?;

        let ns_gone =
            ns_count.status == ResponseStatus::Success && ns_count.get_i64("cnt") == Some(0);
        let schemas_gone = schema_count.status == ResponseStatus::Success
            && schema_count.get_i64("cnt") == Some(0);

        let table_query = server
            .execute_sql(&format!("SELECT COUNT(*) AS cnt FROM {}.{}", ns, shared_table))
            .await?;
        let table_unqueryable = table_query.status == ResponseStatus::Error;

        if ns_gone && schemas_gone && table_unqueryable {
            break;
        }

        if Instant::now() >= deadline {
            anyhow::bail!(
                "namespace state not fully removed in time (ns_gone={}, schemas_gone={}, \
                 table_unqueryable={})",
                ns_gone,
                schemas_gone,
                table_unqueryable
            );
        }

        sleep(Duration::from_millis(100)).await;
    }

    // Namespace directories may persist briefly as empty placeholders; table state files must not.
    let _ = wait_for_path_absent(&namespace_storage_dir, Duration::from_secs(2)).await;
    let leftover_state = find_table_state_files(&server.storage_root(), &ns);
    anyhow::ensure!(
        leftover_state.is_empty(),
        "namespace table-state files still exist after DROP NAMESPACE: {:?}",
        leftover_state
    );

    Ok(())
}
