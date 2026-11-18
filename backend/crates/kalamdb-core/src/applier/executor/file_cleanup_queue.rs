//! Durable file-cleanup intents.
//!
//! A commit records each replaced file in the same storage batch as the row
//! write. Object deletion runs after that batch returns. A restart leaves the
//! intent in place, and the next commit drains it. Deleting an intent happens
//! only after every listed object delete succeeds.

use std::sync::Arc;

use kalamdb_commons::{
    models::UserId,
    schemas::TableType,
    storage::{Operation, Partition},
    TableId,
};
use kalamdb_store::storage_trait::{Result as StorageResult, StorageBackend, StorageError};
use kalamdb_system::FileRef;
use serde::{Deserialize, Serialize};

use super::prepared_commit::FileCleanupIntent;
use crate::app_context::AppContext;

const PARTITION_NAME: &str = "sys_file_cleanup";

#[derive(Serialize, Deserialize)]
struct DurableFileCleanup {
    table_id: TableId,
    table_type: TableType,
    user_id: Option<String>,
    file_refs: Vec<FileRef>,
}

fn cleanup_partition() -> Partition {
    Partition::new(PARTITION_NAME)
}

fn intent_key(intent: &FileCleanupIntent) -> Vec<u8> {
    let mut key = format!(
        "{}:{}:{}",
        intent.table_id,
        intent.table_type,
        intent.user_id.as_ref().map(|user| user.as_str()).unwrap_or("")
    );
    for file_ref in &intent.file_refs {
        key.push(':');
        key.push_str(&file_ref.id);
    }
    key.into_bytes()
}

/// Puts that record `intents` in the commit batch. Empty input does no I/O.
pub(crate) fn cleanup_puts(
    backend: &dyn StorageBackend,
    intents: &[FileCleanupIntent],
) -> StorageResult<Vec<Operation>> {
    if intents.is_empty() {
        return Ok(Vec::new());
    }
    let partition = cleanup_partition();
    backend.create_partition(&partition)?;
    let mut operations = Vec::with_capacity(intents.len());
    for intent in intents {
        let record = DurableFileCleanup {
            table_id: intent.table_id.clone(),
            table_type: intent.table_type,
            user_id: intent.user_id.as_ref().map(|user| user.as_str().to_string()),
            file_refs: intent.file_refs.clone(),
        };
        let value = serde_json::to_vec(&record).map_err(|error| {
            StorageError::SerializationError(format!("file cleanup intent: {error}"))
        })?;
        operations.push(Operation::Put {
            partition: partition.clone(),
            key: intent_key(intent),
            value,
        });
    }
    Ok(operations)
}

/// Record intents that belong to a row write which has already committed.
///
/// Object deletion runs after the intent batch. A failed intent write deletes
/// the files before returning, so a cleanup is not dropped on the floor.
pub(crate) async fn queue_file_cleanup(app: Arc<AppContext>, intents: Vec<FileCleanupIntent>) {
    let intents: Vec<_> =
        intents.into_iter().filter(|intent| !intent.file_refs.is_empty()).collect();
    if intents.is_empty() {
        return;
    }
    let queued = match cleanup_puts(app.storage_backend().as_ref(), &intents) {
        Ok(operations) => app.storage_backend().batch(operations).is_ok(),
        Err(error) => {
            log::warn!("file cleanup intent was not recorded: {error}");
            false
        },
    };
    if queued {
        let immediate = Arc::clone(&app);
        let stored = kalamdb_store::defer_after_persist_async(async move {
            drain_file_cleanup(app).await;
        });
        if !stored {
            drain_file_cleanup(immediate).await;
        }
        return;
    }
    for intent in &intents {
        super::utils::fileref_util::delete_file_refs_best_effort(
            app.as_ref(),
            &intent.table_id,
            intent.table_type,
            intent.user_id.as_ref(),
            &intent.file_refs,
        )
        .await;
    }
}

/// Delete every recorded object, then drop the intent when that delete succeeds.
pub(crate) async fn drain_file_cleanup(app: Arc<AppContext>) {
    let backend = app.storage_backend();
    let partition = cleanup_partition();
    if !backend.partition_exists(&partition) {
        return;
    }
    let Ok(scan) = backend.scan(&partition, None, None, None) else {
        return;
    };
    let Ok(items) = scan.collect::<std::result::Result<Vec<_>, _>>() else {
        return;
    };
    for (key, value) in items {
        let Ok(record) = serde_json::from_slice::<DurableFileCleanup>(&value) else {
            let _ = backend.delete(&partition, &key);
            continue;
        };
        if delete_recorded_files(&app, &record).await {
            let _ = backend.delete(&partition, &key);
        }
    }
}

async fn delete_recorded_files(app: &AppContext, record: &DurableFileCleanup) -> bool {
    if record.file_refs.is_empty() {
        return true;
    }
    let Ok(storage_id) = app.schema_registry().get_storage_id(&record.table_id) else {
        return false;
    };
    let user = record.user_id.as_ref().map(|user| UserId::new(user.clone()));
    let results = app
        .file_storage_service()
        .delete_files(
            &record.file_refs,
            &storage_id,
            record.table_type,
            &record.table_id,
            user.as_ref(),
        )
        .await;
    let succeeded = results.iter().all(Result::is_ok);
    if !succeeded {
        log::warn!(
            "file cleanup for {} left {} object deletes pending",
            record.table_id,
            results.iter().filter(|result| result.is_err()).count()
        );
    }
    succeeded
}
