use std::{fs, path::Path, path::PathBuf, time::Duration};

use anyhow::Result;
use kalam_client::models::ResponseStatus;
use kalamdb_commons::TableId;

use super::test_support::{
    consolidated_helpers::unique_namespace,
    flush::{flush_table_and_wait, wait_for_parquet_files_for_table},
};

fn find_manifest_files(root: &Path) -> Vec<PathBuf> {
    fn recurse(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                recurse(&path, out);
            } else if path.file_name().and_then(|n| n.to_str()) == Some("manifest.json") {
                out.push(path);
            }
        }
    }

    let mut out = Vec::new();
    recurse(root, &mut out);
    out
}

fn find_batch_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("batch-") && name.ends_with(".parquet"))
                .unwrap_or(false)
        })
        .collect()
}

#[tokio::test]
#[ntest::timeout(90000)]
async fn test_manifest_missing_or_corrupt_is_handled_without_server_crash_over_http() -> Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;

    // ---------------------------------------------------------------------
    // Case 1: Missing manifest should fall back to directory scan.
    // ---------------------------------------------------------------------
    {
        let namespace = unique_namespace("manifest_missing_fallback");
        let table = "events";

        let resp = server.execute_sql(&format!("CREATE NAMESPACE {}", namespace)).await?;
        anyhow::ensure!(resp.status == ResponseStatus::Success);

        let resp = server
            .execute_sql(&format!(
                "CREATE TABLE {}.{} (id BIGINT PRIMARY KEY, value TEXT) WITH (TYPE='SHARED', \
                 FLUSH_POLICY='rows:2')",
                namespace, table
            ))
            .await?;
        anyhow::ensure!(resp.status == ResponseStatus::Success);

        for id in 1..=6 {
            let insert = server
                .execute_sql(&format!(
                    "INSERT INTO {}.{} (id, value) VALUES ({}, 'v_{}')",
                    namespace, table, id, id
                ))
                .await?;
            anyhow::ensure!(insert.status == ResponseStatus::Success);
        }

        flush_table_and_wait(server, &namespace, table).await?;
        let _ = wait_for_parquet_files_for_table(
            server,
            &namespace,
            table,
            1,
            Duration::from_secs(20),
        )
        .await?;

        let storage_root = server.storage_root();
        let manifest_path = find_manifest_files(&storage_root)
            .into_iter()
            .find(|p| p.to_string_lossy().contains(&namespace) && p.to_string_lossy().contains(table))
            .ok_or_else(|| anyhow::anyhow!("manifest.json not found for {}.{}", namespace, table))?;

        let table_id = TableId::from_strings(&namespace, table);
        let _ = server.app_context().manifest_service().invalidate_table(&table_id)?;

        fs::remove_file(&manifest_path)?;

        let count_resp = server
            .execute_sql(&format!("SELECT COUNT(*) AS cnt FROM {}.{}", namespace, table))
            .await?;
        anyhow::ensure!(
            count_resp.status == ResponseStatus::Success,
            "query should succeed via manifest-missing fallback"
        );
        anyhow::ensure!(
            count_resp.get_i64("cnt") == Some(6),
            "unexpected row count after manifest-missing fallback"
        );
    }

    // ---------------------------------------------------------------------
    // Case 2: Missing parquet and corrupt manifest should return errors
    // gracefully, and server should stay healthy.
    // ---------------------------------------------------------------------
    {
        let namespace = unique_namespace("manifest_faults");
        let table = "orders";

        let resp = server.execute_sql(&format!("CREATE NAMESPACE {}", namespace)).await?;
        anyhow::ensure!(resp.status == ResponseStatus::Success);

        let resp = server
            .execute_sql(&format!(
                "CREATE TABLE {}.{} (id BIGINT PRIMARY KEY, value TEXT) WITH (TYPE='SHARED', \
                 FLUSH_POLICY='rows:2')",
                namespace, table
            ))
            .await?;
        anyhow::ensure!(resp.status == ResponseStatus::Success);

        for id in 1..=4 {
            let insert = server
                .execute_sql(&format!(
                    "INSERT INTO {}.{} (id, value) VALUES ({}, 'o_{}')",
                    namespace, table, id, id
                ))
                .await?;
            anyhow::ensure!(insert.status == ResponseStatus::Success);
        }

        flush_table_and_wait(server, &namespace, table).await?;
        let _ = wait_for_parquet_files_for_table(
            server,
            &namespace,
            table,
            1,
            Duration::from_secs(20),
        )
        .await?;

        let storage_root = server.storage_root();
        let manifest_path = find_manifest_files(&storage_root)
            .into_iter()
            .find(|p| p.to_string_lossy().contains(&namespace) && p.to_string_lossy().contains(table))
            .ok_or_else(|| anyhow::anyhow!("manifest.json not found for {}.{}", namespace, table))?;
        let table_dir = manifest_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("manifest path missing parent dir"))?;

        let mut batch_files = find_batch_files(table_dir);
        anyhow::ensure!(!batch_files.is_empty(), "expected at least one parquet batch file");

        let table_id = TableId::from_strings(&namespace, table);
        let _ = server.app_context().manifest_service().invalidate_table(&table_id)?;

        let first_batch = batch_files.pop().expect("batch file list unexpectedly empty");
        fs::remove_file(&first_batch)?;

        let missing_file_resp = server
            .execute_sql(&format!("SELECT COUNT(*) AS cnt FROM {}.{}", namespace, table))
            .await?;
        match missing_file_resp.status {
            ResponseStatus::Error => {
                // Accept explicit error responses for missing segment reads.
            },
            ResponseStatus::Success => {
                // Newer read paths may skip missing parquet files instead of failing.
                // In that case, row count should be lower than the original full dataset.
                let count = missing_file_resp.get_i64("cnt").unwrap_or_default();
                anyhow::ensure!(
                    count < 4,
                    "missing parquet file fallback should not return full row count"
                );
            },
        }

        fs::write(&manifest_path, "{ this is not valid json")?;
        let _ = server.app_context().manifest_service().invalidate_table(&table_id)?;

        let corrupt_manifest_resp = server
            .execute_sql(&format!("SELECT COUNT(*) AS cnt FROM {}.{}", namespace, table))
            .await?;
        anyhow::ensure!(
            corrupt_manifest_resp.status == ResponseStatus::Error,
            "corrupt manifest should return an error response"
        );

        // Server should remain healthy after both fault paths.
        let health = server.execute_sql("SELECT 1 AS ok").await?;
        anyhow::ensure!(health.status == ResponseStatus::Success);
    }

    Ok(())
}